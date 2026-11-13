//! Scoped, nonblocking Git requests. Mutation failures never consume the draft.
use crate::{
    guest_task_board::Scope,
    share::{GuestRpcError, guest_text},
};
use serde_json::{Value, json};
use std::{
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};

pub type Reply = Result<Value, GuestRpcError>;
#[derive(Clone, Copy, Debug)]
pub enum Action {
    Status,
    Commit,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    Busy,
    Unavailable,
    Invalid,
    Unknown,
    Saved,
    WrongProject,
    TooLarge,
    Host(String),
}
impl Notice {
    pub fn text(&self, fr: bool) -> String {
        match (self, fr) {
            (Self::Busy, true) => "Une opération Git est en cours ou la file est pleine.",
            (Self::Busy, false) => "A Git operation is pending or the queue is full.",
            (Self::Unavailable, true) => "Git est indisponible. Actualisez pour réessayer.",
            (Self::Unavailable, false) => "Git is unavailable. Refresh to retry.",
            (Self::Invalid, true) => "Réponse Git invalide.",
            (Self::Invalid, false) => "Invalid Git response.",
            (Self::Unknown, true) => "Résultat du commit inconnu. Message conservé; vérifiez l’historique avant de réessayer.",
            (Self::Unknown, false) => "Commit outcome unknown. Message kept; check history before retrying.",
            (Self::Saved, true) => "Commit confirmé par l’hôte.",
            (Self::Saved, false) => "Commit confirmed by the host.",
            (Self::WrongProject, true) => "Ce message appartient à un autre projet ou à une session terminée. Revenez au projet, copiez-le ou abandonnez-le.",
            (Self::WrongProject, false) => "This message belongs to another project or an ended session. Return to its project, copy it or discard it.",
            (Self::TooLarge, true) => "Le message du commit doit contenir de 1 à 2 000 octets.",
            (Self::TooLarge, false) => "The commit message must contain 1 to 2,000 bytes.",
            (Self::Host(text), _) => return text.clone(),
        }.into()
    }
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub scope: Option<Scope>,
    pub writable: bool,
    pub draft_scope: Option<Scope>,
    pub draft: String,
    pub status: Option<String>,
    pub hidden: u64,
    pub limited: bool,
    pub pending: bool,
    pub uncertain: bool,
    pub notice: Option<Notice>,
}
impl Snapshot {
    pub fn editable(&self) -> bool {
        self.scope.is_some()
            && self.writable
            && self.draft_scope.is_none_or(|s| Some(s) == self.scope)
    }
}
struct Pending {
    action: Action,
    revision: u64,
    receiver: Receiver<Reply>,
    deadline: Instant,
}
#[derive(Default)]
pub struct Controller {
    view: Snapshot,
    pending: Option<Pending>,
    revision: u64,
    oversized: bool,
}
impl Controller {
    pub fn snapshot(&self) -> Snapshot {
        self.view.clone()
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn bind(&mut self, scope: Option<Scope>, writable: bool) -> bool {
        let changed = self.view.scope != scope || self.view.writable != writable;
        if self.view.scope != scope {
            let uncertain = self
                .pending
                .take()
                .is_some_and(|p| matches!(p.action, Action::Commit));
            self.view.uncertain |= uncertain;
            self.view.pending = false;
            if self.view.draft.is_empty() && !self.view.uncertain {
                self.view.draft_scope = None;
            }
            self.view.status = None;
            self.view.hidden = 0;
            self.view.limited = false;
            self.view.notice = if uncertain {
                Some(Notice::Unknown)
            } else if !self.view.draft.is_empty() && self.view.draft_scope != scope {
                Some(Notice::WrongProject)
            } else {
                None
            };
            self.view.scope = scope;
        }
        self.view.writable = writable && scope.is_some();
        changed
    }
    pub fn edit(&mut self, scope: Scope, text: String) -> bool {
        if self.view.scope != Some(scope) || !self.view.editable() {
            return false;
        }
        if text.len() > 64 * 1024 {
            self.oversized = true;
            self.view.notice = Some(Notice::TooLarge);
            return false;
        }
        self.oversized = false;
        if self.view.draft == text {
            return false;
        }
        self.revision = self.revision.wrapping_add(1);
        // Keep the origin even when cleared while a commit is in flight.
        self.view.draft_scope = Some(scope);
        self.view.draft = text;
        if self.view.draft.trim().len() > 2000 {
            self.view.notice = Some(Notice::TooLarge);
        } else if matches!(self.view.notice, Some(Notice::Saved | Notice::TooLarge)) {
            self.view.notice = None;
        }
        true
    }
    pub fn discard(&mut self) -> bool {
        if self.busy() {
            return false;
        }
        self.view.draft.clear();
        self.view.draft_scope = None;
        self.view.notice = None;
        self.oversized = false;
        self.view.uncertain = false;
        self.revision = self.revision.wrapping_add(1);
        true
    }
    pub fn start(
        &mut self,
        scope: Scope,
        action: Action,
        now: Instant,
        send: impl FnOnce(Value) -> Result<Receiver<Reply>, String>,
    ) -> bool {
        if self.view.scope != Some(scope) {
            return false;
        }
        if self.busy() {
            self.view.notice = Some(Notice::Busy);
            return false;
        }
        let payload = match action {
            Action::Status => json!({"op":"scm.status","projectId":scope.project}),
            Action::Commit => {
                if !self.view.editable() {
                    self.view.notice = Some(Notice::WrongProject);
                    return false;
                }
                let message = self.view.draft.trim();
                if self.oversized
                    || message.is_empty()
                    || message.len() > 2000
                    || message.contains('\0')
                {
                    self.view.notice = Some(Notice::TooLarge);
                    return false;
                }
                json!({"op":"scm.commit","projectId":scope.project,"message":message})
            }
        };
        match send(payload) {
            Ok(receiver) => {
                self.pending = Some(Pending {
                    action,
                    revision: self.revision,
                    receiver,
                    deadline: now + Duration::from_secs(25),
                });
                self.view.pending = true;
                self.view.notice = None;
            }
            Err(_) => {
                self.view.notice = Some(Notice::Busy);
                return false;
            }
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
        self.view.pending = false;
        self.view.notice = match (pending.action, result) {
            (Action::Status, Some(Ok(value))) => {
                if let Some(status) = value
                    .get("status")
                    .and_then(Value::as_str)
                    .filter(|s| s.len() <= 256 * 1024)
                {
                    let hidden = value.get("hidden").map_or(Some(0), Value::as_u64);
                    let limited = value.get("truncated").map_or(Some(false), Value::as_bool);
                    let (Some(hidden), Some(limited)) = (hidden, limited) else {
                        self.view.notice = Some(Notice::Invalid);
                        return true;
                    };
                    if hidden > 1_000_000 {
                        self.view.notice = Some(Notice::Invalid);
                        return true;
                    }
                    self.view.hidden = hidden;
                    self.view.limited = limited;
                    self.view.status = Some(status.into());
                    None
                } else {
                    Some(Notice::Invalid)
                }
            }
            (Action::Status, Some(Err(GuestRpcError::Host(error)))) => {
                Some(Notice::Host(guest_text::task_title(&error, "", 200)))
            }
            (Action::Status, _) => Some(Notice::Unavailable),
            (Action::Commit, Some(Ok(value))) if value.get("ok") == Some(&Value::Bool(true)) => {
                if pending.revision == self.revision {
                    self.view.draft.clear();
                    self.view.draft_scope = None;
                    self.oversized = false;
                }
                self.view.status = None;
                self.view.uncertain = false;
                Some(Notice::Saved)
            }
            // A timeout or process error may occur after Git changed the repository.
            // Preserve the message and never retry a mutation automatically.
            (Action::Commit, Some(Err(GuestRpcError::Host(error)))) => {
                self.view.uncertain = true;
                Some(Notice::Host(guest_text::task_title(&error, "", 200)))
            }
            (Action::Commit, _) => {
                self.view.uncertain = true;
                Some(Notice::Unknown)
            }
        };
        true
    }
}
