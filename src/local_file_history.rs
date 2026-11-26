//! Private saved versions for the native local editor, using Tessel's History layout.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

// A poll can miss more than one save. In that case reload once so no visible version is lost.
static LAST_SAVE: std::sync::Mutex<(u64, PathBuf)> = std::sync::Mutex::new((0, PathBuf::new()));
pub fn changed_since(cursor: &mut u64, file: Option<&Path>) -> bool {
    let latest = LAST_SAVE.lock().unwrap_or_else(|e| e.into_inner());
    let count = latest.0.wrapping_sub(*cursor);
    *cursor = latest.0;
    file.is_some_and(|f| count > 1 || (count == 1 && f == latest.1))
}

const LIMIT: usize = 50;
pub(crate) const MAX_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Entry {
    pub id: String,
    pub timestamp: i64,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resource_key: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct Index {
    resource: String,
    entries: Vec<Entry>,
}

fn folder(profile: &Path, path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let resource = path.to_str()?;
    if resource.encode_utf16().count() >= 4096 {
        return None;
    }
    let key = format!("{:x}", Sha256::digest(resource.to_lowercase().as_bytes()));
    Some(profile.join("History").join(&key[..24]))
}

fn valid_id(id: &str) -> bool {
    (6..=32).contains(&id.len())
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

fn index(dir: &Path, file: &Path) -> io::Result<Index> {
    use std::io::Read;
    let resource = file
        .to_str()
        .ok_or_else(|| io::Error::other("Invalid history path"))?;
    match fs::File::open(dir.join("entries.json")) {
        Ok(file) => {
            let data: Index = serde_json::from_reader(file.take(128 * 1024))?;
            if data.entries.len() > LIMIT
                || !data.entries.iter().all(|e| valid_id(&e.id))
                || data.resource.to_lowercase() != resource.to_lowercase()
            {
                return Err(io::Error::other("Invalid history index"));
            }
            Ok(data)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Index {
            resource: resource.to_owned(),
            entries: Vec::new(),
        }),
        Err(e) => Err(e),
    }
}

pub fn list(profile: &Path, file: &Path) -> io::Result<Vec<Entry>> {
    let Some(dir) = folder(profile, file) else {
        return Ok(Vec::new());
    };
    let mut entries = index(&dir, file)?.entries;
    entries.reverse();
    Ok(entries)
}

/// Project permission checks happen in the host dispatcher. Only return versions
/// recorded against exactly this resource, including on case-sensitive volumes.
pub(crate) fn shared_read(
    profile: &Path,
    file: &Path,
    id: Option<&str>,
) -> Result<serde_json::Value, String> {
    let dir = folder(profile, file).ok_or("Invalid history path")?;
    let exact = format!(
        "{:x}",
        Sha256::digest(file.to_str().ok_or("Invalid resource")?.as_bytes())
    );
    for p in [
        profile.join("History"),
        dir.clone(),
        dir.join("entries.json"),
    ] {
        match p.canonicalize() {
            Ok(real) if real == p => {}
            Err(e)
                if e.kind() == io::ErrorKind::NotFound
                    && fs::symlink_metadata(&p)
                        .is_err_and(|e| e.kind() == io::ErrorKind::NotFound) => {}
            _ => return Err("History unavailable".into()),
        }
    }
    let index = index(&dir, file).map_err(|_| "History unavailable")?;
    let mut entries = index
        .entries
        .into_iter()
        .rev()
        .filter(|e| e.resource_key.as_ref() == Some(&exact));
    if let Some(id) = id {
        let entry = entries.find(|e| e.id == id).ok_or("Version unavailable")?;
        let path = dir.join(entry.id);
        if path.canonicalize().ok().as_ref() != Some(&path) {
            return Err("Version unavailable".into());
        }
        return Ok(serde_json::json!({"text":bounded_text(&path, MAX_BYTES)?}));
    }
    Ok(
        serde_json::json!({"entries":entries.map(|e| serde_json::json!({"id":e.id,"source":if e.source=="save" {"save"} else {"other"},"timestamp":e.timestamp})).collect::<Vec<_>>() }),
    )
}

pub fn diff(profile: &Path, root: &Path, file: &Path, id: &str) -> Result<String, String> {
    let dir = folder(profile, file).ok_or("Invalid history path")?;
    let old = version_text(&dir, file, id)?;
    let path = crate::files::confined(root, file)?;
    let current = bounded_text(&path, crate::files::MAX_TEXT)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    Ok(text_diff(&name, &old, &current))
}

fn version_text(dir: &Path, file: &Path, id: &str) -> Result<String, String> {
    if !valid_id(id)
        || !index(dir, file)
            .map_err(|e| e.to_string())?
            .entries
            .iter()
            .any(|e| e.id == id)
    {
        return Err("Saved version is no longer available.".into());
    }
    bounded_text(&dir.join(id), MAX_BYTES)
}
fn bounded_text(path: &Path, limit: usize) -> Result<String, String> {
    use std::io::Read;
    let mut s = String::new();
    fs::File::open(path)
        .and_then(|f| f.take(limit as u64 + 1).read_to_string(&mut s))
        .map_err(|e| e.to_string())?;
    if s.len() > limit {
        return Err("File exceeds the text editor limit.".into());
    }
    Ok(s)
}

fn text_diff(name: &str, old: &str, current: &str) -> String {
    similar::TextDiff::configure()
        .timeout(std::time::Duration::from_millis(500))
        .diff_lines(old, current)
        .unified_diff()
        .header(&format!("a/{name}"), &format!("b/{name}"))
        .to_string()
}

pub fn record(profile: &Path, file: &Path, text: &str) -> io::Result<()> {
    let Some(dir) = folder(profile, file) else {
        return Ok(());
    };
    if append(&dir, file, text)? {
        let mut latest = LAST_SAVE.lock().unwrap_or_else(|e| e.into_inner());
        latest.0 = latest.0.wrapping_add(1);
        file.clone_into(&mut latest.1);
    }
    Ok(())
}

/// SSH paths are case-sensitive and are only used as resource identifiers.
pub(crate) fn remote_versions(
    profile: &Path,
    alias: &str,
    path: &str,
    saved: Option<&str>,
) -> io::Result<Vec<Entry>> {
    let (dir, resource) = remote_resource(profile, alias, path)?;
    let file = Path::new(&resource);
    if let Some(text) = saved {
        append(&dir, file, text)?;
    }
    let mut versions = index(&dir, file)?.entries;
    versions.reverse();
    Ok(versions)
}

fn remote_resource(profile: &Path, alias: &str, path: &str) -> io::Result<(PathBuf, String)> {
    if alias.is_empty() || alias.contains('/') || !path.starts_with('/') {
        return Err(io::Error::other("Invalid SSH history resource"));
    }
    let resource = format!("ssh://{}{path}", alias.to_ascii_lowercase());
    let key = format!("{:x}", Sha256::digest(resource.as_bytes()));
    let dir = profile.join("History").join("ssh").join(&key[..24]);
    Ok((dir, resource))
}

pub(crate) fn remote_diff(
    profile: &Path,
    alias: &str,
    path: &str,
    id: &str,
    current: &[u8],
) -> Result<String, String> {
    if current.len() > crate::files::MAX_TEXT || current.contains(&0) {
        return Err("Remote file is not editable text.".into());
    }
    let current = std::str::from_utf8(current).map_err(|_| "Remote file is not UTF-8 text.")?;
    let (dir, resource) = remote_resource(profile, alias, path).map_err(|e| e.to_string())?;
    let old = version_text(&dir, Path::new(&resource), id)?;
    Ok(text_diff(
        path.rsplit('/').next().unwrap_or(path),
        &old,
        current,
    ))
}

fn append(dir: &Path, file: &Path, text: &str) -> io::Result<bool> {
    use std::io::Write;
    if text.len() > MAX_BYTES {
        return Ok(false);
    }
    fs::create_dir_all(dir)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".lock"))?;
    fs2::FileExt::lock_exclusive(&lock)?;
    let mut index = index(dir, file)?;
    let resource_key = format!(
        "{:x}",
        Sha256::digest(
            file.to_str()
                .ok_or_else(|| io::Error::other("Invalid resource"))?
                .as_bytes()
        )
    );
    if let Some(last) = index.entries.last()
        && last.resource_key.as_ref() == Some(&resource_key)
        && fs::read(dir.join(&last.id)).is_ok_and(|s| s == text.as_bytes())
    {
        return Ok(false);
    }
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(|_| io::Error::other("Cannot allocate history entry"))?;
    let id = format!("{:016x}", u64::from_le_bytes(random));
    let path = dir.join(&id);
    let mut version = tempfile::NamedTempFile::new_in(dir)?;
    version.write_all(text.as_bytes())?;
    version.as_file().sync_all()?;
    version.persist_noclobber(&path)?;
    index.entries.push(Entry {
        id,
        timestamp: chrono::Utc::now().timestamp_millis(),
        source: "save".into(),
        resource_key: Some(resource_key),
    });
    let obsolete = (index.entries.len() > LIMIT).then(|| index.entries.remove(0));
    if let Err(e) =
        crate::workspace::atomic_write(&dir.join("entries.json"), &serde_json::to_vec(&index)?)
    {
        let _ = fs::remove_file(path);
        return Err(io::Error::other(e));
    }
    if let Some(old) = obsolete {
        fs::remove_file(dir.join(old.id))?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_saved_versions_only_return_entries_for_the_exact_resource() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let upper = root.join("Sample.txt");
        let lower = root.join("sample.txt");
        record(&root, &upper, "first").unwrap();
        record(&root, &lower, "second").unwrap();
        let entries = shared_read(&root, &upper, None).unwrap();
        let rows = entries["entries"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].as_object().unwrap().len(), 3);
        let id = rows[0]["id"].as_str().unwrap();
        assert_eq!(
            shared_read(&root, &upper, Some(id)).unwrap()["text"],
            "first"
        );
        assert!(shared_read(&root, &lower, Some(id)).is_err());
        assert!(shared_read(&root, &upper, Some("../entries.json")).is_err());
    }
    #[test]
    fn local_file_history_failed_save_preserves_versions_and_original_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("doc.txt");
        let profile = dir.path().join("profile");
        fs::write(&file, "one").unwrap();
        let mut doc = crate::files::Document::open(dir.path(), &file).unwrap();
        doc.text = "two".into();
        doc.save(Some(&profile)).unwrap();
        let versions = list(&profile, &doc.path).unwrap();
        assert_eq!(versions.len(), 1);
        fs::write(&file, "external").unwrap();
        doc.text = "three".into();
        assert!(doc.save(Some(&profile)).is_err());
        assert_eq!(list(&profile, &doc.path).unwrap().len(), 1);
        let patch = diff(&profile, dir.path(), &doc.path, &versions[0].id).unwrap();
        assert!(patch.contains("-two") && patch.contains("+external"));
        assert!(diff(&profile, dir.path(), &doc.path, "../outside").is_err());
    }
    #[test]
    fn local_file_history_retains_fifty_distinct_bounded_versions() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("file.txt");
        let profile = temp.path().join("private");
        record(&profile, &file, &"x".repeat(MAX_BYTES + 1)).unwrap();
        assert!(!profile.exists());
        for i in 0..52 {
            record(&profile, &file, &i.to_string()).unwrap();
        }
        record(&profile, &file, "51").unwrap();
        let dir = folder(&profile, &file).unwrap();
        let items = index(&dir, &file).unwrap().entries;
        assert_eq!(items.len(), LIMIT);
        assert_eq!(fs::read_to_string(dir.join(&items[0].id)).unwrap(), "2");
        assert_eq!(fs::read_to_string(dir.join(&items[49].id)).unwrap(), "51");
        assert_eq!(fs::read_dir(dir).unwrap().count(), LIMIT + 2);
    }
}
