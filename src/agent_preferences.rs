//! Local agent menu preferences. Changing visibility never stops existing panes.
use crate::workspace::{Result, Store, Workspace};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub enabled: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self { enabled: true }
    }
}

pub fn enabled(state: &Workspace, id: &str) -> bool {
    let canonical = crate::agent_catalog::find(id).map_or(id, |preset| preset.id);
    state
        .agent_prefs
        .get(canonical)
        .or_else(|| {
            if canonical == "cursor" {
                state.agent_prefs.get("cursor-agent")
            } else {
                None
            }
        })
        .is_none_or(|prefs| prefs.enabled)
}

pub fn set_enabled(state: &mut Workspace, store: &Store, id: &str, value: bool) -> Result<()> {
    let id = crate::agent_catalog::find(id).ok_or("Unknown agent.")?.id;
    if enabled(state, id) == value {
        return Ok(());
    }
    let mut next = state.clone();
    next.agent_prefs.entry(id.into()).or_default().enabled = value;
    store.save(&next)?;
    state.agent_prefs = next.agent_prefs;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_cursor_visibility_uses_canonical_id_without_losing_the_choice() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state
            .agent_prefs
            .insert("cursor-agent".into(), Preferences { enabled: false });
        state.validate().unwrap();
        assert!(!enabled(&state, "cursor"));
        assert!(!enabled(&state, "cursor-agent"));
        set_enabled(&mut state, &store, "cursor-agent", true).unwrap();
        assert!(enabled(&state, "cursor"));
        assert!(enabled(&state, "cursor-agent"));
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert!(enabled(&loaded, "cursor-agent"));
    }

    #[test]
    fn old_settings_default_to_enabled_and_disabled_agents_survive_restart() {
        let old: Workspace = serde_json::from_str("{}").unwrap();
        assert!(enabled(&old, "codex"));
        let defaults: Preferences = serde_json::from_str("{}").unwrap();
        assert!(defaults.enabled);
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.sessions.push(crate::workspace::SavedSession {
            pane_name: Some("Existing pane".into()),
            id: 1,
            project: None,
            title: "codex".into(),
            launch: crate::launch::Launch::Agent("codex".into()),
            cwd: dir.path().into(),
        });
        set_enabled(&mut state, &store, "codex", false).unwrap();
        assert!(!enabled(&state, "codex"));
        assert!(enabled(&state, "claude"));
        assert_eq!(state.sessions.len(), 1);
        drop(store);
        let (store, mut loaded) = Store::open(dir.path()).unwrap();
        assert!(!enabled(&loaded, "codex"));
        assert_eq!(
            loaded.sessions[0].pane_name.as_deref(),
            Some("Existing pane")
        );
        set_enabled(&mut loaded, &store, "codex", true).unwrap();
        assert!(enabled(&loaded, "codex"));
    }

    #[test]
    fn failed_save_does_not_hide_agent_and_unknown_ids_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        std::fs::create_dir(dir.path().join("workspace.json")).unwrap();
        assert!(set_enabled(&mut state, &store, "codex", false).is_err());
        assert!(enabled(&state, "codex"));
        assert!(state.agent_prefs.is_empty());
        assert!(set_enabled(&mut state, &store, "unknown", false).is_err());
        state
            .agent_prefs
            .insert("unknown".into(), Preferences::default());
        assert!(state.validate().is_err());
    }
}
