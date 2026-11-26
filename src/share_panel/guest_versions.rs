use crate::{
    guest_file_editor::Key,
    share::{GuestRpcError, GuestSession},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Version {
    pub id: String,
    timestamp: i64,
}
impl Version {
    pub fn label(&self) -> String {
        chrono::DateTime::from_timestamp_millis(self.timestamp)
            .map(|dt| {
                dt.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_else(|| self.id.clone())
    }
}
struct Pending {
    reply: Receiver<Result<Value, GuestRpcError>>,
    deadline: Instant,
    id: Option<String>,
}
#[derive(Default)]
pub(super) struct Versions {
    key: Option<Key>,
    pending: Option<Pending>,
    pub rows: Option<Arc<Vec<Version>>>,
    pub text: Option<Arc<String>>,
    pub failed: bool,
}
impl Versions {
    pub fn bind(&mut self, key: Option<&Key>) -> bool {
        if self.key.as_ref() == key {
            return false;
        }
        *self = Self {
            key: key.cloned(),
            ..Default::default()
        };
        true
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn request(&mut self, session: &GuestSession, id: Option<&str>) {
        let Some(key) = self.key.as_ref() else { return };
        if self.busy() || session.generation() != key.scope.session {
            return;
        }
        let mut q = json!({"projectId":key.scope.project,"path":key.path,"op":"history.local"});
        if let Some(id) = id {
            if !self
                .rows
                .as_ref()
                .is_some_and(|rows| rows.iter().any(|r| r.id == id))
            {
                return;
            }
            q["op"] = json!("history.localRead");
            q["id"] = json!(id);
        } else {
            self.rows = None;
        }
        self.text = None;
        self.failed = false;
        match session.request(q) {
            Ok(reply) => {
                self.pending = Some(Pending {
                    reply,
                    deadline: Instant::now() + Duration::from_secs(25),
                    id: id.map(str::to_owned),
                })
            }
            Err(_) => self.failed = true,
        }
    }
    pub fn poll(&mut self, now: Instant) -> bool {
        let Some(p) = self.pending.as_ref() else {
            return false;
        };
        let result = match p.reply.try_recv() {
            Ok(Ok(v)) => Some(v),
            Err(TryRecvError::Empty) if now < p.deadline => return false,
            _ => None,
        };
        let Some(p) = self.pending.take() else {
            return false;
        };
        if p.id.is_some() {
            self.text = result.and_then(|mut v| match v.get_mut("text")?.take() {
                Value::String(text) if text.len() <= 2 * 1024 * 1024 => Some(Arc::new(text)),
                _ => None,
            });
            self.failed = self.text.is_none();
        } else {
            self.rows = result
                .filter(|v| v["rel"].as_str() == self.key.as_ref().map(|k| k.path.as_str()))
                .and_then(|mut v| {
                    if v["entries"].as_array()?.len() > 50 {
                        return None;
                    }
                    let rows = serde_json::from_value::<Vec<Version>>(v["entries"].take()).ok()?;
                    rows.iter()
                        .all(|r| {
                            (6..=32).contains(&r.id.len())
                                && r.id
                                    .bytes()
                                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                        })
                        .then(|| Arc::new(rows))
                });
            self.failed = self.rows.is_none();
        }
        true
    }
}
