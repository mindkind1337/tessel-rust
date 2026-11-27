//! A file draft belongs to one guest session, project and path, never to the last global download.
use crate::{
    guest_task_board::Scope,
    share::{GuestRpcError, guest_text},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};
pub(crate) fn decode_text(value: &Value, limit: usize) -> Option<String> {
    value
        .get("data")
        .and_then(Value::as_str)
        .filter(|s| s.len() <= limit.div_ceil(3) * 4)
        .and_then(|s| B64.decode(s).ok())
        .filter(|bytes| bytes.len() <= limit)
        .and_then(|bytes| String::from_utf8(bytes).ok())
}
const MAX_READ: usize = 8 * 1024 * 1024;
// Fits the existing 256 KiB request envelope after base64 and a bounded path.
pub const MAX_EDIT: usize = 180 * 1024;
type Reply = Result<Value, GuestRpcError>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub scope: Scope,
    pub path: String,
}
#[derive(Clone, Debug)]
pub struct Document {
    pub key: Key,
    pub text: Arc<String>,
    baseline: Arc<String>,
    read_only: bool,
}
impl Document {
    /// Bound layout work without modifying the underlying file or copied text.
    pub fn preview(&self) -> (&str, bool) {
        let mut end = self.text.len().min(64 * 1024);
        while !self.text.is_char_boundary(end) {
            end -= 1;
        }
        (&self.text[..end], end < self.text.len())
    }

    pub fn dirty(&self) -> bool {
        self.text != self.baseline
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    Busy,
    Unavailable,
    Invalid,
    Unknown,
    UnsavedDraft,
    ReadOnly,
    TooLarge,
    Saved,
    Conflict,
    Host(String),
}
impl Notice {
    pub fn text(&self, fr: bool) -> String {
        match (self,fr) {
            (Self::Busy,true)=>"Une opération est en cours ou la file est pleine.", (Self::Busy,false)=>"An operation is pending or the queue is full.",
            (Self::Unavailable,true)=>"Le fichier est indisponible.", (Self::Unavailable,false)=>"The file is unavailable.",
            (Self::Invalid,true)=>"Réponse invalide ou fichier non textuel UTF-8.", (Self::Invalid,false)=>"Invalid response or non-UTF-8 text file.",
            (Self::Unknown,true)=>"Résultat inconnu. Le brouillon est conservé; vérifiez le fichier avant de réessayer.", (Self::Unknown,false)=>"Outcome unknown. Your draft is kept; verify the file before retrying.",
            (Self::UnsavedDraft,true)=>"Un brouillon non enregistré est conservé. Revenez à son projet pour l’enregistrer, copiez-le ou abandonnez-le avant d’ouvrir un autre fichier.", (Self::UnsavedDraft,false)=>"An unsaved draft is kept. Return to its project to save it, copy it, or discard it before opening another file.",
            (Self::ReadOnly,true)=>"Ce fichier ou cette action n’est plus accessible.", (Self::ReadOnly,false)=>"This file or action is no longer available.",
            (Self::TooLarge,true)=>"L’édition invitée est limitée à 180 Kio par enregistrement.", (Self::TooLarge,false)=>"Guest editing is limited to 180 KiB per save.",
            (Self::Conflict,true)=>"Le fichier a changé chez l’hôte. Votre brouillon est conservé. Copiez-le avant de recharger le fichier.", (Self::Conflict,false)=>"The host file changed. Your draft is kept. Copy it before reloading the file.",
            (Self::Saved,true)=>"Fichier enregistré.", (Self::Saved,false)=>"File saved.",
            (Self::Host(text),_)=>return text.clone(),
        }.into()
    }
}
enum Operation {
    Read(Key),
    Save(Key, Arc<String>),
}
struct Pending {
    operation: Operation,
    deadline: Instant,
    receiver: Receiver<Reply>,
}
#[derive(Default)]
pub struct Editor {
    scope: Option<Scope>,
    writable: bool,
    pub(crate) tabs: Vec<Key>,
    pub document: Option<Document>,
    pending: Option<Pending>,
    pub notice: Option<Notice>,
    oversized_edit: bool,
    uncertain_write: bool,
}
impl Editor {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn has_draft(&self) -> bool {
        self.document.as_ref().is_some_and(|d| {
            d.dirty()
                || self.uncertain_write
                || self
                    .pending
                    .as_ref()
                    .is_some_and(|p| matches!(p.operation, Operation::Save(..)))
        })
    }
    pub fn editable(&self) -> bool {
        self.writable
            && self.document.as_ref().is_some_and(|d| {
                Some(d.key.scope) == self.scope && d.baseline.len() <= MAX_EDIT && !d.read_only
            })
    }
    pub fn bind(&mut self, scope: Option<Scope>, writable: bool) -> bool {
        let changed = self.scope != scope || self.writable != writable;
        if self.scope != scope {
            self.tabs.clear();
            let uncertain = self
                .pending
                .as_ref()
                .is_some_and(|p| matches!(p.operation, Operation::Save(..)));
            self.uncertain_write |= uncertain;
            self.pending = None;
            if self.has_draft() {
                self.notice = Some(if self.uncertain_write {
                    Notice::Unknown
                } else {
                    Notice::UnsavedDraft
                });
            } else {
                self.document = None;
                self.oversized_edit = false;
                self.notice = None;
            }
            if let Some(doc) = &self.document
                && Some(doc.key.scope) == scope
            {
                self.tabs.push(doc.key.clone());
            }
            self.scope = scope;
        }
        self.writable = scope.is_some() && writable;
        changed
    }
    pub fn edit(&mut self, key: &Key, text: String) -> bool {
        if !self.editable() {
            return false;
        }
        let Some(document) = self.document.as_mut().filter(|d| &d.key == key) else {
            return false;
        };
        if text.len() > MAX_READ {
            self.oversized_edit = true;
            self.notice = Some(Notice::TooLarge);
            return false;
        }
        self.oversized_edit = false;
        if text.len() > MAX_EDIT {
            self.notice = Some(Notice::TooLarge);
        } else if self.notice == Some(Notice::TooLarge) {
            self.notice = None;
        }
        if document.text.as_str() == text {
            return false;
        }
        document.text = Arc::new(text);
        if self.notice == Some(Notice::Saved) {
            self.notice = None;
        }
        true
    }
    pub fn discard(&mut self) -> bool {
        if self.pending.is_some() {
            return false;
        }
        if let Some(doc) = &self.document {
            self.tabs.retain(|key| key != &doc.key);
        }
        self.clear_active_document();
        true
    }

    pub(crate) fn clear_active_document(&mut self) {
        self.document = None;
        self.uncertain_write = false;
        self.oversized_edit = false;
        self.notice = None;
    }
    pub fn open(
        &mut self,
        key: Key,
        now: Instant,
        send: impl FnOnce(Value) -> Result<Receiver<Reply>, String>,
    ) -> bool {
        if Some(key.scope) != self.scope || self.pending.is_some() {
            return false;
        }
        if self.has_draft() {
            self.notice = Some(if self.uncertain_write {
                Notice::Unknown
            } else {
                Notice::UnsavedDraft
            });
            return true;
        }
        if !valid_path(&key.path) {
            self.notice = Some(Notice::Invalid);
            return true;
        }
        let payload = json!({"op":"files.read","projectId":key.scope.project,"path":key.path});
        match send(payload) {
            Ok(receiver) => {
                self.document = None;
                self.oversized_edit = false;
                self.notice = None;
                self.pending = Some(Pending {
                    operation: Operation::Read(key),
                    deadline: now + Duration::from_secs(20),
                    receiver,
                });
            }
            Err(_) => self.notice = Some(Notice::Busy),
        }
        true
    }
    pub fn save(
        &mut self,
        key: &Key,
        now: Instant,
        send: impl FnOnce(Value) -> Result<Receiver<Reply>, String>,
    ) -> bool {
        if self.pending.is_some() {
            return false;
        }
        if !self.editable() || !valid_path(&key.path) {
            self.notice = Some(Notice::ReadOnly);
            return true;
        }
        let Some(document) = self
            .document
            .as_ref()
            .filter(|d| &d.key == key && (d.dirty() || self.uncertain_write))
        else {
            return false;
        };
        if self.oversized_edit || document.text.len() > MAX_EDIT {
            self.notice = Some(Notice::TooLarge);
            return true;
        }
        let text = document.text.clone();
        let payload = json!({"op":"files.write","projectId":key.scope.project,"path":key.path,"data":B64.encode(text.as_bytes()),"expectHash":format!("{:x}", Sha256::digest(document.baseline.as_bytes()))});
        match send(payload) {
            Ok(receiver) => {
                self.notice = None;
                self.pending = Some(Pending {
                    operation: Operation::Save(key.clone(), text),
                    deadline: now + Duration::from_secs(20),
                    receiver,
                });
            }
            Err(_) => self.notice = Some(Notice::Busy),
        }
        true
    }
    pub fn poll(&mut self, now: Instant) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) if now < pending.deadline => return false,
            Err(_) => None,
        };
        let pending = self.pending.take().unwrap();
        let saving = matches!(pending.operation, Operation::Save(..));
        let value = match result {
            Some(Ok(value)) => value,
            Some(Err(GuestRpcError::Host(error))) => {
                self.notice = Some(if saving && error.starts_with("File conflict:") {
                    Notice::Conflict
                } else {
                    Notice::Host(guest_text::task_title(&error, "", 200))
                });
                return true;
            }
            Some(Err(GuestRpcError::Transport(_))) | None => {
                self.uncertain_write |= saving;
                self.notice = Some(if saving {
                    Notice::Unknown
                } else {
                    Notice::Unavailable
                });
                return true;
            }
        };
        match pending.operation {
            Operation::Read(key) => {
                let text = decode_text(&value, MAX_READ);
                if let Some(text) = text {
                    if !self.tabs.contains(&key) {
                        self.tabs.push(key.clone());
                    }
                    let text = Arc::new(text);
                    self.notice = (text.len() > MAX_EDIT).then_some(Notice::TooLarge);
                    self.document = Some(Document {
                        key,
                        text: text.clone(),
                        baseline: text,
                        read_only: value
                            .get("readOnly")
                            .is_some_and(|v| v != &Value::Bool(false)),
                    });
                } else {
                    self.notice = Some(Notice::Invalid);
                }
            }
            Operation::Save(key, text) => {
                if value.get("ok").and_then(Value::as_bool) == Some(true) {
                    if let Some(document) = self.document.as_mut().filter(|d| d.key == key) {
                        document.baseline = text;
                    }
                    self.uncertain_write = false;
                    self.notice = Some(Notice::Saved);
                } else {
                    self.uncertain_write = true;
                    self.notice = Some(Notice::Unknown);
                }
            }
        }
        true
    }
}
pub(crate) fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.chars().any(|c| {
            c.is_control()
                || c == ':'
                || c == '\\'
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
        })
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}
