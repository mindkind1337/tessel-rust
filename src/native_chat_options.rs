//! Model and effort choices for native chats; never carries launch permissions.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Save only the explicit native override, preserving independent global fallback.
/// The caller updates its live chat only after this transaction succeeds.
pub fn save_choice(
    state: &mut crate::workspace::Workspace,
    store: &crate::workspace::Store,
    pane: u64,
    option: &str,
    value: Option<&Value>,
) -> Result<Option<Value>, String> {
    let session = state
        .sessions
        .iter()
        .find(|s| s.id == pane)
        .ok_or("Pane no longer exists.")?;
    let provider = match session.launch {
        crate::launch::Launch::CodexChat => "codex",
        crate::launch::Launch::ClaudeChat => "claude",
        _ => return Err("This pane is not a native chat.".into()),
    };
    let mut options = Options::from_value(session.native_options.as_ref());
    let selected = match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        _ => return Err("Invalid native chat choice.".into()),
    };
    match option {
        "model" => options.model = selected,
        "effort" => options.effort = selected,
        _ => return Err("Unsupported native chat option.".into()),
    }
    options.normalize();
    if !options.valid_for(provider) {
        return Err("Invalid native chat choice.".into());
    }
    let values = options.value();
    let mut next = state.clone();
    next.sessions
        .iter_mut()
        .find(|s| s.id == pane)
        .expect("validated pane")
        .native_options = values.clone();
    store.save(&next)?;
    state.sessions = next.sessions;
    Ok(values)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub model: String,
    pub effort: String,
}

impl Options {
    pub fn from_value(value: Option<&Value>) -> Self {
        value
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    pub fn valid_for(&self, provider: &str) -> bool {
        (self.model.is_empty() || crate::agent_arg_options::safe_session_value(self.model.trim()))
            && match provider {
                "codex" => matches!(
                    self.effort.as_str(),
                    "" | "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
                ),
                "claude" => matches!(
                    self.effort.as_str(),
                    "" | "low" | "medium" | "high" | "xhigh" | "max"
                ),
                _ => false,
            }
    }

    pub fn normalize(&mut self) {
        self.model = self.model.trim().to_owned();
    }

    /// Vue's chatOpen falls back independently for model and effort, on resume too.
    pub fn effective(&self, defaults: Option<&Value>) -> Self {
        let inherit = |own: &str, key| {
            if own.is_empty() {
                defaults
                    .and_then(|v| v.get(key))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            } else {
                own.to_owned()
            }
        };
        Self {
            model: inherit(&self.model, "model").trim().to_owned(),
            effort: inherit(&self.effort, "effort"),
        }
    }

    pub fn value(&self) -> Option<Value> {
        (!self.model.is_empty() || !self.effort.is_empty())
            .then(|| serde_json::json!({"model":self.model,"effort":self.effort}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn transactional_picker_preserves_fallback_scope_and_failed_write() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = crate::workspace::Store::open(dir.path()).unwrap();
        let project = state.add_project(dir.path()).unwrap();
        let cwd = state.active().unwrap().path.clone();
        for (id, launch) in [(1, "CodexChat"), (2, "ClaudeChat"), (3, "Shell")] {
            state.sessions.push(
                serde_json::from_value(
                    json!({"id":id,"project":project,"title":launch,"launch":launch,"cwd":cwd}),
                )
                .unwrap(),
            );
        }
        let high = json!("high");
        assert_eq!(
            save_choice(&mut state, &store, 1, "effort", Some(&high)).unwrap(),
            Some(json!({"model":"","effort":"high"}))
        );
        assert!(state.sessions[0].session_options.is_none());
        assert!(state.sessions[1].native_options.is_none());
        assert!(state.agent_model_defaults.is_empty());
        save_choice(&mut state, &store, 1, "model", Some(&json!("gpt-custom"))).unwrap();
        let cleared = save_choice(&mut state, &store, 1, "model", None).unwrap();
        assert_eq!(
            Options::from_value(cleared.as_ref()).effective(Some(&json!({"model":"global"}))),
            Options {
                model: "global".into(),
                effort: "high".into()
            }
        );
        for (id, key, value) in [
            (1, "permissions", json!("yolo")),
            (2, "effort", json!("ultra")),
            (3, "model", json!("x")),
            (1, "model", json!(true)),
            (1, "model", json!("--unsafe")),
        ] {
            let before = serde_json::to_value(&state).unwrap();
            assert!(save_choice(&mut state, &store, id, key, Some(&value)).is_err());
            assert_eq!(serde_json::to_value(&state).unwrap(), before);
        }
        let before = serde_json::to_value(&state).unwrap();
        std::fs::remove_file(dir.path().join("workspace.json")).unwrap();
        std::fs::create_dir(dir.path().join("workspace.json")).unwrap();
        assert!(save_choice(&mut state, &store, 1, "model", Some(&json!("different"))).is_err());
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
    }
    #[test]
    fn independent_fallback_and_clearing_follow_current_defaults() {
        let defaults = json!({"model":"global", "effort":"high", "permissions":"yolo"});
        let choice = Options {
            model: "pane".into(),
            ..Default::default()
        };
        assert_eq!(
            choice.effective(Some(&defaults)),
            Options {
                model: "pane".into(),
                effort: "high".into()
            }
        );
        assert_eq!(Options::default().effective(None), Options::default());
        assert_eq!(
            Options::default().effective(Some(&defaults)).model,
            "global"
        );
        assert_eq!(
            choice.effective(Some(&json!({"effort":"low"}))).effort,
            "low"
        );
        assert_eq!(Options::from_value(choice.value().as_ref()), choice);
        assert!(Options::default().value().is_none());
        for effort in ["max", "ultra"] {
            assert!(
                Options {
                    effort: effort.into(),
                    ..Default::default()
                }
                .valid_for("codex")
            );
        }
        assert!(
            Options {
                model: "model@20250929+extra=1".into(),
                effort: "xhigh".into()
            }
            .valid_for("codex")
        );
        assert!(
            !Options {
                effort: "none".into(),
                ..Default::default()
            }
            .valid_for("claude")
        );
        assert!(
            !Options {
                effort: "bogus".into(),
                ..Default::default()
            }
            .valid_for("codex")
        );
        assert_eq!(
            Options {
                model: " model ".into(),
                ..Default::default()
            }
            .effective(None)
            .model,
            "model"
        );
    }
    #[test]
    fn rejects_untrusted_fields_and_arguments() {
        assert!(serde_json::from_value::<Options>(json!({"permissions":"yolo"})).is_err());
        for model in ["--unsafe", "bad;command", "a\nb", "$(code)"] {
            assert!(
                !Options {
                    model: model.into(),
                    ..Default::default()
                }
                .valid_for("claude")
            );
        }
        assert!(
            !Options {
                effort: "bogus".into(),
                ..Default::default()
            }
            .valid_for("claude")
        );
        assert!(
            Options {
                model: "claude-opus-4-8[1m]".into(),
                effort: "max".into()
            }
            .valid_for("claude")
        );
    }
}
