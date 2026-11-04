use serde::{Deserialize, Deserializer, Serialize};
use std::fmt::Write as _;

/// Largest identifier or sequence number: every one stays exact in a JavaScript number.
pub const MAX_ID: u64 = (1u64 << 53) - 1;

/// 32 lowercase hexadecimal digits. One spelling per value, so two spellings can never
/// name the same thing (a deduplication key written in capitals is another key).
fn hex32(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

macro_rules! token {
    ($(#[$doc:meta])* $name:ident, $what:literal) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: &str) -> Option<Self> {
                hex32(value).then(|| Self(value.to_owned()))
            }

            /// From 16 random bytes the caller drew.
            pub fn from_bytes(bytes: [u8; 16]) -> Self {
                let mut text = String::with_capacity(32);
                for byte in bytes {
                    let _ = write!(text, "{byte:02x}");
                }
                Self(text)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                Self::parse(&value).ok_or_else(|| serde::de::Error::custom(concat!("invalid ", $what)))
            }
        }
    };
}

token!(
    /// One run of a host daemon. A restarted daemon has another: what the old one owned is gone.
    Epoch,
    "host epoch"
);
token!(
    /// A stream of events (a terminal's output, a tree's changes), unique within an epoch.
    StreamId,
    "stream id"
);
token!(
    /// Chosen by the client for an operation with an effect (creating a terminal), so a retry
    /// after a lost reply, even over a new connection, finds the first outcome instead of
    /// doing it again.
    DedupKey,
    "deduplication key"
);
