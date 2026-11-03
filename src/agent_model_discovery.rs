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
    catalogs: BTreeMap<String, Vec<Value>>,
}
impl Discovery {
    pub fn open(dir: &Path) -> Self {
        let cache = Cache::open(dir);
        let catalogs = cache
            .entries()
            .iter()
            .filter_map(|(agent, entry)| {
                crate::agent_model_listed::catalog(agent, &entry.models)
                    .map(|rows| (agent.clone(), rows))
            })
            .collect();
        Self {
            cache,
            catalogs,
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
    pub fn active(&self) -> bool {
        !self.pending.is_empty() || self.saving()
    }
    pub fn models(&self, agent: &str) -> Option<&[Value]> {
        self.catalogs.get(agent).map(Vec::as_slice)
    }
    pub fn can_refresh(agent: &str) -> bool {
        if agent == "muse" {
            return true;
        }
        agent_model_probe::spec(agent).is_some_and(|s| s.protocol.is_none())
            && crate::agent_model_listed::catalog(agent, &[]).is_some()
    }

    pub fn refresh(
        &mut self,
        agent: &str,
        command: &str,
        wake: impl FnOnce() + Send + 'static,
    ) -> bool {
        if !Self::can_refresh(agent) {
            return false;
        }
        if agent == "muse" {
            return self.start(agent, crate::muse_model_probe::list, wake);
        }
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
                .all(|b| b.is_ascii_alphanumeric() || b"_.-\\/: ".contains(&b))
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
                        .map_or(0.0, |d| d.as_millis() as f64);
                    if self
                        .cache
                        .keep_in_memory(&agent, &Value::Array(models), now)
                        .is_err()
                    {
                        self.errors.insert(agent, Failure::Empty);
                        continue;
                    }
                    self.dirty = true;
                    if let Some(rows) = crate::agent_model_listed::catalog(
                        &agent,
                        &self.cache.entries()[&agent].models,
                    ) {
                        self.catalogs.insert(agent.clone(), rows);
                    }
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
        let mut state = Discovery::open(dir.path());
        assert_eq!(state.models("codex").unwrap()[0]["id"], "old-model");
        let (tx, rx) = mpsc::channel();
        assert!(state.start("codex", move || rx.recv().unwrap(), || {}));
        assert!(!state.start("codex", || panic!("duplicate must never run"), || {}));
        state.poll();
        assert!(state.busy("codex"));
        assert!(state.active());
        assert_eq!(state.lists()["codex"].models[0]["id"], "old-model");
        assert_eq!(state.models("codex").unwrap()[0]["id"], "old-model");
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
        assert_eq!(
            Discovery::open(dir.path()).models("codex"),
            state.models("codex")
        );
        assert_eq!(state.models("codex").unwrap()[0]["id"], "new-model");
        assert!(!state.active());
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
    fn disconnected_worker_clears_busy_and_unknown_provider_never_runs() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = Discovery::open(dir.path());
        let (tx, rx) = mpsc::channel();
        state.pending.insert("codex".into(), rx);
        drop(tx);
        state.poll();
        assert!(!state.busy("codex"));
        assert_eq!(state.error("codex"), Some(&Failure::Process(Error::Io)));
        assert!(Discovery::can_refresh("muse"));
        assert!(!state.refresh("unknown", "must-not-run", || panic!(
            "unknown provider must not run"
        )));
    }

    #[cfg(windows)]
    #[test]
    fn custom_command_with_spaces_runs_only_listing_and_restores_catalog() {
        let dir = tempfile::Builder::new()
            .prefix("model discovery ")
            .tempdir()
            .unwrap();
        let shim = dir.path().join("synthetic.cmd");
        std::fs::write(
            &shim,
            "@echo off\r\n\"%~dp0node.exe\" \"%~dp0listing.cjs\" %*\r\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("listing.cjs"), r#"
if (JSON.stringify(process.argv.slice(2)) !== '["debug","models"]') process.exit(7);
console.log(JSON.stringify({models:[{slug:'synthetic-model',display_name:'Synthetic model',supported_reasoning_levels:[{effort:'high'}]}]}));
"#).unwrap();
        let cache = dir.path().join("cache");
        let mut state = Discovery::open(&cache);
        assert!(state.refresh("codex", shim.to_str().unwrap(), || {}));
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while state.active() && std::time::Instant::now() < until {
            state.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!state.active());
        assert_eq!(state.error("codex"), None);
        let restored = Discovery::open(&cache);
        let models = restored.models("codex").unwrap();
        assert_eq!(models[0]["id"], "synthetic-model");
        assert_eq!(
            models[0]["options"][0]["kind"]["choices"][0]["value"],
            "high"
        );
    }
}
