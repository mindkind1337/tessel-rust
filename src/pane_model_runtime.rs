//! Local conversation model telemetry, scoped to the launched terminal profile.
use crate::{
    pane_model_profile::Profile,
    pane_model_worker::{Failure, Worker},
    pane_session_model::{ModelSource, ShownAgentModel},
};
use std::time::{Duration, Instant};

type Scope = (Profile, String);

pub struct Runtime {
    scope: Option<Scope>,
    worker: Worker<Option<Scope>, Option<ShownAgentModel>>,
    model: Option<ShownAgentModel>,
    revision: u64,
    read_revision: u64,
    quiet_at: Instant,
    refresh_at: Instant,
    retry_at: Instant,
}
impl Default for Runtime {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            scope: None,
            worker: Worker::default(),
            model: None,
            revision: 0,
            read_revision: 0,
            quiet_at: now,
            refresh_at: now,
            retry_at: now,
        }
    }
}
impl Runtime {
    /// Nonblocking, called only for displayed panes. Terminal output debounces
    /// for 350 ms; a periodic 20 s refresh also covers quiet or busy terminals.
    pub fn tick(
        &mut self,
        profile: Option<&Profile>,
        conversation: Option<&str>,
        revision: u64,
        now: Instant,
        wake: impl FnOnce() + Send + 'static,
    ) -> Option<&ShownAgentModel> {
        let scope = profile
            .filter(|p| p.agent() == "codex")
            .zip(conversation.filter(|id| crate::claude::valid_uuid(id)))
            .map(|(profile, id)| (profile.clone(), id.to_owned()));
        if self.scope != scope {
            self.scope = scope;
            self.worker.bind(self.scope.clone());
            self.model = None;
            self.refresh_at = now;
            self.retry_at = now;
            self.read_revision = revision;
        }
        if let Some(Ok(Some(model))) = self.worker.poll() {
            self.model = Some(model);
        }
        let scope = self.scope.as_ref()?;
        if revision != self.revision {
            self.revision = revision;
            self.quiet_at = now + Duration::from_millis(350);
        }
        let due =
            now >= self.refresh_at || (revision != self.read_revision && now >= self.quiet_at);
        if due && now >= self.retry_at && !self.worker.busy() {
            let scope = scope.clone();
            match self.worker.start(move || read_codex(&scope), wake) {
                Ok(true) => {
                    self.refresh_at = now + Duration::from_secs(20);
                    self.read_revision = revision;
                }
                Ok(false) => self.retry_at = now + Duration::from_millis(250),
                Err(_) => self.retry_at = now + Duration::from_secs(5),
            }
        }
        self.model.as_ref()
    }
}

fn read_codex((profile, id): &Scope) -> Result<Option<ShownAgentModel>, Failure> {
    let Some(mut file) = crate::codex_sessions::open_verified(profile.root(), profile.cwd(), id)
        .map_err(|_| Failure::Io)?
    else {
        return Ok(None);
    };
    crate::pane_model_log::latest(&mut file, codex_turn)
}

// Temporary integration adapter for the source codexModelFromText contract.
// Replace with Franklin's shared reader when its differential fixture lands.
fn codex_turn(text: &str) -> Option<ShownAgentModel> {
    text.lines().rev().find_map(|line| {
        if line.len() > 2 * 1024 * 1024 || !line.contains("\"model\"") {
            return None;
        }
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        let payload = value.get("payload")?;
        let (model, effort) =
            if value.get("type").and_then(|value| value.as_str()) == Some("turn_context") {
                (
                    payload.get("model")?,
                    payload
                        .get("effort")
                        .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                        .or_else(|| {
                            payload
                                .get("reasoning_effort")
                                .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                        })
                        .or_else(|| {
                            payload
                                .get("collaboration_mode")?
                                .get("settings")?
                                .get("reasoning_effort")
                        }),
                )
            } else if payload.get("type")?.as_str()? == "thread_settings_applied" {
                let settings = payload.get("thread_settings")?;
                (
                    settings.get("model")?,
                    settings
                        .get("reasoning_effort")
                        .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                        .or_else(|| settings.get("effort")),
                )
            } else {
                return None;
            };
        let clean = crate::pane_session_model::saved_pane_model(Some(
            &serde_json::json!({"model":model,"effort":effort}),
        ))?;
        let at = value
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .and_then(|t| u64::try_from(t.timestamp_millis()).ok());
        Some(ShownAgentModel {
            model: clean.model,
            name: None,
            effort: clean.effort,
            chosen_effort: None,
            source: ModelSource::Session,
            at,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    const ID: &str = "12345678-1234-4234-8234-123456789abc";

    fn fixture(dir: &std::path::Path, model: &str) -> (Profile, std::path::PathBuf) {
        let root = dir.join("profile");
        fs::create_dir_all(root.join("archived_sessions")).unwrap();
        let path = root
            .join("archived_sessions")
            .join(format!("rollout-{ID}.jsonl"));
        let metadata = serde_json::json!({"type":"session_meta","payload":{"id":ID,"cwd":dir,"timestamp":"2026-11-03T00:00:00Z"}});
        let turn =
            serde_json::json!({"type":"turn_context","payload":{"model":model,"effort":"high"}});
        fs::write(&path, format!("{metadata}\n{turn}\n")).unwrap();
        let mut command = portable_pty::CommandBuilder::new("synthetic");
        command.env_clear();
        command.cwd(dir);
        command.env("CODEX_HOME", root);
        (Profile::capture("codex", &command, dir).unwrap(), path)
    }

    #[test]
    fn reads_only_the_verified_profile_and_preserves_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let (profile, _) = fixture(dir.path(), "gpt-5.6-sol");
        let model = read_codex(&(profile, ID.into())).unwrap().unwrap();
        assert_eq!(model.model, "gpt-5.6-sol");
        assert_eq!(model.effort.as_deref(), Some("high"));
        assert_eq!(model.source, ModelSource::Session);
        let shown = codex_turn("{\"type\":\"event_msg\",\"timestamp\":\"2026-11-03T00:00:00Z\",\"payload\":{\"type\":\"thread_settings_applied\",\"thread_settings\":{\"model\":\"new-model\",\"reasoning_effort\":\"xhigh\"}}}").unwrap();
        assert_eq!(shown.model, "new-model");
        assert_eq!(shown.effort.as_deref(), Some("xhigh"));
        assert!(shown.at.is_some());
        assert!(
            codex_turn("{\"type\":\"tool_output\",\"payload\":{\"model\":\"not-a-turn\"}}")
                .is_none()
        );
    }

    #[test]
    fn asynchronous_refresh_keeps_transient_failures_but_clears_changed_scope() {
        let _guard = crate::pane_model_worker::TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (profile, path) = fixture(dir.path(), "first-model");
        let mut state = Runtime::default();
        let start = Instant::now();
        let deadline = start + Duration::from_secs(5);
        while state
            .tick(Some(&profile), Some(ID), 0, Instant::now(), || {})
            .is_none()
        {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state.model.as_ref().unwrap().model, "first-model");
        fs::remove_file(path).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        state.tick(
            Some(&profile),
            Some(ID),
            0,
            start + Duration::from_secs(21),
            move || {
                tx.send(()).unwrap();
            },
        );
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            state
                .tick(
                    Some(&profile),
                    Some(ID),
                    0,
                    start + Duration::from_secs(22),
                    || {}
                )
                .unwrap()
                .model,
            "first-model"
        );
        assert!(
            state
                .tick(None, None, 0, start + Duration::from_secs(23), || {})
                .is_none()
        );
        assert!(state.model.is_none());
    }
}
