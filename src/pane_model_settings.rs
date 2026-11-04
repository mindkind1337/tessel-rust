//! Transactional per-pane launch choices. No terminal commands or global-default writes.
use crate::{
    launch::Launch,
    workspace::{Result, Store, Workspace},
};
use serde_json::Value;

pub fn save(
    state: &mut Workspace,
    store: &Store,
    pane: u64,
    discovered: Option<&[Value]>,
    option: &str,
    value: Option<&Value>,
) -> Result<Option<Value>> {
    if option == "model"
        && value.is_some_and(|v| {
            !v.is_null()
                && !v
                    .as_str()
                    .is_some_and(crate::agent_arg_options::safe_session_value)
        })
    {
        return Err("Invalid pane model choice.".into());
    }
    let session = state
        .sessions
        .iter()
        .find(|s| s.id == pane)
        .ok_or("Pane no longer exists.")?;
    let Launch::Agent(agent) = &session.launch else {
        return Err("This pane has no terminal model picker.".into());
    };
    let catalog =
        crate::agent_model_catalog::find(agent).ok_or("This agent has no model choices.")?;
    let models = crate::agent_model_catalog::models_for(agent, discovered);
    let base = crate::pane_session_model::launch_session_values(
        session.session_options.as_ref(),
        &state.agent_model_defaults,
        agent,
    );
    let effective = crate::pane_session_model::effective_model_id(&models, None);
    if option != "model" {
        let model = base
            .as_ref()
            .and_then(|v| v.get("model"))
            .and_then(Value::as_str)
            .or(effective)
            .ok_or("Choose a model first.")?;
        let spec = crate::agent_model_catalog::model_options(catalog, Some(&models), model)
            .iter()
            .find(|s| s["id"].as_str() == Some(option))
            .ok_or("Unknown model option.")?;
        if !crate::agent_model_defaults_ui::is_launch_option(agent, spec) {
            return Err("This option requires a live agent command.".into());
        }
        if let Some(value) = value.filter(|v| !v.is_null()) {
            let valid = match spec["kind"]["type"].as_str() {
                Some("boolean") => value.is_boolean(),
                Some("select") => spec["kind"]["choices"].as_array().is_some_and(|choices| {
                    choices
                        .iter()
                        .any(|choice| choice.get("value") == Some(value))
                }),
                _ => false,
            };
            if !valid {
                return Err("Invalid value for this model option.".into());
            }
        }
    }
    let next_values = crate::pane_session_model::next_pane_values(
        agent,
        &models,
        base.as_ref(),
        effective,
        option,
        value,
    );
    if next_values.as_ref().is_some_and(|v| {
        crate::pane_session_model::valid_pane_session_options_value(v).as_ref() != Some(v)
    }) {
        return Err("Invalid pane model choice.".into());
    }
    let mut next = state.clone();
    next.sessions
        .iter_mut()
        .find(|s| s.id == pane)
        .expect("validated pane")
        .session_options = next_values.clone();
    store.save(&next)?;
    state.sessions = next.sessions;
    Ok(next_values)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_disk_write_preserves_in_memory_choice() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.sessions.push(serde_json::from_value(serde_json::json!({"id": 1, "project": null, "title":"Codex", "launch":{"Agent":"codex"}, "cwd":dir.path(), "session_options":{"model":"old-model"}})).unwrap());
        std::fs::create_dir(dir.path().join("workspace.json")).unwrap();
        let before = serde_json::to_value(&state).unwrap();
        assert!(
            save(
                &mut state,
                &store,
                1,
                None,
                "model",
                Some(&Value::String("new-model".into()))
            )
            .is_err()
        );
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
    }
    #[test]
    fn choosing_effort_without_a_pinned_model_uses_the_displayed_default_row() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.sessions.push(serde_json::from_value(serde_json::json!({"id":1,"project":null,"title":"Codex","launch":{"Agent":"codex"},"cwd":dir.path()})).unwrap());
        let models = crate::agent_model_catalog::models_for("codex", None);
        let default = crate::pane_session_model::effective_model_id(&models, None).unwrap();
        let values = save(
            &mut state,
            &store,
            1,
            None,
            "effort",
            Some(&Value::String("high".into())),
        )
        .unwrap()
        .unwrap();
        assert_eq!(values["model"], default);
        assert_eq!(values["effort"], "high");
        assert!(state.agent_model_defaults.is_empty());
        let before = state.sessions[0].session_options.clone();
        assert!(
            save(
                &mut state,
                &store,
                1,
                None,
                "effort",
                Some(&Value::String("not-a-choice".into()))
            )
            .is_err()
        );
        assert_eq!(state.sessions[0].session_options, before);
    }
    #[test]
    fn choices_are_scoped_persisted_and_do_not_replace_manual_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.sessions.push(serde_json::from_value(serde_json::json!({"id": 1, "project": null, "title":"Codex", "launch":{"Agent":"codex"}, "cwd":dir.path()})).unwrap());
        state.sessions.push(serde_json::from_value(serde_json::json!({"id": 2, "project": null, "title":"Shell", "launch":"Shell", "cwd":dir.path()})).unwrap());
        state.agent_model_defaults.insert(
            "codex".into(),
            crate::agent_model_defaults::Defaults {
                model: Some("global-model".into()),
                ..Default::default()
            },
        );
        let defaults = state.agent_model_defaults.clone();
        let chosen = Value::String("pane-model".into());
        assert_eq!(
            save(&mut state, &store, 1, None, "model", Some(&chosen)).unwrap(),
            Some(serde_json::json!({"model":"pane-model"}))
        );
        assert_eq!(state.agent_model_defaults, defaults);
        assert!(state.sessions[1].session_options.is_none());
        assert!(
            crate::agent_preferences::for_pane_launch(
                &state,
                "codex",
                None,
                state.sessions[0].session_options.as_ref()
            )
            .args
            .contains("pane-model")
        );
        state.agent_prefs.insert(
            "codex".into(),
            crate::agent_preferences::Preferences {
                args: "--model manual".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            crate::agent_preferences::for_pane_launch(
                &state,
                "codex",
                None,
                state.sessions[0].session_options.as_ref()
            )
            .args,
            "--model manual"
        );
        assert!(save(&mut state, &store, 2, None, "model", Some(&chosen)).is_err());
        assert!(save(&mut state, &store, 99, None, "model", Some(&chosen)).is_err());
        let before = state.sessions[0].session_options.clone();
        assert!(
            save(
                &mut state,
                &store,
                1,
                None,
                "model",
                Some(&Value::String("bad\nvalue".into()))
            )
            .is_err()
        );
        assert_eq!(state.sessions[0].session_options, before);
        assert_eq!(
            save(&mut state, &store, 1, None, "model", None).unwrap(),
            None
        );
        assert_eq!(state.agent_model_defaults, defaults);
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert!(loaded.sessions[0].session_options.is_none());
    }
}
