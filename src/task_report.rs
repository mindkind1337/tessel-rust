//! Bounded report content; paths are inert text, never execution or file authority.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Succeeded,
    Failed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub outcome: Outcome,
    pub summary: String,
    pub files: Vec<String>,
    pub by: Option<u64>,
    pub at_ms: u64,
}
impl Report {
    pub fn validate(&self) -> Result<(), String> {
        if self.summary.trim().is_empty()
            || self.summary.encode_utf16().count() > 2000
            || self.files.len() > 50
            || self.files.iter().any(|p| p.encode_utf16().count() > 300)
            || self.by == Some(0)
        {
            return Err("Invalid task report.".into());
        }
        Ok(())
    }
}
