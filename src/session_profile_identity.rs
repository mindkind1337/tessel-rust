//! Portable shape of a local-only session profile binding.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Binding {
    Default,
    Managed { account_id: String },
    Configured { scope_hash: String },
}

impl Binding {
    pub fn valid(&self) -> bool {
        match self {
            Self::Default => true,
            Self::Managed { account_id } => {
                !account_id.is_empty()
                    && account_id.len() <= 64
                    && account_id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            }
            Self::Configured { scope_hash } => {
                scope_hash.len() == 64
                    && scope_hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }
        }
    }
}
