//! Explicit per-agent launch defaults; per-model choices survive clearing a model.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Defaults {
    pub model: Option<String>,
    pub values_by_model: BTreeMap<String, BTreeMap<String, Value>>,
}
impl Defaults {
    pub fn values(&self) -> Option<Value> {
        let model = self
            .model
            .as_deref()
            .filter(|id| !crate::agent_env::trim(id).is_empty())?;
        let mut values = Map::new();
        values.insert("model".into(), model.into());
        if let Some(stored) = self.values_by_model.get(model) {
            values.extend(
                stored
                    .iter()
                    .filter(|(_, v)| v.is_string() || v.is_boolean())
                    .map(|(k, v)| (k.clone(), v.clone())),
            );
        }
        Some(Value::Object(values))
    }
    pub fn set_option(&mut self, id: &str, value: Option<Value>) {
        let Some(model) = &self.model else {
            return;
        };
        let values = self.values_by_model.entry(model.clone()).or_default();
        if let Some(value) = value {
            values.insert(id.into(), value);
        } else {
            values.remove(id);
        }
    }
    pub fn validate(&self) -> bool {
        self.model
            .as_deref()
            .is_none_or(crate::agent_arg_options::safe_session_value)
            && self.values_by_model.len() <= 100
            && self.values_by_model.iter().all(|(model, values)| {
                !model.is_empty()
                    && model.encode_utf16().count() <= 160
                    && values.len() <= 10
                    && values.iter().all(|(id, value)| {
                        !id.is_empty()
                            && id.len() <= 40
                            && id.as_bytes()[0].is_ascii_alphabetic()
                            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                            && (value.is_boolean()
                                || value
                                    .as_str()
                                    .is_some_and(crate::agent_arg_options::safe_session_value))
                    })
            })
    }
}
pub fn save(
    state: &mut crate::workspace::Workspace,
    store: &crate::workspace::Store,
    agent: &str,
    defaults: Defaults,
) -> crate::workspace::Result<()> {
    if crate::agent_model_catalog::find(agent).is_none() || !defaults.validate() {
        return Err("Invalid agent model defaults.".into());
    }
    let mut next = state.clone();
    next.agent_model_defaults.insert(agent.into(), defaults);
    store.save(&next)?;
    state.agent_model_defaults = next.agent_model_defaults;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn choosing_clearing_and_reselecting_preserves_explicit_options() {
        let mut defaults = Defaults::default();
        assert!(defaults.values().is_none());
        defaults.model = Some("gpt-5.6-sol".into());
        defaults.set_option("effort", Some("high".into()));
        defaults.model = None;
        assert!(defaults.values().is_none());
        defaults.model = Some("gpt-5.6-sol".into());
        assert_eq!(defaults.values().unwrap()["effort"], "high");
        defaults.set_option("effort", None);
        assert!(defaults.values().unwrap().get("effort").is_none());
        assert!(defaults.validate());
    }
    #[test]
    fn saved_choices_survive_restart_and_failed_write_retains_previous() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = crate::workspace::Store::open(dir.path()).unwrap();
        let defaults = Defaults {
            model: Some("gpt-5.6-sol".into()),
            ..Default::default()
        };
        save(&mut state, &store, "codex", defaults.clone()).unwrap();
        drop(store);
        let (_, state) = crate::workspace::Store::open(dir.path()).unwrap();
        assert_eq!(state.agent_model_defaults["codex"], defaults);
        let blocked = tempfile::tempdir().unwrap();
        let (store, mut state) = crate::workspace::Store::open(blocked.path()).unwrap();
        std::fs::create_dir(blocked.path().join("workspace.json")).unwrap();
        assert!(save(&mut state, &store, "codex", defaults).is_err());
        assert!(state.agent_model_defaults.is_empty());
        assert!(
            !Defaults {
                model: Some("--yolo".into()),
                ..Default::default()
            }
            .validate()
        );
    }
}
