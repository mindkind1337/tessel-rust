//! Native first-turn contract. The owning launch controller must hold a live,
//! project-scoped request, stop on errors/cancellation and enforce its deadline.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Pending,
    Connecting,
    Submitted,
    Cancelled,
    Failed,
}

pub trait Native {
    /// Submit the owner's task to a fresh chat without replacing its composer.
    /// A used chat is cancelled rather than steered. This does not authorize
    /// itself from persisted task metadata or bypass the provider's permissions.
    fn begin_task_turn(
        &mut self,
        prompt: &str,
        connection_attempted: bool,
    ) -> Result<Phase, String>;
}

pub fn validate_prompt(prompt: &str) -> Result<(), String> {
    if prompt.trim().is_empty() || prompt.len() > 64 * 1024 || prompt.contains('\0') {
        Err("Task instructions must contain 1 to 65536 bytes without null characters.".into())
    } else {
        Ok(())
    }
}
