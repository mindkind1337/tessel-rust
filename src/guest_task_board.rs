//! Renderer-independent, nonblocking guest task state. Host replies are untrusted.
use crate::share::{GuestRpcError, guest_text};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};

pub type Reply = Result<Value, GuestRpcError>;
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Scope {
    pub session: u64,
    pub project: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub id: u64,
    pub title: String,
    pub column: usize,
    pub guest_by: Option<String>,
}
#[derive(Clone, Debug)]
pub enum Action {
    Refresh,
    Add,
}
impl Action {
    fn mutation(&self) -> bool {
        !matches!(self, Self::Refresh)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    Busy,
    Unavailable,
    InvalidReply,
    OutcomeUnknown,
    Reconciled,
    Host(String),
    NotAllowed,
    InvalidInput,
}
impl Notice {
    pub fn text(&self, fr: bool) -> String {
        match self {
            Self::Busy => if fr { "Une requête est déjà en cours ou la file est pleine." } else { "A request is pending or the queue is full." },
            Self::Unavailable => if fr { "Le tableau est indisponible. Actualisez pour réessayer." } else { "The board is unavailable. Refresh to retry." },
            Self::InvalidReply => if fr { "L’hôte a renvoyé un tableau invalide." } else { "The host returned an invalid task board." },
            Self::OutcomeUnknown => if fr { "Résultat inconnu. Le brouillon est conservé; vérifiez le tableau avant de réessayer." } else { "Outcome unknown. Your draft is kept; check the board before retrying." },
            Self::Reconciled => if fr { "Tableau actualisé. Vérifiez les cartes avant de renvoyer le brouillon conservé." } else { "Board refreshed. Check the cards before resubmitting your retained draft." },
            Self::NotAllowed => if fr { "Ce projet ou cette action n’est plus accessible." } else { "This project or action is no longer available." },
            Self::InvalidInput => if fr { "Le titre doit contenir de 1 à 300 caractères." } else { "The title must contain 1 to 300 characters." },
            Self::Host(text) => return text.clone(),
        }.into()
    }
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub scope: Option<Scope>,
    pub writable: bool,
    pub pending: bool,
    pub cards: Vec<Card>,
    pub limited: bool,
    pub draft: String,
    pub draft_over_limit: bool,
    pub notice: Option<Notice>,
}
struct Pending {
    scope: Scope,
    action: Action,
    revision: u64,
    deadline: Instant,
    reply: Receiver<Reply>,
}
#[derive(Default)]
pub struct Controller {
    view: Snapshot,
    revision: u64,
    pending: Option<Pending>,
    reconcile: bool,
    // Retain a bounded set of drafts while switching authorized projects in one session.
    drafts: Vec<(Scope, String, u64)>,
    drafts_session: Option<u64>,
}
impl Controller {
    pub fn input(&self) -> Option<(Scope, &str)> {
        self.view
            .writable
            .then(|| (self.view.scope.unwrap(), self.view.draft.as_str()))
    }
    pub fn snapshot(&self) -> Snapshot {
        self.view.clone()
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn bind(&mut self, scope: Option<Scope>, writable: bool) -> bool {
        let changed = self.view.scope != scope || self.view.writable != writable;
        if self.view.scope != scope {
            let ended_draft = scope.is_none().then(|| self.view.draft.clone());
            let outcome_unknown =
                scope.is_none() && self.pending.as_ref().is_some_and(|p| p.action.mutation());
            let different_session = scope.is_some_and(|s| self.drafts_session != Some(s.session));
            if different_session {
                self.drafts.clear();
                self.drafts_session = scope.map(|s| s.session);
            } else if let Some(old) = self.view.scope {
                self.drafts.retain(|(s, _, _)| *s != old);
                if !self.view.draft.is_empty() {
                    self.drafts
                        .push((old, self.view.draft.clone(), self.revision));
                }
                if self.drafts.len() > 32 {
                    self.drafts.remove(0);
                }
            }
            let saved = scope
                .and_then(|s| self.drafts.iter().position(|(key, _, _)| *key == s))
                .map(|index| self.drafts.remove(index));
            self.view = Snapshot {
                scope,
                ..Snapshot::default()
            };
            self.revision = 0;
            if let Some((_, draft, revision)) = saved {
                self.view.draft = draft;
                self.revision = revision;
            }
            if let Some(draft) = ended_draft {
                self.view.draft = draft;
            }
            if outcome_unknown {
                self.view.notice = Some(Notice::OutcomeUnknown);
            }
            self.pending = None;
            self.reconcile = scope.is_some();
        }
        self.view.writable = scope.is_some() && writable;
        changed
    }
    pub fn touch_draft(&mut self, scope: Scope) -> bool {
        if self.view.scope != Some(scope) || !self.view.writable {
            return false;
        }
        self.view.draft_over_limit = true;
        self.revision = self.revision.wrapping_add(1);
        if self.revision == 0 {
            self.pending = None;
            self.view.pending = false;
        }
        true
    }
    pub fn edit_draft(&mut self, scope: Scope, text: String) -> bool {
        if self.view.scope != Some(scope) || !self.view.writable || text.len() > 4096 {
            return false;
        }
        if self.view.draft == text && !self.view.draft_over_limit {
            return false;
        }
        self.view.draft_over_limit = false;
        if self.view.notice == Some(Notice::InvalidInput) {
            self.view.notice = None;
        }
        self.view.draft = text;
        self.revision = self.revision.wrapping_add(1);
        if self.revision == 0 {
            self.pending = None;
            self.view.pending = false;
        }
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
        if self.pending.is_some() {
            return false;
        }
        if matches!(action, Action::Add) && self.view.draft_over_limit {
            self.view.notice = Some(Notice::InvalidInput);
            return true;
        }
        if action.mutation() && !self.view.writable {
            self.view.notice = Some(Notice::NotAllowed);
            return true;
        }
        let Some(payload) = self.payload(scope, &action) else {
            self.view.notice = Some(Notice::InvalidInput);
            return true;
        };
        match send(payload) {
            Ok(reply) => {
                if matches!(action, Action::Refresh) {
                    self.reconcile = false;
                }
                self.pending = Some(Pending {
                    scope,
                    action,
                    revision: self.revision,
                    deadline: now + REQUEST_TIMEOUT,
                    reply,
                });
                self.view.pending = true;
                if self.view.notice != Some(Notice::OutcomeUnknown) {
                    self.view.notice = None;
                }
            }
            Err(_) => self.view.notice = Some(Notice::Busy),
        }
        true
    }
    fn payload(&self, scope: Scope, action: &Action) -> Option<Value> {
        let mut body = match action {
            Action::Refresh => json!({"op":"tasks.list"}),
            Action::Add => {
                json!({"op":"tasks.mutate", "action":"add", "title":guest_text::clean_task_input(&self.view.draft)?, "column":"todo"})
            }
        };
        body["projectId"] = scope.project.into();
        Some(body)
    }
    pub fn poll(&mut self, now: Instant) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        let result = match pending.reply.try_recv() {
            Ok(reply) => Some(reply),
            Err(TryRecvError::Disconnected) => None,
            Err(TryRecvError::Empty) if now >= pending.deadline => None,
            Err(TryRecvError::Empty) => return false,
        };
        let pending = self.pending.take().unwrap();
        self.view.pending = false;
        if self.view.scope != Some(pending.scope) {
            return true;
        }
        let Some(result) = result else {
            self.view.notice = Some(if pending.action.mutation() {
                Notice::OutcomeUnknown
            } else {
                Notice::Unavailable
            });
            self.reconcile = pending.action.mutation();
            return true;
        };
        match result {
            Ok(value) => match checked_board(&value) {
                Some((cards, limited)) => {
                    self.view.cards = cards;
                    self.view.limited = limited;
                    if matches!(pending.action, Action::Add) && pending.revision == self.revision {
                        self.view.draft.clear();
                    }
                    self.view.notice = if matches!(pending.action, Action::Refresh)
                        && self.view.notice == Some(Notice::OutcomeUnknown)
                    {
                        Some(Notice::Reconciled)
                    } else {
                        None
                    };
                }
                None => {
                    self.view.notice = Some(if pending.action.mutation() {
                        Notice::OutcomeUnknown
                    } else {
                        Notice::InvalidReply
                    });
                    self.reconcile = pending.action.mutation();
                }
            },
            Err(GuestRpcError::Transport(error)) => {
                if pending.action.mutation() {
                    self.view.notice = Some(Notice::OutcomeUnknown);
                    self.reconcile = true;
                } else {
                    let message = guest_text::task_title(&error, "", 200);
                    self.view.notice = Some(if message.is_empty() {
                        Notice::Unavailable
                    } else {
                        Notice::Host(message)
                    });
                }
            }
            Err(GuestRpcError::Host(error))
                if pending.action.mutation()
                    && error == crate::share::TASK_RESULT_NOT_AVAILABLE =>
            {
                self.view.notice = Some(Notice::OutcomeUnknown);
                self.reconcile = true;
            }
            Err(GuestRpcError::Host(error)) => {
                let message = guest_text::task_title(&error, "", 200);
                self.view.notice = Some(if message.is_empty() {
                    Notice::Unavailable
                } else {
                    Notice::Host(message)
                });
            }
        }
        true
    }
    pub fn take_reconcile(&mut self) -> bool {
        self.pending.is_none() && std::mem::take(&mut self.reconcile)
    }
}
const COLUMNS: [&str; 4] = ["todo", "doing", "review", "done"];
fn checked_board(value: &Value) -> Option<(Vec<Card>, bool)> {
    let rows = value.get("tasks")?.as_array()?;
    if rows.len() > 500 {
        return None;
    }
    let mut limited = value.get("limited")?.as_bool()?;
    let mut ids = HashSet::new();
    let mut cards = Vec::with_capacity(rows.len());
    for row in rows {
        let raw = row.get("id")?.as_str()?;
        let id = raw.parse::<u64>().ok()?;
        if id.to_string() != raw || !ids.insert(id) {
            return None;
        }
        let title = row.get("title")?.as_str()?;
        if title.len() > 1200 || title.chars().count() > 300 {
            return None;
        }
        let title = if title.is_empty() {
            String::new()
        } else {
            let Some(clean) = guest_text::clean_task_input(title) else {
                limited = true;
                continue;
            };
            clean
        };
        let guest_by = match row.get("guestBy") {
            None | Some(Value::Null) => None,
            Some(Value::String(name)) if name.len() <= 160 && name.chars().count() <= 40 => {
                guest_text::clean_task_input(name)
            }
            _ => return None,
        };
        let column = COLUMNS
            .iter()
            .position(|c| Some(*c) == row.get("column").and_then(Value::as_str))?;
        cards.push(Card {
            id,
            title,
            column,
            guest_by,
        });
    }
    Some((cards, limited))
}
