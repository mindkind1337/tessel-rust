//! Scoped directory replies. A late listing never repopulates another guest project.
use crate::{
    guest_file_editor::valid_path,
    guest_task_board::Scope,
    share::{GuestRpcError, guest_text},
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub directory: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    Busy,
    Invalid,
    Unavailable,
    Partial,
    Host(String),
}
impl Notice {
    pub fn text(&self, fr: bool) -> String {
        match (self, fr) {
            (Self::Busy, true) => "Une lecture est en cours ou la file est pleine.",
            (Self::Busy, false) => "A read is pending or the queue is full.",
            (Self::Invalid, true) => "L’hôte a renvoyé une liste de fichiers invalide.",
            (Self::Invalid, false) => "The host returned an invalid file list.",
            (Self::Unavailable, true) => "Le dossier est indisponible. Actualisez pour réessayer.",
            (Self::Unavailable, false) => "The folder is unavailable. Refresh to retry.",
            (Self::Partial, true) => "Affichage partiel du dossier.",
            (Self::Partial, false) => "Partial folder listing.",
            (Self::Host(text), _) => return text.clone(),
        }
        .into()
    }
}
struct Pending {
    path: String,
    receiver: Receiver<Result<Value, GuestRpcError>>,
    deadline: Instant,
}
#[derive(Default)]
pub struct Listing {
    scope: Option<Scope>,
    pub path: String,
    pub entries: Vec<Entry>,
    pending: Option<Pending>,
    initial: bool,
    pub notice: Option<Notice>,
}
impl Listing {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn bind(&mut self, scope: Option<Scope>) -> bool {
        if self.scope == scope {
            return false;
        }
        self.scope = scope;
        self.entries.clear();
        self.path.clear();
        self.pending = None;
        self.notice = None;
        self.initial = scope.is_some();
        true
    }
    pub fn take_initial(&mut self) -> bool {
        self.pending.is_none() && std::mem::take(&mut self.initial)
    }
    pub fn start(
        &mut self,
        scope: Scope,
        path: String,
        now: Instant,
        send: impl FnOnce(Value) -> Result<Receiver<Result<Value, GuestRpcError>>, String>,
    ) -> bool {
        if self.scope != Some(scope) || self.pending.is_some() {
            return false;
        }
        if !path.is_empty() && !valid_path(&path) {
            self.notice = Some(Notice::Invalid);
            return true;
        }
        let payload = json!({"op":"files.list","projectId":scope.project,"path":path});
        match send(payload) {
            Ok(receiver) => {
                if path != self.path {
                    self.entries.clear();
                }
                self.path = path.clone();
                self.notice = None;
                self.initial = false;
                self.pending = Some(Pending {
                    path,
                    receiver,
                    deadline: now + Duration::from_secs(20),
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
            Ok(v) => Some(v),
            Err(TryRecvError::Empty) if now < pending.deadline => return false,
            Err(_) => None,
        };
        let pending = self.pending.take().unwrap();
        match result {
            Some(Ok(value)) => match checked_entries(&value, &pending.path) {
                Some((entries, partial)) => {
                    self.entries = entries;
                    self.notice = partial.then_some(Notice::Partial);
                }
                None => self.notice = Some(Notice::Invalid),
            },
            Some(Err(GuestRpcError::Host(error))) => {
                self.notice = Some(Notice::Host(guest_text::task_title(&error, "", 200)))
            }
            _ => self.notice = Some(Notice::Unavailable),
        }
        true
    }
}
fn checked_entries(value: &Value, parent: &str) -> Option<(Vec<Entry>, bool)> {
    let rows = value.get("entries")?.as_array()?;
    if rows.len() > 10_000 {
        return None;
    }
    let mut entries = Vec::with_capacity(rows.len().min(2048));
    let mut seen = HashSet::new();
    let mut used = 0usize;
    let mut partial = false;
    for row in rows {
        let name = row.get("name")?.as_str()?;
        let path = row.get("path")?.as_str()?;
        let directory = row.get("directory")?.as_bool()?;
        if path.rsplit('/').next() != Some(name)
            || path.rsplit_once('/').map_or("", |(p, _)| p) != parent
            || !seen.insert(path)
        {
            return None;
        }
        if !valid_path(name) || name.contains('/') || !valid_path(path) {
            partial = true;
            continue;
        }
        used = used.saturating_add(name.len() + path.len());
        if used > 2 * 1024 * 1024 {
            partial = true;
            continue;
        }
        entries.push(Entry {
            name: name.into(),
            path: path.into(),
            directory,
        });
    }
    Some((entries, partial))
}
