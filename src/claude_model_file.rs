//! Worker-only Claude/OpenClaude transcript lookup in the captured launch profile.
//! Returns an open file, never transcript text; callers retain this exact handle.
use std::{
    fs::{self, File},
    io::{self, BufRead, BufReader, Read},
    path::Path,
};

const MAX_ENTRIES: usize = 4096;
const MAX_RECORD: u64 = 256 * 1024;
const MAX_BYTES: u64 = 8 * 1024 * 1024;

fn normalized(path: &str) -> String {
    if cfg!(windows) {
        path.replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    } else if path == "/" {
        path.into()
    } else {
        path.trim_end_matches('/').into()
    }
}

/// Locate the requested UUID beneath `projects`, then verify a top-level
/// conversation record's sessionId and cwd before exposing the file to a reader.
/// File-history snapshots without identity are skipped. Invalid input, exhausted
/// budgets and IO errors are distinct from an absent or mismatched conversation.
pub fn open_verified(root: &Path, cwd: &Path, id: &str) -> Result<Option<File>, String> {
    open_bounded(root, cwd, id, MAX_ENTRIES, MAX_BYTES)
}

fn open_bounded(
    root: &Path,
    cwd: &Path,
    id: &str,
    entry_limit: usize,
    byte_limit: u64,
) -> Result<Option<File>, String> {
    if !root.is_absolute() || !cwd.is_absolute() || !crate::claude::valid_uuid(id) {
        return Err("Invalid Claude model conversation scope.".into());
    }
    let cwd = normalized(cwd.to_str().ok_or("Invalid conversation folder.")?);
    let entries = match fs::read_dir(root.join("projects")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cannot inspect Claude model conversation folders.".into()),
    };
    let mut remaining = byte_limit;
    for (count, entry) in entries.enumerate() {
        if count >= entry_limit {
            return Err("Claude model conversation entry limit reached.".into());
        }
        let entry = entry.map_err(|_| "Cannot enumerate Claude model conversations.")?;
        if !entry
            .file_type()
            .map_err(|_| "Cannot inspect Claude model folder.")?
            .is_dir()
        {
            continue;
        }
        let path = entry.path().join(format!("{id}.jsonl"));
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => (),
            Ok(_) => continue,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return Err("Cannot inspect Claude model conversation.".into()),
        }
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return Err("Cannot open Claude model conversation.".into()),
        };
        if !file
            .metadata()
            .map_err(|_| "Cannot inspect open Claude conversation.")?
            .is_file()
        {
            continue;
        }
        if remaining == 0 {
            return Err("Claude model conversation byte limit reached.".into());
        }
        let mut bounded = (&mut file).take(remaining);
        let verified = verify_records(&mut BufReader::new(&mut bounded), &cwd, id)?;
        remaining = bounded.limit();
        if verified {
            return Ok(Some(file));
        }
        if remaining == 0 {
            return Err("Claude model conversation byte limit reached.".into());
        }
    }
    Ok(None)
}

fn verify_records(reader: &mut impl BufRead, cwd: &str, id: &str) -> Result<bool, String> {
    loop {
        let mut line = Vec::new();
        (&mut *reader)
            .take(MAX_RECORD + 1)
            .read_until(b'\n', &mut line)
            .map_err(|_| "Cannot read Claude conversation identity.")?;
        if line.len() as u64 > MAX_RECORD {
            return Err("Claude conversation identity record limit reached.".into());
        }
        if line.is_empty() {
            return Ok(false);
        }
        // Writers may still be appending the final record. A budget boundary
        // must not turn a truncated JSON prefix into proof of identity either.
        if !line.ends_with(b"\n") {
            return Ok(false);
        }
        let Ok(record) = serde_json::from_slice::<serde_json::Value>(&line) else {
            continue;
        };
        if !matches!(
            record.get("type").and_then(|v| v.as_str()),
            Some("user" | "assistant")
        ) || record.get("isSidechain") == Some(&serde_json::Value::Bool(true))
        {
            continue;
        }
        let (Some(found_id), Some(found_cwd)) = (
            record.get("sessionId").and_then(|v| v.as_str()),
            record.get("cwd").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        return Ok(found_id == id && normalized(found_cwd) == cwd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom};
    const ID: &str = "12345678-1234-4234-8234-123456789abc";

    fn fixture(root: &Path, cwd: &Path, id: &str) -> std::path::PathBuf {
        let folder = root.join("projects").join("synthetic-project");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join(format!("{ID}.jsonl"));
        let record = serde_json::json!({"type":"user", "sessionId":id, "cwd":cwd});
        fs::write(
            &path,
            format!("{{\"type\":\"file-history-snapshot\"}}\n{record}\n"),
        )
        .unwrap();
        path
    }

    #[test]
    fn verifies_identity_after_snapshots_and_keeps_the_same_handle() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("profile");
        let path = fixture(&root, dir.path(), ID);
        let mut file = open_verified(&root, dir.path(), ID).unwrap().unwrap();
        fs::rename(&path, path.with_extension("old")).unwrap();
        fs::write(&path, "replacement must not be read").unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        assert!(text.contains(ID));
        assert!(!text.contains("replacement"));
    }

    #[test]
    fn rejects_foreign_ids_folders_profiles_and_invalid_scopes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("profile");
        fixture(&root, dir.path(), "22345678-1234-4234-8234-123456789abc");
        assert!(open_verified(&root, dir.path(), ID).unwrap().is_none());
        fixture(&root, &dir.path().join("other-project"), ID);
        assert!(open_verified(&root, dir.path(), ID).unwrap().is_none());
        fixture(&root, dir.path(), ID);
        assert!(
            open_verified(&dir.path().join("other-profile"), dir.path(), ID)
                .unwrap()
                .is_none()
        );
        assert!(open_verified(&root, dir.path(), "../../foreign").is_err());
        assert!(open_verified(Path::new("relative"), dir.path(), ID).is_err());
    }

    #[test]
    fn reports_exhausted_budgets_instead_of_absence() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("profile");
        let path = fixture(&root, dir.path(), ID);
        assert!(open_bounded(&root, dir.path(), ID, 0, MAX_BYTES).is_err());
        assert!(open_bounded(&root, dir.path(), ID, MAX_ENTRIES, 5).is_err());
        fs::write(path, vec![b'x'; MAX_RECORD as usize + 1]).unwrap();
        assert!(open_verified(&root, dir.path(), ID).is_err());
    }

    #[test]
    fn sidechains_and_arbitrary_payloads_cannot_claim_the_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("profile");
        let path = fixture(&root, dir.path(), ID);
        let sidechain = serde_json::json!({
            "type":"assistant", "sessionId":ID, "cwd":dir.path(),
            "isSidechain":true, "message":{"model":"not-the-main-conversation"}
        });
        let tool = serde_json::json!({"type":"tool_result", "sessionId":ID, "cwd":dir.path()});
        fs::write(&path, format!("not json\n{sidechain}\n{tool}\n")).unwrap();
        assert!(open_verified(&root, dir.path(), ID).unwrap().is_none());
        let wrong =
            serde_json::json!({"type":"user", "sessionId":ID, "cwd":dir.path().join("other")});
        let right = serde_json::json!({"type":"user", "sessionId":ID, "cwd":dir.path()});
        fs::write(&path, right.to_string()).unwrap();
        assert!(open_verified(&root, dir.path(), ID).unwrap().is_none());
        fs::write(path, format!("{wrong}\n{right}\n")).unwrap();
        assert!(open_verified(&root, dir.path(), ID).unwrap().is_none());
    }
}
