use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Largest decoded chunk of file content or terminal output in one message.
pub const MAX_CHUNK_BYTES: usize = 64 * 1024;
/// Linux limits: one name, a whole path.
pub const MAX_COMPONENT_BYTES: usize = 255;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_PATH_COMPONENTS: usize = 1024;

const fn encoded_len(bytes: usize) -> usize {
    bytes.div_ceil(3) * 4
}

/// Decodes canonical padded base64 (the standard alphabet), refusing a text longer than
/// `max` bytes could need before decoding anything.
fn decode_bounded(text: &str, max: usize) -> Option<Vec<u8>> {
    if text.len() > encoded_len(max) {
        return None;
    }
    let bytes = STANDARD.decode(text).ok()?;
    (bytes.len() <= max).then_some(bytes)
}

/// Original bytes, sent as base64, at most `MAX_CHUNK_BYTES` once decoded.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Chunk(Vec<u8>);

impl Chunk {
    pub fn new(bytes: Vec<u8>) -> Option<Self> {
        (bytes.len() <= MAX_CHUNK_BYTES).then_some(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl Serialize for Chunk {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for Chunk {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        decode_bounded(&text, MAX_CHUNK_BYTES)
            .map(Self)
            .ok_or_else(|| serde::de::Error::custom("invalid chunk"))
    }
}

/// A path below an opened root, as the raw bytes of each name: a Linux file name need not
/// be UTF-8, and a lossy conversion would name another file. Never absolute, never "." or
/// "..", never a "/" or NUL inside a name. Empty means the root itself. A label for people
/// to read is a separate field wherever one is shown.
#[derive(Clone, Debug, PartialEq, Eq, Default, Hash)]
pub struct RelPath(Vec<Vec<u8>>);

fn valid_component(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= MAX_COMPONENT_BYTES
        && name != b"."
        && name != b".."
        && !name.contains(&b'/')
        && !name.contains(&0)
}

impl RelPath {
    pub fn root() -> Self {
        Self(Vec::new())
    }

    pub fn from_components(components: Vec<Vec<u8>>) -> Option<Self> {
        let joined =
            components.iter().map(Vec::len).sum::<usize>() + components.len().saturating_sub(1);
        (components.len() <= MAX_PATH_COMPONENTS
            && joined <= MAX_PATH_BYTES
            && components.iter().all(|c| valid_component(c)))
        .then_some(Self(components))
    }

    pub fn components(&self) -> &[Vec<u8>] {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The names joined with "/", relative to the root (empty for the root).
    pub fn to_bytes(&self) -> Vec<u8> {
        self.0.join(&b'/')
    }
}

impl Serialize for RelPath {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(|name| STANDARD.encode(name)))
    }
}

impl<'de> Deserialize<'de> for RelPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let texts = Vec::<String>::deserialize(deserializer)?;
        if texts.len() > MAX_PATH_COMPONENTS {
            return Err(serde::de::Error::custom("invalid path"));
        }
        let mut components = Vec::with_capacity(texts.len());
        for text in &texts {
            components.push(
                decode_bounded(text, MAX_COMPONENT_BYTES)
                    .ok_or_else(|| serde::de::Error::custom("invalid path"))?,
            );
        }
        Self::from_components(components).ok_or_else(|| serde::de::Error::custom("invalid path"))
    }
}
