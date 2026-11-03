// Adapted from Tessel main/agentModelList.js / Orca.
// Copyright (c) 2026 Lovecast Inc. MIT; see THIRD_PARTY_NOTICES.md.
//! Successful discovery results only. Opening this cache never starts a CLI.
use crate::agent_model_probe_parse::valid_listed;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub const AGENTS: [&str; 8] = [
    "claude",
    "codex",
    "grok",
    "opencode",
    "pi",
    "cursor",
    "antigravity",
    "muse",
];
const FILE: &str = "agent-models.json";
// More than the maximum sanitized contents of all eight listings combined.
const MAX_CACHE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub models: Vec<Value>,
    pub fetched_at: f64,
}

#[derive(Clone)]
pub struct Cache {
    path: PathBuf,
    entries: BTreeMap<String, Entry>,
}

impl Cache {
    pub fn open(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let entries = read(&path).unwrap_or_default();
        Self { path, entries }
    }

    pub fn entries(&self) -> &BTreeMap<String, Entry> {
        &self.entries
    }

    /// Reject empty/unsupported results without changing the last good list.
    /// A storage failure keeps the new success in memory; the caller can report
    /// the error. The next successful discovery retries the entire snapshot.
    pub fn keep(&mut self, agent: &str, models: &Value, fetched_at: f64) -> Result<(), String> {
        self.keep_in_memory(agent, models, fetched_at)?;
        self.persist()
    }

    pub fn keep_in_memory(
        &mut self,
        agent: &str,
        models: &Value,
        fetched_at: f64,
    ) -> Result<(), String> {
        if !AGENTS.contains(&agent) {
            return Err("unsupported".into());
        }
        let models = valid_listed(models);
        if models.is_empty() {
            return Err("empty".into());
        }
        let fetched_at = if fetched_at.is_finite() {
            fetched_at
        } else {
            0.0
        };
        self.entries
            .insert(agent.into(), Entry { models, fetched_at });
        Ok(())
    }

    pub fn persist(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&self.entries).map_err(|e| e.to_string())?;
        let parent = self.path.parent().ok_or("No cache directory")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        crate::workspace::atomic_write(&self.path, &bytes)
    }
}

fn read(path: &Path) -> Option<BTreeMap<String, Entry>> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_CACHE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        return None;
    }
    let data: Value = serde_json::from_slice(&bytes).ok()?;
    let mut entries = BTreeMap::new();
    for agent in AGENTS {
        let models = valid_listed(&data[agent]["models"]);
        if !models.is_empty() {
            let fetched_at = data[agent]["fetchedAt"]
                .as_f64()
                .filter(|n| n.is_finite())
                .unwrap_or(0.0);
            entries.insert(agent.into(), Entry { models, fetched_at });
        }
    }
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip_sanitizes_and_failed_results_keep_last_success() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = Cache::open(dir.path());
        for agent in AGENTS {
            cache.keep(agent, &json!([{"id":"model-one", "label":"One", "secret":"discard"}, {"id":"model-one"}]), 1234.5).unwrap();
        }
        let bytes = fs::read(dir.path().join(FILE)).unwrap();
        assert!(
            cache
                .keep("codex", &json!([{"id":"bad id"}]), 555.0)
                .is_err()
        );
        assert!(
            cache
                .keep("unknown", &json!([{"id":"model-two"}]), 555.0)
                .is_err()
        );
        assert_eq!(bytes, fs::read(dir.path().join(FILE)).unwrap());
        let loaded = Cache::open(dir.path());
        assert_eq!(cache.entries(), loaded.entries());
        assert_eq!(loaded.entries().len(), 8);
        for entry in loaded.entries().values() {
            assert_eq!(entry.models, vec![json!({"id":"model-one", "label":"One"})]);
            assert_eq!(entry.fetched_at, 1234.5);
        }
    }

    #[test]
    fn damaged_cache_and_unknown_agents_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        for input in ["{", "null", "[]", "true"] {
            fs::write(&path, input).unwrap();
            assert!(Cache::open(dir.path()).entries().is_empty());
        }
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "codex":{"models":[{"id":"model-one"}],"fetchedAt":"invalid"},
                "claude":{"models":[]}, "unknown":{"models":[{"id":"model-two"}]}
            }))
            .unwrap(),
        )
        .unwrap();
        let loaded = Cache::open(dir.path());
        assert_eq!(loaded.entries().len(), 1);
        assert_eq!(loaded.entries()["codex"].fetched_at, 0.0);
    }

    #[test]
    fn disk_failure_retains_memory_and_next_success_retries_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        fs::write(&cache_dir, b"not a directory").unwrap();
        let mut cache = Cache::open(&cache_dir);
        assert!(
            cache
                .keep("codex", &json!([{"id":"model-one"}]), 1.0)
                .is_err()
        );
        assert_eq!(cache.entries()["codex"].models[0]["id"], "model-one");
        fs::remove_file(&cache_dir).unwrap();
        cache
            .keep("claude", &json!([{"id":"model-two"}]), f64::NAN)
            .unwrap();
        assert_eq!(Cache::open(&cache_dir).entries(), cache.entries());
        assert_eq!(cache.entries()["claude"].fetched_at, 0.0);
    }

    #[test]
    fn oversized_cache_is_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let file = fs::File::create(dir.path().join(FILE)).unwrap();
        file.set_len(MAX_CACHE_BYTES + 1).unwrap();
        assert!(Cache::open(dir.path()).entries().is_empty());
    }
}
