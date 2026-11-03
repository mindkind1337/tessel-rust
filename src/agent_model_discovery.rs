//! Asynchronous model queries. One pending request per agent; failures retain cached lists.
use crate::{
    agent_model_cache::{Cache, Entry},
    agent_model_probe::{self, Failure},
    agent_model_process::Error,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{SystemTime, UNIX_EPOCH},
};

type Result = std::result::Result<Vec<Value>, Failure>;
pub struct Discovery {
    cache: Cache,
    pending: BTreeMap<String, Receiver<Result>>,
    errors: BTreeMap<String, Failure>,
    storage_error: bool,
    disk: Option<Receiver<bool>>,
    dirty: bool,
}
impl Discovery {
    pub fn open(dir: &Path) -> Self {
        Self {
            cache: Cache::open(dir),
            pending: BTreeMap::new(),
            errors: BTreeMap::new(),
            storage_error: false,
            disk: None,
            dirty: false,
        }
    }
    pub fn lists(&self) -> &BTreeMap<String, Entry> {
        self.cache.entries()
    }
    pub fn busy(&self, agent: &str) -> bool {
        self.pending.contains_key(agent)
    }
    pub fn error(&self, agent: &str) -> Option<&Failure> {
        self.errors.get(agent)
    }
    pub fn storage_error(&self) -> bool {
        self.storage_error
    }
    pub fn saving(&self) -> bool {
        self.dirty || self.disk.is_some()
    }

    pub fn refresh(
        &mut self,
        agent: &str,
        command: &str,
        wake: impl FnOnce() + Send + 'static,
    ) -> bool {
        let Some(spec) = agent_model_probe::spec(agent) else {
            return false;
        };
        if spec.protocol.is_some() {
            return false;
        }
        let custom = crate::agent_env::trim(command);
        let exe = if !custom.is_empty()
            && custom.len() <= 300
            && custom
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-\\/:".contains(&b))
        {
            custom
        } else {
            spec.exe
        }
        .to_owned();
        let id = agent.to_owned();
        self.start(
            agent,
            move || {
                let program = crate::agent_program::resolve(&exe)
                    .map_err(|_| Failure::Process(Error::Start))?
                    .ok_or(Failure::NotFound)?;
                let home = directories::BaseDirs::new().ok_or(Failure::Process(Error::Start))?;
                agent_model_probe::execute(&id, program.command(home.home_dir()))
            },
            wake,
        )
    }

    fn start(
        &mut self,
        agent: &str,
        job: impl FnOnce() -> Result + Send + 'static,
        wake: impl FnOnce() + Send + 'static,
    ) -> bool {
        if self.busy(agent) {
            return false;
        }
        self.errors.remove(agent);
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("agent-model-list".into())
            .spawn(move || {
                let _ = tx.send(job());
                wake();
            }) {
            Ok(_) => {
                self.pending.insert(agent.into(), rx);
                true
            }
            Err(_) => {
                self.errors
                    .insert(agent.into(), Failure::Process(Error::Start));
                false
            }
        }
    }

    /// UI-safe polling: no waits or filesystem operations. Disk snapshots are
    /// serialized on one writer, so an older completion cannot overwrite a newer one.
    pub fn poll(&mut self) {
        let completed: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(agent, rx)| match rx.try_recv() {
                Ok(result) => Some((agent.clone(), result)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some((agent.clone(), Err(Failure::Process(Error::Io))))
                }
            })
            .collect();
        for (agent, result) in completed {
            self.pending.remove(&agent);
            match result {
                Ok(models) => {
                    let models =
                        crate::agent_model_probe_parse::valid_listed(&Value::Array(models));
                    if models.is_empty() {
                        self.errors.insert(agent, Failure::Empty);
                        continue;
                    }
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
                    if self
                        .cache
                        .keep_in_memory(&agent, &Value::Array(models), now)
                        .is_err()
                    {
                        self.errors.insert(agent, Failure::Empty);
                        continue;
                    }
                    self.dirty = true;
                    self.errors.remove(&agent);
                }
                Err(error) => {
                    self.errors.insert(agent, error);
                }
            }
        }
        if let Some(rx) = &self.disk {
            match rx.try_recv() {
                Ok(success) => {
                    self.disk = None;
                    self.storage_error = !success;
                }
                Err(TryRecvError::Disconnected) => {
                    self.disk = None;
                    self.storage_error = true;
                }
                Err(TryRecvError::Empty) => (),
            }
        }
        if self.dirty && self.disk.is_none() {
            let snapshot = self.cache.clone();
            let (tx, rx) = mpsc::channel();
            self.dirty = false;
            match std::thread::Builder::new()
                .name("agent-model-cache".into())
                .spawn(move || {
                    let _ = tx.send(snapshot.persist().is_ok());
                }) {
                Ok(_) => self.disk = Some(rx),
                Err(_) => self.storage_error = true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn complete(state: &mut Discovery) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while (!state.pending.is_empty() || state.disk.is_some() || state.dirty)
            && std::time::Instant::now() < until
        {
            state.poll();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(state.pending.is_empty());
        assert!(state.disk.is_none());
        assert!(!state.dirty);
    }
    #[test]
    fn duplicate_requests_do_not_start_and_failure_preserves_last_success() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = Discovery::open(dir.path());
        state
            .cache
            .keep("codex", &json!([{"id":"old-model"}]), 1.0)
            .unwrap();
        let (tx, rx) = mpsc::channel();
        assert!(state.start("codex", move || rx.recv().unwrap(), || {}));
        assert!(!state.start("codex", || panic!("duplicate must never run"), || {}));
        state.poll();
        assert!(state.busy("codex"));
        assert_eq!(state.lists()["codex"].models[0]["id"], "old-model");
        tx.send(Err(Failure::Empty)).unwrap();
        complete(&mut state);
        assert_eq!(state.error("codex"), Some(&Failure::Empty));
        assert_eq!(state.lists()["codex"].models[0]["id"], "old-model");
        assert!(state.start(
            "codex",
            || Ok(vec![json!({"id":"new-model","extra":"discard"})]),
            || {}
        ));
        complete(&mut state);
        assert!(state.error("codex").is_none());
        assert_eq!(
            state.lists()["codex"].models,
            vec![json!({"id":"new-model","label":"new-model"})]
        );
        assert_eq!(Discovery::open(dir.path()).lists(), state.lists());
    }
    #[test]
    fn disk_failure_retains_results_and_next_success_persists_all_agents() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("cache");
        std::fs::write(&blocked, b"not a directory").unwrap();
        let mut state = Discovery::open(&blocked);
        assert!(state.start("codex", || Ok(vec![json!({"id":"one"})]), || {}));
        complete(&mut state);
        assert!(state.storage_error());
        assert!(state.error("codex").is_none());
        assert_eq!(state.lists()["codex"].models[0]["id"], "one");
        std::fs::remove_file(&blocked).unwrap();
        assert!(state.start("claude", || Ok(vec![json!({"id":"two"})]), || {}));
        assert!(state.start("grok", || Ok(vec![json!({"id":"three"})]), || {}));
        complete(&mut state);
        assert!(!state.storage_error());
        assert!(!state.saving());
        assert_eq!(Discovery::open(&blocked).lists(), state.lists());
        assert_eq!(state.lists().len(), 3);
    }

    #[test]
    fn disconnected_worker_clears_busy_and_muse_never_runs_as_bare_serve() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = Discovery::open(dir.path());
        let (tx, rx) = mpsc::channel();
        state.pending.insert("codex".into(), rx);
        drop(tx);
        state.poll();
        assert!(!state.busy("codex"));
        assert_eq!(state.error("codex"), Some(&Failure::Process(Error::Io)));
        assert!(!state.refresh("muse", "must-not-run", || panic!(
            "unsupported protocol must not run"
        )));
        assert!(!state.refresh("unknown", "must-not-run", || panic!(
            "unknown provider must not run"
        )));
    }
}
