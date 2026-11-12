//! Validate a history-owned transcript before asking the OS to recycle it.
use crate::side_panels_model::{SessionHistoryEntry, SessionSource};
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::{Component, Path, PathBuf},
};

pub fn same_identity(a: &SessionHistoryEntry, b: &SessionHistoryEntry) -> bool {
    a.agent_id == b.agent_id
        && a.id == b.id
        && a.profile_binding == b.profile_binding
        && a.file_path == b.file_path
        && a.folder == b.folder
}

/// No filesystem access; only an entry already discovered in a known local profile qualifies.
pub fn source_for(
    entry: &SessionHistoryEntry,
    known: &[SessionHistoryEntry],
    sources: &[SessionSource],
) -> Result<SessionSource, String> {
    if entry.open_pane_id.is_some()
        || !crate::claude::valid_uuid(&entry.id)
        || !known
            .iter()
            .any(|cached| cached.open_pane_id.is_none() && same_identity(entry, cached))
    {
        return Err("This conversation is no longer available; refresh the history.".into());
    }
    sources
        .iter()
        .find(|source| {
            source.agent_id == entry.agent_id
                && Some(&source.binding) == entry.profile_binding.as_ref()
        })
        .cloned()
        .ok_or_else(|| "The original conversation profile is unavailable.".into())
}

fn plain_metadata(path: &Path) -> Result<fs::Metadata, String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "The saved transcript is unavailable.")?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("Linked transcript paths cannot be recycled.".into());
        }
    }
    if meta.file_type().is_symlink() {
        return Err("Linked transcript paths cannot be recycled.".into());
    }
    Ok(meta)
}

/// Worker-only: check every path component and provider identity, with bounded metadata reading.
pub fn validate(
    source: &SessionSource,
    entry: &SessionHistoryEntry,
) -> Result<(PathBuf, PathBuf), String> {
    if source.agent_id != entry.agent_id
        || Some(&source.binding) != entry.profile_binding.as_ref()
        || !crate::claude::valid_uuid(&entry.id)
        || entry.open_pane_id.is_some()
    {
        return Err("Invalid conversation identity.".into());
    }
    let path = entry
        .file_path
        .as_deref()
        .ok_or("This conversation has no local transcript.")?;
    let rel = path
        .strip_prefix(&source.root)
        .map_err(|_| "Transcript is outside its profile.")?;
    let parts: Vec<_> = rel.components().collect();
    if !source.root.is_absolute() || parts.iter().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("Invalid transcript path.".into());
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid transcript name.")?;
    let first = parts.first().map(|p| p.as_os_str());
    let shape = match source.agent_id.as_str() {
        "claude" => {
            parts.len() == 3
                && first == Some(std::ffi::OsStr::new("projects"))
                && name == format!("{}.jsonl", entry.id)
        }
        "codex" => {
            ((parts.len() == 2 && first == Some(std::ffi::OsStr::new("archived_sessions")))
                || ((2..=5).contains(&parts.len())
                    && first == Some(std::ffi::OsStr::new("sessions"))))
                && name.starts_with("rollout-")
                && name.ends_with(&format!("-{}.jsonl", entry.id))
        }
        _ => false,
    };
    if !shape {
        return Err("This file is not a recognized conversation transcript.".into());
    }
    if !plain_metadata(&source.root)?.is_dir() {
        return Err("Profile folder is unavailable.".into());
    }
    let root =
        fs::canonicalize(&source.root).map_err(|_| "Cannot resolve the conversation profile.")?;
    let mut current = source.root.clone();
    for (idx, part) in parts.iter().enumerate() {
        current.push(part.as_os_str());
        let meta = plain_metadata(&current)?;
        if (idx + 1 == parts.len() && !meta.is_file()) || (idx + 1 < parts.len() && !meta.is_dir())
        {
            return Err("Transcript path changed; refresh the history.".into());
        }
    }
    let canonical = fs::canonicalize(path).map_err(|_| "Cannot resolve the transcript.")?;
    if !canonical.starts_with(&root) || canonical == root {
        return Err("Transcript is outside its profile.".into());
    }
    if source.agent_id == "codex" {
        let file =
            fs::File::open(&canonical).map_err(|_| "Cannot inspect conversation metadata.")?;
        let mut line = Vec::new();
        BufReader::new(file.take(65537))
            .read_until(b'\n', &mut line)
            .map_err(|_| "Cannot read conversation metadata.")?;
        let meta =
            crate::codex_sessions::parse_metadata(&line).ok_or("Invalid conversation metadata.")?;
        if meta.id != entry.id
            || crate::session_profile::path_key(Path::new(&meta.cwd))
                != crate::session_profile::path_key(&entry.folder)
        {
            return Err("Conversation identity changed; refresh the history.".into());
        }
    }
    Ok((root, canonical))
}

pub fn recycle(source: &SessionSource, entry: &SessionHistoryEntry) -> Result<(), String> {
    let (root, path) = validate(source, entry)?;
    crate::file_actions::move_to_trash(&root, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path, agent: &str) -> (SessionSource, SessionHistoryEntry) {
        let binding = crate::session_profile::Binding::Default;
        let id = "12345678-1234-4234-8234-123456789abc".to_owned();
        let path = if agent == "claude" {
            root.join("projects/synthetic").join(format!("{id}.jsonl"))
        } else {
            root.join("archived_sessions")
                .join(format!("rollout-test-{id}.jsonl"))
        };
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, serde_json::to_vec(&serde_json::json!({"type":"session_meta","payload":{"id":id,"cwd":root,"timestamp":"2026-01-01T00:00:00Z"}})).unwrap()).unwrap();
        (
            SessionSource {
                agent_id: agent.into(),
                root: root.into(),
                binding: binding.clone(),
                label: String::new(),
            },
            SessionHistoryEntry {
                agent_id: agent.into(),
                id,
                folder: root.into(),
                profile_binding: Some(binding),
                file_path: Some(path),
                open_pane_id: None,
                title: "Synthetic".into(),
                updated_at_ms: 0,
                profile_label: String::new(),
                transcript_snippet: None,
            },
        )
    }
    #[test]
    fn only_known_closed_transcripts_inside_their_profile_qualify() {
        let temp = tempfile::tempdir().unwrap();
        let (source, entry) = fixture(temp.path(), "claude");
        assert!(source_for(&entry, &[], std::slice::from_ref(&source)).is_err());
        assert!(
            source_for(
                &entry,
                std::slice::from_ref(&entry),
                std::slice::from_ref(&source)
            )
            .is_ok()
        );
        assert!(validate(&source, &entry).is_ok());
        let mut altered = entry.clone();
        altered.open_pane_id = Some(1);
        assert!(validate(&source, &altered).is_err());
        altered = entry.clone();
        altered.file_path = Some(temp.path().join("outside.jsonl"));
        assert!(validate(&source, &altered).is_err());
        altered.file_path = Some(
            temp.path()
                .join("projects/synthetic/../synthetic")
                .join(format!("{}.jsonl", entry.id)),
        );
        assert!(validate(&source, &altered).is_err());
        altered = entry.clone();
        altered.profile_binding = Some(crate::session_profile::Binding::Managed {
            account_id: "other".into(),
        });
        assert!(validate(&source, &altered).is_err());
        assert!(entry.file_path.unwrap().exists());
    }
    #[test]
    fn codex_metadata_must_match_id_and_original_folder() {
        let temp = tempfile::tempdir().unwrap();
        let (source, mut entry) = fixture(temp.path(), "codex");
        assert!(validate(&source, &entry).is_ok());
        entry.folder = temp.path().join("other");
        assert!(validate(&source, &entry).is_err());
        entry.folder = temp.path().into();
        fs::write(entry.file_path.as_ref().unwrap(), vec![b' '; 65538]).unwrap();
        assert!(validate(&source, &entry).is_err());
    }
    #[test]
    #[cfg(windows)]
    fn junction_cannot_redirect_a_transcript_outside_its_profile() {
        let temp = tempfile::tempdir().unwrap();
        let (source, entry) = fixture(&temp.path().join("profile"), "claude");
        let path = entry.file_path.as_ref().unwrap();
        let original = fs::read(path).unwrap();
        let folder = path.parent().unwrap();
        fs::remove_file(path).unwrap();
        fs::remove_dir(folder).unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let target = outside.join(path.file_name().unwrap());
        fs::write(&target, &original).unwrap();
        let status = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(
                dunce::simplified(folder)
                    .to_string_lossy()
                    .replace('/', "\\"),
            )
            .arg(
                dunce::simplified(&outside)
                    .to_string_lossy()
                    .replace('/', "\\"),
            )
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "cannot create synthetic junction: {} {}",
            String::from_utf8_lossy(&status.stdout),
            String::from_utf8_lossy(&status.stderr)
        );
        let result = validate(&source, &entry);
        fs::remove_dir(folder).unwrap();
        assert!(result.unwrap_err().contains("Linked transcript"));
        assert_eq!(fs::read(target).unwrap(), original);
    }

    #[test]
    #[cfg(windows)]
    fn synthetic_transcript_uses_native_recycle_operation() {
        let temp = tempfile::tempdir().unwrap();
        let (source, entry) = fixture(temp.path(), "claude");
        let path = entry.file_path.clone().unwrap();
        std::thread::spawn(move || {
            use windows::Win32::System::Com::{
                COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize,
            };
            unsafe {
                CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap();
            }
            let result = recycle(&source, &entry);
            unsafe {
                CoUninitialize();
            }
            result
        })
        .join()
        .unwrap()
        .unwrap();
        assert!(!path.exists());
        assert!(temp.path().is_dir());
    }
}
