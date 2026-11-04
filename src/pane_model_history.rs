//! Last-observed display metadata. Never used as launch or Live Share authority.
use crate::{
    pane_session_model::{self as model, SavedPaneModel, ShownAgentModel},
    workspace::{Store, Workspace},
};

pub fn restored(saved: &SavedPaneModel) -> ShownAgentModel {
    ShownAgentModel {
        model: saved.model.clone(),
        name: saved.name.clone(),
        effort: saved.effort.clone(),
        chosen_effort: None,
        source: model::ModelSource::Saved,
        at: None,
    }
}

pub fn snapshot(shown: &ShownAgentModel) -> Option<SavedPaneModel> {
    let value = serde_json::to_value(shown).ok()?;
    model::pane_model_to_save(Some(&value))
}

/// Commit a batch once; failed disk writes leave the caller's workspace unchanged.
pub fn persist(
    state: &mut Workspace,
    store: &Store,
    updates: &[(u64, SavedPaneModel)],
) -> Result<bool, String> {
    let mut next = state.clone();
    let mut changed = false;
    for (id, model) in updates {
        let Some(pane) = next.sessions.iter_mut().find(|pane| pane.id == *id) else {
            continue;
        };
        if pane.last_model.as_ref() != Some(model) {
            pane.last_model = Some(model.clone());
            changed = true;
        }
    }
    if !changed {
        return Ok(false);
    }
    store.save(&next)?;
    state.sessions = next.sessions;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_placeholder_never_rewrites_itself_and_failed_save_preserves_previous_model() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.sessions.push(serde_json::from_value(serde_json::json!({"id":1,"project":null,"title":"Codex","launch":{"Agent":"codex"},"cwd":dir.path()})).unwrap());
        assert!(
            state.sessions[0].last_model.is_none(),
            "old workspaces need no migration"
        );
        let old = SavedPaneModel {
            model: "gpt-5.6-sol".into(),
            name: None,
            effort: Some("high".into()),
        };
        assert!(snapshot(&restored(&old)).is_none());
        assert!(persist(&mut state, &store, &[(1, old.clone())]).unwrap());
        assert!(!persist(&mut state, &store, &[(1, old.clone())]).unwrap());
        drop(store);
        let (store, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.sessions[0].last_model, Some(old.clone()));
        assert_eq!(
            restored(loaded.sessions[0].last_model.as_ref().unwrap()).source,
            model::ModelSource::Saved
        );
        let before = serde_json::to_value(&state).unwrap();
        std::fs::remove_file(dir.path().join("workspace.json")).unwrap();
        std::fs::create_dir(dir.path().join("workspace.json")).unwrap();
        let new = SavedPaneModel {
            model: "gpt-5.6-luna".into(),
            name: None,
            effort: None,
        };
        assert!(persist(&mut state, &store, &[(1, new)]).is_err());
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
        assert_eq!(state.sessions[0].last_model, Some(old));
    }
    #[test]
    fn validation_rejects_invalid_metadata_and_shell_models_without_changing_state() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.sessions.push(serde_json::from_value(serde_json::json!({"id":1,"project":null,"title":"Shell","launch":"Shell","cwd":dir.path()})).unwrap());
        let model = SavedPaneModel {
            model: "gpt-5.6-sol".into(),
            name: None,
            effort: None,
        };
        assert!(persist(&mut state, &store, &[(1, model)]).is_err());
        assert!(state.sessions[0].last_model.is_none());
        state.sessions[0].launch = crate::launch::Launch::Agent("codex".into());
        let model = SavedPaneModel {
            model: "bad\nmodel".into(),
            name: None,
            effort: None,
        };
        assert!(persist(&mut state, &store, &[(1, model)]).is_err());
        assert!(state.sessions[0].last_model.is_none());
    }
}
