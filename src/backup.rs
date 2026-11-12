//! Portable metadata backups. No credentials, browser profiles or running processes.
use crate::workspace::{Result, Workspace};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};
const LIMIT: usize = 4 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Backup {
    format: String,
    version: u32,
    workspace: Workspace,
}
pub fn export(path: &Path, state: &Workspace) -> Result<()> {
    state.validate()?;
    let mut portable = state.clone();
    portable.quick_commands.clear();
    portable.custom_agents.clear();
    portable
        .provider_accounts_prefs
        .host_account_defaults
        .clear();
    for prefs in portable.agent_prefs.values_mut() {
        prefs.env.clear();
    }
    for pane in &mut portable.sessions {
        if matches!(&pane.launch, crate::launch::Launch::Agent(id) if id.starts_with("custom-")) {
            pane.launch = crate::launch::Launch::Shell;
        }
        if let Some(native) = &mut pane.native_host {
            native.identity = None;
        }
        pane.codex_seed = None;
        pane.profile_binding = None;
        pane.launch_fingerprint = None;
    }
    let bytes = serde_json::to_vec_pretty(&Backup {
        format: "tessel-rust-settings".into(),
        version: 1,
        workspace: portable,
    })
    .map_err(|_| "Cannot encode settings")?;
    if bytes.len() > LIMIT {
        return Err("Settings backup exceeds 4 MiB".into());
    }
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().ok_or("No export directory")?)
        .map_err(|e| e.to_string())?;
    temp.write_all(&bytes)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    temp.persist_noclobber(path)
        .map_err(|_| "Destination exists or cannot be saved; existing file retained")?;
    Ok(())
}
pub fn import(path: &Path) -> Result<Workspace> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > LIMIT {
        return Err("Settings backup exceeds 4 MiB".into());
    }
    let mut backup: Backup =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid Rust settings backup")?;
    if backup.format != "tessel-rust-settings" || backup.version != 1 {
        return Err("Unsupported settings backup version".into());
    }
    // A portable file must never introduce launch commands or environment
    // secrets. Visibility is inert and remains portable.
    backup.workspace.quick_commands.clear();
    backup.workspace.custom_agents.clear();
    backup
        .workspace
        .provider_accounts_prefs
        .host_account_defaults
        .clear();
    backup.workspace.orchestration_prefs.team_wake_unconfirmed = false;
    backup.workspace.orchestration_prefs.confirm_workers = true;
    backup.workspace.agent_folder_trust = false;
    for prefs in backup.workspace.agent_prefs.values_mut() {
        prefs.reset();
    }
    for pane in &mut backup.workspace.sessions {
        if matches!(&pane.launch, crate::launch::Launch::Agent(id) if id.starts_with("custom-")) {
            pane.launch = crate::launch::Launch::Shell;
        }
        if let Some(native) = &mut pane.native_host {
            native.identity = None;
        }
        pane.codex_seed = None;
        pane.profile_binding = None;
        pane.launch_fingerprint = None;
    }
    backup.workspace.validate()?;
    Ok(backup.workspace)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_host_backups_keep_an_inert_marker_without_resumable_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("native.json");
        let identity = serde_json::json!({
            "epoch": tessel_host_protocol::Epoch::from_bytes([1; 16]),
            "terminal": tessel_host_protocol::TerminalId::from_bytes([2; 16]),
        });
        let mut state = Workspace::default();
        state.sessions.push(
            serde_json::from_value(serde_json::json!({
                "id": 1, "project": null, "title": "Native fixture", "cwd": dir.path(),
                "launch": {"Ssh": {"alias": "fixture", "folder": ""}},
                "native_host": {"identity": identity.clone()},
            }))
            .unwrap(),
        );
        state.validate().unwrap();
        export(&path, &state).unwrap();
        assert!(
            state.sessions[0]
                .native_host
                .as_ref()
                .unwrap()
                .identity
                .is_some()
        );
        let mut backup: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(backup["workspace"]["sessions"][0]["native_host"]["identity"].is_null());
        backup["workspace"]["sessions"][0]["native_host"]["identity"] = identity;
        std::fs::write(&path, serde_json::to_vec(&backup).unwrap()).unwrap();
        let restored = import(&path).unwrap();
        assert!(
            restored.sessions[0]
                .native_host
                .as_ref()
                .unwrap()
                .identity
                .is_none()
        );
        assert!(
            serde_json::from_value::<crate::workspace::SavedHostTerminal>(
                serde_json::json!({"identity": null, "lease": "must-not-be-accepted"})
            )
            .is_err()
        );
        state.sessions[0].launch = crate::launch::Launch::Shell;
        assert!(state.validate().is_err());
    }
    #[test]
    fn export_omits_environment_and_import_cannot_introduce_launch_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("portable.json");
        let mut state = Workspace::default();
        state.agent_prefs.insert(
            "codex".into(),
            crate::agent_preferences::Preferences {
                enabled: false,
                command: "synthetic-cli".into(),
                args: "--synthetic".into(),
                env: "SYNTHETIC_TOKEN=fixture-only-secret".into(),
            },
        );
        export(&path, &state).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("fixture-only-secret"));
        assert!(!text.contains("SYNTHETIC_TOKEN"));
        assert!(
            state.agent_prefs["codex"]
                .env
                .contains("fixture-only-secret")
        );
        let mut malicious: serde_json::Value = serde_json::from_str(&text).unwrap();
        malicious["workspace"]["agent_prefs"]["codex"]["env"] = "INJECTED=fixture".into();
        malicious["workspace"]["quick_commands"] = serde_json::json!([
            {"id": 1, "name": "Untrusted", "text": "curl evil", "press_enter": true}
        ]);
        std::fs::write(&path, serde_json::to_vec(&malicious).unwrap()).unwrap();
        let imported = import(&path).unwrap();
        let pref = &imported.agent_prefs["codex"];
        assert!(!pref.enabled);
        assert!(!pref.customized());
        assert!(imported.quick_commands.is_empty());
    }

    #[test]
    fn restore_keeps_previous_settings_and_never_starts_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let (store, before) = crate::workspace::Store::open(dir.path()).unwrap();
        let mut after = Workspace {
            language: crate::i18n::Language::French,
            ..Default::default()
        };
        after.sessions.push(crate::workspace::SavedSession {
            native_host: None,
            last_model: None,
            codex_seed: None,
            conversation_id: None,
            profile_binding: None,
            launch_fingerprint: None,
            session_options: None,
            native_options: None,
            worktree: None,
            pane_name: None,
            broadcast: true,
            id: 4,
            project: None,
            title: "sleeping".into(),
            launch: crate::launch::Launch::Shell,
            cwd: dir.path().to_owned(),
        });
        let previous = store.restore(&before, &after).unwrap();
        let old: Workspace = serde_json::from_slice(&std::fs::read(previous).unwrap()).unwrap();
        assert!(old.sessions.is_empty());
        drop(store);
        let (_, restored) = crate::workspace::Store::open(dir.path()).unwrap();
        assert_eq!(restored.sessions.len(), 1);
        assert_eq!(restored.language, crate::i18n::Language::French);
        let mut exhausted = Workspace {
            next_id: u64::MAX - 1,
            ..Default::default()
        };
        assert!(exhausted.id().is_err());
    }
    #[test]
    fn roundtrip_refuses_overwrite_and_untrusted_formats() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut state = Workspace::default();
        state.add_project(dir.path()).unwrap();
        state.agent_prefs.insert(
            "codex".into(),
            crate::agent_preferences::Preferences {
                enabled: false,
                ..Default::default()
            },
        );
        state.sessions.push(crate::workspace::SavedSession {
            native_host: None,
            last_model: None,
            codex_seed: None,
            conversation_id: None,
            profile_binding: None,
            launch_fingerprint: Some("a".repeat(64)),
            session_options: Some(serde_json::json!({"model":"gpt-5.6-sol","effort":"high"})),
            native_options: None,
            worktree: None,
            pane_name: Some("Custom agent".into()),
            broadcast: false,
            id: 1,
            project: state.active_project,
            title: "codex".into(),
            launch: crate::launch::Launch::Agent("codex".into()),
            cwd: dir.path().to_owned(),
        });
        state.sessions[0].profile_binding =
            Some(crate::session_profile_identity::Binding::Managed {
                account_id: "synthetic".into(),
            });
        state.sessions[0].codex_seed = Some(
            crate::codex_terminal::Discovery::new(
                &crate::agent_preferences::Preferences {
                    env: "CODEX_HOME=synthetic".into(),
                    ..Default::default()
                },
                dir.path(),
            )
            .unwrap()
            .seed(),
        );
        state.sessions[0].last_model = Some(crate::pane_session_model::SavedPaneModel {
            model: "gpt-5.6-sol".into(),
            name: None,
            effort: Some("high".into()),
        });
        export(&path, &state).unwrap();
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("codex_seed")
        );
        assert!(export(&path, &Workspace::default()).is_err());
        let mut imported = import(&path).unwrap();
        assert_eq!(
            imported.sessions[0].last_model,
            state.sessions[0].last_model
        );
        assert!(imported.sessions[0].profile_binding.is_none());
        assert!(imported.sessions[0].codex_seed.is_none());
        assert!(imported.sessions[0].launch_fingerprint.is_none());
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("launch_fingerprint")
        );
        imported.migrate_native_threads();
        assert_eq!(imported.projects, state.projects);
        assert_eq!(imported.agent_prefs, state.agent_prefs);
        assert_eq!(
            imported.sessions[0].session_options,
            state.sessions[0].session_options
        );
        assert_eq!(
            imported.sessions[0].pane_name.as_deref(),
            Some("Custom agent")
        );
        assert_eq!(imported.sessions[0].title, "codex");
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["workspace"]["sessions"][0]["launch_fingerprint"] =
            "untrusted-plaintext-signature".into();
        value["workspace"]["sessions"][0]["codex_seed"] =
            serde_json::json!({"since_ms": 1, "scope": "a".repeat(64)});
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(import(&path).unwrap().sessions[0].codex_seed.is_none());
        assert!(
            import(&path).unwrap().sessions[0]
                .launch_fingerprint
                .is_none()
        );
        value["workspace"]["sessions"] = serde_json::json!([{"id":1,"project":null,"title":"bad","launch":{"Agent":"arbitrary-command"},"cwd":dir.path()}]);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(import(&path).is_err());
        std::fs::write(&path, vec![b' '; LIMIT + 1]).unwrap();
        assert!(import(&path).is_err());
        std::fs::write(&path, b"{}").unwrap();
        assert!(import(&path).is_err());
    }
}
