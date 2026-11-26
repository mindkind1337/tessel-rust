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
    comparison: Option<Receiver<Option<crate::saved_version_diff::Patch>>>,
    pub diff: Option<Arc<crate::saved_version_diff::Patch>>,
    pub diff_failed: bool,
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
        self.pending.is_some() || self.comparison.is_some()
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
        self.diff = None;
        self.diff_failed = false;
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
    fn compare(&mut self, session: Option<&GuestSession>, saved: Arc<String>) {
        let Some(key) = &self.key else { return };
        let Some(session) = session.filter(|s| s.generation() == key.scope.session) else {
            self.diff_failed = true;
            return;
        };
        let query = json!({"op":"files.read","projectId":key.scope.project,"path":key.path});
        let Ok(reply) = session.request(query) else {
            self.diff_failed = true;
            return;
        };
        let name = key.path.rsplit('/').next().unwrap_or(&key.path).to_owned();
        let (send, receive) = std::sync::mpsc::channel();
        self.comparison = Some(receive);
        std::thread::spawn(move || {
            let patch = reply
                .recv_timeout(Duration::from_secs(25))
                .ok()
                .and_then(Result::ok)
                .and_then(|v| crate::guest_file_editor::decode_text(&v, 2 * 1024 * 1024))
                .map(|current| {
                    crate::saved_version_diff::Patch::new(crate::saved_version_diff::text_diff(
                        &name, &saved, &current,
                    ))
                });
            let _ = send.send(patch);
        });
    }
    pub fn poll(&mut self, now: Instant, session: Option<&GuestSession>) -> bool {
        if let Some(reply) = &self.comparison {
            let patch = match reply.try_recv() {
                Ok(patch) => patch,
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => None,
            };
            self.diff = patch.map(Arc::new);
            self.diff_failed = self.diff.is_none();
            self.comparison = None;
            return true;
        }
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
            if let Some(text) = self.text.clone() {
                self.compare(session, text);
            }
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

pub(crate) fn comparison_title(fr: bool, empty: bool) -> &'static str {
    match (fr, empty) {
        (true, true) => "Aucune différence.",
        (false, true) => "No differences.",
        (true, false) => "Différences avec le fichier de l’hôte",
        (false, false) => "Changes against host file",
    }
}
pub(crate) fn comparison_error(fr: bool) -> &'static str {
    if fr {
        "Comparaison indisponible. Le fichier actuel doit être un texte UTF-8 accessible de 2 Mio ou moins."
    } else {
        "Comparison unavailable. The current host file must be accessible UTF-8 text of 2 MiB or less."
    }
}
