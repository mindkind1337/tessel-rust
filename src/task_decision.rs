//! Stored questions and user answers. Request identity prevents stale UI replies.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub text: String,
    pub at_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub request: u64,
    pub question: String,
    pub options: Vec<String>,
    pub by: Option<u64>,
    pub asked_at_ms: u64,
    pub answer: Option<Answer>,
}
impl Decision {
    pub fn pending(&self) -> bool {
        self.answer.is_none()
    }
    pub fn validate(&self, next_id: u64) -> Result<(), String> {
        if self.request == 0
            || self.request >= next_id
            || self.by == Some(0)
            || self.question.trim().is_empty()
            || self.question.encode_utf16().count() > 500
            || self.options.len() > 6
            || self
                .options
                .iter()
                .any(|v| v.trim().is_empty() || v.encode_utf16().count() > 80)
            || self
                .answer
                .as_ref()
                .is_some_and(|a| a.text.trim().is_empty() || a.text.encode_utf16().count() > 500)
        {
            return Err("Invalid task decision.".into());
        }
        Ok(())
    }
}
