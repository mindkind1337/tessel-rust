//! Content versions detect stale reads; this is not a filesystem compare-and-swap.
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};
pub(super) const CONFLICT: &str = "File conflict: the host file changed. Your draft is kept.";
pub(super) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(super) fn check(path: &Path, expected: Option<&str>) -> Result<bool, String> {
    if expected.is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit())) {
        return Err(CONFLICT.into());
    }
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && expected.is_none() => {
            return Ok(false);
        }
        Err(_) => return Err(CONFLICT.into()),
    };
    let expected = expected.ok_or(CONFLICT)?;
    if !file.metadata().map_err(|_| CONFLICT)?.is_file() {
        return Err(CONFLICT.into());
    }
    let mut bytes = Vec::new();
    file.take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CONFLICT)?;
    if bytes.len() > 8 * 1024 * 1024 || !hash(&bytes).eq_ignore_ascii_case(expected) {
        return Err(CONFLICT.into());
    }
    Ok(true)
}
