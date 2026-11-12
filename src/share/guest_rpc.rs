//! Distinguish host replies from transport failures; some host replies have unknown outcomes.
pub const TASK_RESULT_NOT_AVAILABLE: &str =
    "Task result is not available yet; refresh the board before retrying.";
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuestRpcError {
    Host(String),
    Transport(String),
}
impl From<String> for GuestRpcError {
    fn from(value: String) -> Self {
        Self::Transport(value)
    }
}
impl From<&str> for GuestRpcError {
    fn from(value: &str) -> Self {
        Self::Transport(value.into())
    }
}
impl std::fmt::Display for GuestRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Host(text) | Self::Transport(text) => f.write_str(text),
        }
    }
}
