//! Local agent menu preferences. Changing visibility never stops existing panes.
use crate::workspace::{Result, Store, Workspace};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub enabled: bool,
    pub command: String,
    pub args: String,
    pub env: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: true,
            command: String::new(),
            args: String::new(),
            env: String::new(),
        }
    }
}

impl Preferences {
    pub fn customized(&self) -> bool {
        [&self.command, &self.args, &self.env]
            .iter()
            .any(|s| !crate::agent_env::trim(s).is_empty())
    }
    pub fn reset(&mut self) {
        let enabled = self.enabled;
        *self = Self {
            enabled,
            ..Self::default()
        };
    }
}

/// Preserve the legacy Cursor preference until a canonical entry is saved.
pub fn get<'a>(state: &'a Workspace, id: &str) -> Option<&'a Preferences> {
    let canonical = crate::agent_catalog::find(id).map_or(id, |preset| preset.id);
    state.agent_prefs.get(canonical).or_else(|| {
        (canonical == "cursor")
            .then(|| state.agent_prefs.get("cursor-agent"))
            .flatten()
    })
}

pub fn save(state: &mut Workspace, store: &Store, id: &str, value: Preferences) -> Result<()> {
    let id = crate::agent_catalog::find(id).ok_or("Unknown agent.")?.id;
    let mut next = state.clone();
    next.agent_prefs.insert(id.into(), value);
    store.save(&next)?;
    state.agent_prefs = next.agent_prefs;
    Ok(())
}

/// Snapshot the effective local terminal launch without mutating saved arguments.
#[cfg(test)]
pub fn for_launch(state: &Workspace, id: &str) -> Preferences {
    for_launch_with_models(state, id, None)
}
pub fn for_launch_with_models(
    state: &Workspace,
    id: &str,
    discovered: Option<&[serde_json::Value]>,
) -> Preferences {
    let mut prefs = get(state, id).cloned().unwrap_or_default();
    if let Some(values) = state
        .agent_model_defaults
        .get(id)
        .and_then(crate::agent_model_defaults::Defaults::values)
    {
        let models = crate::agent_model_catalog::models_for(id, discovered);
        let flags = crate::agent_model_launch::text(id, &values, &prefs.args, Some(&models));
        if !flags.is_empty() {
            prefs.args = format!("{flags} {}", prefs.args).trim_end().into();
        }
    }
    prefs
}

pub fn enabled(state: &Workspace, id: &str) -> bool {
    get(state, id).is_none_or(|prefs| prefs.enabled)
}

pub fn set_enabled(state: &mut Workspace, store: &Store, id: &str, value: bool) -> Result<()> {
    let id = crate::agent_catalog::find(id).ok_or("Unknown agent.")?.id;
    if enabled(state, id) == value {
        return Ok(());
    }
    let mut prefs = get(state, id).cloned().unwrap_or_default();
    prefs.enabled = value;
    save(state, store, id, prefs)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovered_options_control_launch_without_inventing_unadvertised_effort() {
        let mut state = Workspace::default();
        let mut defaults = crate::agent_model_defaults::Defaults {
            model: Some("dynamic-model".into()),
            ..Default::default()
        };
        defaults.set_option("effort", Some("high".into()));
        state.agent_model_defaults.insert("claude".into(), defaults);
        let rows =
            vec![serde_json::json!({"id":"dynamic-model", "label":"Dynamic", "effortLevels":[]})];
        let models = crate::agent_model_listed::catalog("claude", &rows).unwrap();
        assert_eq!(
            for_launch_with_models(&state, "claude", Some(&models)).args,
            "--model dynamic-model"
        );
        let rows = vec![
            serde_json::json!({"id":"dynamic-model", "label":"Dynamic", "effortLevels":["high"]}),
        ];
        let models = crate::agent_model_listed::catalog("claude", &rows).unwrap();
        assert_eq!(
            for_launch_with_models(&state, "claude", Some(&models)).args,
            "--model dynamic-model --effort high"
        );
        // Source Codex merging inherits seed options when an unknown listed
        // model advertises none; Claude above replaces its seed entirely.
        state
            .agent_model_defaults
            .insert("codex".into(), state.agent_model_defaults["claude"].clone());
        let models = crate::agent_model_listed::catalog(
            "codex",
            &[serde_json::json!({"id":"dynamic-model", "label":"Dynamic"})],
        )
        .unwrap();
        assert_eq!(
            for_launch_with_models(&state, "codex", Some(&models)).args,
            "-m dynamic-model -c model_reasoning_effort=high"
        );
    }

    #[test]
    fn model_flags_are_explicit_and_manual_arguments_win_without_mutating_settings() {
        let mut state = Workspace::default();
        assert!(for_launch(&state, "codex").args.is_empty());
        let mut defaults = crate::agent_model_defaults::Defaults {
            model: Some("gpt-5.6-sol".into()),
            ..Default::default()
        };
        defaults.set_option("effort", Some("high".into()));
        state.agent_model_defaults.insert("codex".into(), defaults);
        assert_eq!(
            for_launch(&state, "codex").args,
            "-m gpt-5.6-sol -c model_reasoning_effort=high"
        );
        state.agent_prefs.insert(
            "codex".into(),
            Preferences {
                args: "--model manual --effort low".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            for_launch(&state, "codex").args,
            "--model manual --effort low"
        );
        assert_eq!(
            get(&state, "codex").unwrap().args,
            "--model manual --effort low"
        );
        state.agent_model_defaults.get_mut("codex").unwrap().model = None;
        state.agent_prefs.clear();
        assert!(for_launch(&state, "codex").args.is_empty());
    }

    #[test]
    fn launch_settings_persist_reset_and_visibility_does_not_erase_them() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let prefs = Preferences {
            enabled: false,
            command: "custom-cli".into(),
            args: "--model test".into(),
            env: "SYNTHETIC=test".into(),
        };
        save(&mut state, &store, "codex", prefs.clone()).unwrap();
        set_enabled(&mut state, &store, "codex", true).unwrap();
        assert_eq!(get(&state, "codex").unwrap().env, prefs.env);
        drop(store);
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let mut loaded = get(&state, "codex").unwrap().clone();
        assert!(loaded.customized());
        loaded.enabled = false;
        loaded.reset();
        assert!(!loaded.enabled);
        assert!(!loaded.customized());
        save(&mut state, &store, "codex", loaded.clone()).unwrap();
        assert_eq!(get(&state, "codex"), Some(&loaded));
        let blank = Preferences {
            command: "\u{feff}".into(),
            ..Default::default()
        };
        assert!(!blank.customized());
    }

    #[test]
    fn failed_save_keeps_all_previous_launch_settings() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let before = Preferences {
            command: "first".into(),
            ..Default::default()
        };
        state.agent_prefs.insert("codex".into(), before.clone());
        std::fs::create_dir(dir.path().join("workspace.json")).unwrap();
        assert!(save(&mut state, &store, "codex", Preferences::default()).is_err());
        assert_eq!(get(&state, "codex"), Some(&before));
    }

    #[test]
    fn legacy_cursor_visibility_uses_canonical_id_without_losing_the_choice() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.agent_prefs.insert(
            "cursor-agent".into(),
            Preferences {
                enabled: false,
                ..Default::default()
            },
        );
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
