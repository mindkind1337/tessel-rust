//! Model and effort choices for native chats; never carries launch permissions.
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
