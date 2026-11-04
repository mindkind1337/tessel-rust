//! Bounded, local Codex terminal discovery. Only the first metadata line is parsed.
//! Call on a worker, never on the UI thread. No transcript or credential is returned.
use chrono::{DateTime, Datelike, Duration, Local, TimeZone};
use std::{
    collections::HashSet,
    fs,
    io::{self, BufRead, BufReader, Read},
    path::Path,
};

const MAX_LINE: u64 = 64 * 1024;
const MAX_ENTRIES: usize = 4096;
const MAX_BYTES: u64 = 32 * 1024 * 1024;
const SLACK_MS: i64 = 5000;

#[derive(Clone, PartialEq, Eq)]
pub struct Metadata {
    pub id: String,
    pub cwd: String,
    pub time_ms: i64,
}

/// Deliberately excludes arbitrary payload fields (including prompts).
pub fn parse_metadata(line: &[u8]) -> Option<Metadata> {
    if line.len() > MAX_LINE as usize {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(line).ok()?;
    if value.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let payload = value.get("payload")?;
    let id = payload.get("id")?.as_str()?;
    if !crate::claude::valid_uuid(id) {
        return None;
    }
    let cwd = payload.get("cwd")?.as_str()?;
    if cwd.is_empty() || cwd.len() > 32768 || cwd.contains('\0') {
        return None;
    }
    let timestamp = payload
        .get("timestamp")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| value.get("timestamp")?.as_str())?;
    let time_ms = DateTime::parse_from_rfc3339(timestamp)
        .ok()?
        .timestamp_millis();
    Some(Metadata {
        id: id.into(),
        cwd: cwd.into(),
        time_ms,
    })
}

fn normalized(path: &str) -> String {
    if cfg!(windows) {
        path.replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    } else {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            "/".into()
        } else {
            trimmed.into()
        }
    }
}

pub struct Query<'a> {
    pub cwd: &'a Path,
    pub since_ms: i64,
    pub now_ms: i64,
    pub exclude: &'a HashSet<String>,
    pub latest: bool,
    pub active_since_ms: Option<i64>,
}

/// Verifies a saved ID even outside discovery's thirty-day window. A filename
/// alone is not evidence: its first metadata record must match both ID and cwd.
pub fn verify(root: &Path, cwd: &Path, id: &str) -> Result<bool, String> {
    if !crate::claude::valid_uuid(id) || !cwd.is_absolute() {
        return Err("Invalid saved Codex conversation.".into());
    }
    let want = normalized(cwd.to_str().ok_or("Invalid conversation folder.")?);
    let suffix = format!("-{id}.jsonl");
    let mut pending = vec![
        (root.join("sessions"), 3),
        (root.join("archived_sessions"), 0),
    ];
    let mut count = 0;
    while let Some((dir, depth)) = pending.pop() {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return Err("Cannot verify Codex conversation folders.".into()),
        };
        for entry in entries {
            count += 1;
            if count > MAX_ENTRIES {
                return Err("Codex verification entry limit reached.".into());
            }
            let entry = entry.map_err(|_| "Cannot enumerate saved Codex conversations.")?;
            let kind = entry
                .file_type()
                .map_err(|_| "Cannot inspect saved Codex metadata.")?;
            if kind.is_dir() && depth > 0 {
                pending.push((entry.path(), depth - 1));
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !kind.is_file() || !name.starts_with("rollout-") || !name.ends_with(&suffix) {
                continue;
            }
            let file = match fs::File::open(entry.path()) {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => return Err("Cannot read saved Codex metadata.".into()),
            };
            let mut line = Vec::new();
            BufReader::new(file.take(MAX_LINE + 1))
                .read_until(b'\n', &mut line)
                .map_err(|_| "Cannot read saved Codex metadata.")?;
            if parse_metadata(&line)
                .is_some_and(|meta| meta.id == id && normalized(&meta.cwd) == want)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Matches source's thirty-day window, five-second launch slack, and excluded IDs.
/// An unreadable directory or exhausted budget is an error, not a missing session.
/// Missing/invalid timestamps are not trusted as evidence of ownership.
pub fn find(root: &Path, query: Query<'_>) -> Result<Option<String>, String> {
    let cwd = query
        .cwd
        .to_str()
        .filter(|s| !s.is_empty())
        .ok_or("Invalid conversation folder.")?;
    if !query.cwd.is_absolute() || query.since_ms > query.now_ms {
        return Err("Invalid conversation discovery interval.".into());
    }
    let since = query
        .since_ms
        .max(query.now_ms.saturating_sub(30 * 86_400_000));
    let local_day = |ms| {
        Local
            .timestamp_millis_opt(ms)
            .single()
            .map(|d| d.date_naive())
    };
    let mut day = local_day(since.saturating_sub(86_400_000)).ok_or("Invalid discovery date.")?;
    let end = local_day(query.now_ms).ok_or("Invalid discovery date.")?;
    let want = normalized(cwd);
    let mut best: Option<Metadata> = None;
    let mut entries_seen = 0;
    let mut bytes_read = 0;
    while day <= end {
        let dir = root
            .join("sessions")
            .join(format!("{:04}", day.year()))
            .join(format!("{:02}", day.month()))
            .join(format!("{:02}", day.day()));
        let entries = match fs::read_dir(dir) {
            Ok(entries) => Some(entries),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(_) => return Err("Cannot inspect Codex conversation folders.".into()),
        };
        if let Some(entries) = entries {
            for entry in entries {
                entries_seen += 1;
                if entries_seen > MAX_ENTRIES {
                    return Err("Codex discovery entry limit reached.".into());
                }
                let entry = entry.map_err(|_| "Cannot enumerate Codex conversations.")?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                    continue;
                }
                if !entry
                    .file_type()
                    .map_err(|_| "Cannot inspect Codex metadata.")?
                    .is_file()
                {
                    continue;
                }
                let file = match fs::File::open(entry.path()) {
                    Ok(file) => file,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                    Err(_) => return Err("Cannot read Codex conversation metadata.".into()),
                };
                let stat = file
                    .metadata()
                    .map_err(|_| "Cannot inspect Codex metadata.")?;
                if !stat.is_file() {
                    continue;
                }
                let modified: DateTime<chrono::Utc> = stat
                    .modified()
                    .map_err(|_| "Cannot inspect Codex modification time.")?
                    .into();
                let modified = modified.timestamp_millis();
                if modified < since.saturating_sub(SLACK_MS)
                    || query
                        .active_since_ms
                        .is_some_and(|active| modified < active)
                {
                    continue;
                }
                let mut line = Vec::new();
                let allowance = (MAX_LINE + 1).min(MAX_BYTES - bytes_read);
                if allowance == 0 {
                    return Err("Codex discovery byte limit reached.".into());
                }
                let mut reader = BufReader::new(file.take(allowance));
                reader
                    .read_until(b'\n', &mut line)
                    .map_err(|_| "Cannot read Codex conversation metadata.")?;
                // BufReader can fetch beyond the newline. Account for all bytes
                // read from disk, including that bounded read-ahead.
                bytes_read += allowance - reader.get_ref().limit();
                if allowance < MAX_LINE + 1 && line.len() as u64 == allowance {
                    return Err("Codex discovery byte limit reached.".into());
                }
                let Some(meta) = parse_metadata(&line) else {
                    continue;
                };
                if query.exclude.contains(&meta.id)
                    || normalized(&meta.cwd) != want
                    || meta.time_ms < since.saturating_sub(SLACK_MS)
                    || meta.time_ms > query.now_ms.saturating_add(SLACK_MS)
                {
                    continue;
                }
                let better = best.as_ref().is_none_or(|before| {
                    if meta.time_ms == before.time_ms {
                        meta.id < before.id
                    } else if query.latest {
                        meta.time_ms > before.time_ms
                    } else {
                        meta.time_ms < before.time_ms
                    }
                });
                if better {
                    best = Some(meta);
                }
            }
        }
        day = day
            .checked_add_signed(Duration::days(1))
            .ok_or("Invalid discovery date.")?;
    }
    Ok(best.map(|meta| meta.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIRST: &str = "12345678-1234-4234-8234-123456789abc";
    const SECOND: &str = "22345678-1234-4234-8234-123456789abc";
    #[test]
    fn verifies_old_and_archived_ids_without_trusting_names_or_other_projects() {
        let temp = tempfile::tempdir().unwrap();
        for relative in ["sessions/2020/01/02", "archived_sessions"] {
            let dir = temp.path().join(relative);
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join(format!("rollout-2020-01-02-{FIRST}.jsonl"));
            fs::write(&path, line(SECOND, temp.path(), 1_577_923_200_000)).unwrap();
            assert!(!verify(temp.path(), temp.path(), FIRST).unwrap());
            fs::write(&path, line(FIRST, temp.path(), 1_577_923_200_000)).unwrap();
            assert!(verify(temp.path(), temp.path(), FIRST).unwrap());
            assert!(!verify(temp.path(), &temp.path().join("different"), FIRST).unwrap());
            fs::remove_file(path).unwrap();
        }
        assert!(verify(temp.path(), temp.path(), "../bad").is_err());
        assert!(!verify(temp.path(), temp.path(), FIRST).unwrap());
    }
    #[test]
    fn directory_failures_and_lookup_limits_are_not_reported_as_absence() {
        let temp = tempfile::tempdir().unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let date = Local.timestamp_millis_opt(now).unwrap();
        let dir = temp.path().join(format!(
            "sessions/{:04}/{:02}/{:02}",
            date.year(),
            date.month(),
            date.day()
        ));
        fs::create_dir_all(dir.parent().unwrap()).unwrap();
        fs::write(&dir, b"not a directory").unwrap();
        let excluded = HashSet::new();
        let run = || {
            find(
                temp.path(),
                Query {
                    cwd: temp.path(),
                    since_ms: now,
                    now_ms: now,
                    exclude: &excluded,
                    latest: false,
                    active_since_ms: None,
                },
            )
        };
        assert!(run().is_err());
        fs::remove_file(&dir).unwrap();
        fs::create_dir(&dir).unwrap();
        for n in 0..=MAX_ENTRIES {
            fs::write(dir.join(format!("ignored-{n}")), []).unwrap();
        }
        assert_eq!(
            run().err().as_deref(),
            Some("Codex discovery entry limit reached.")
        );
    }
    fn line(id: &str, cwd: &Path, time: i64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"type":"session_meta", "payload": {
            "id":id, "cwd":cwd, "timestamp":DateTime::from_timestamp_millis(time).unwrap().to_rfc3339()
        }})).unwrap()
    }
    #[test]
    fn rejects_untrusted_metadata_and_never_parses_later_messages() {
        let cwd = std::env::temp_dir();
        let data = line(FIRST, &cwd, 1_700_000_000_000);
        assert_eq!(parse_metadata(&data).unwrap().id, FIRST);
        assert!(parse_metadata(&line("../../bad", &cwd, 1_700_000_000_000)).is_none());
        assert!(parse_metadata(&vec![b' '; MAX_LINE as usize + 1]).is_none());
        assert!(parse_metadata(br#"{"type":"session_meta","payload":{"id":"12345678-1234-4234-8234-123456789abc","cwd":"C:/fake"}}"#).is_none());
        let mut multiple = data;
        multiple.extend_from_slice(b"\n{\"type\":\"message\"}");
        assert!(parse_metadata(&multiple).is_none());
    }
    #[test]
    fn discovers_by_folder_time_and_unclaimed_id_without_reading_transcript() {
        let temp = tempfile::tempdir().unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let date = Local.timestamp_millis_opt(now).unwrap();
        let dir = temp.path().join(format!(
            "sessions/{:04}/{:02}/{:02}",
            date.year(),
            date.month(),
            date.day()
        ));
        fs::create_dir_all(&dir).unwrap();
        for (id, time) in [(FIRST, now - 1000), (SECOND, now)] {
            let mut data = line(id, temp.path(), time);
            data.extend_from_slice(b"\nprivate transcript that is intentionally invalid JSON");
            fs::write(dir.join(format!("rollout-{id}.jsonl")), data).unwrap();
        }
        let mut excluded = HashSet::new();
        let run = |exclude: &HashSet<String>, latest, cwd: &Path, active| {
            find(
                temp.path(),
                Query {
                    cwd,
                    since_ms: now - 2000,
                    now_ms: now,
                    exclude,
                    latest,
                    active_since_ms: active,
                },
            )
            .unwrap()
        };
        assert_eq!(
            run(&excluded, false, temp.path(), None).as_deref(),
            Some(FIRST)
        );
        assert_eq!(
            run(&excluded, true, temp.path(), None).as_deref(),
            Some(SECOND)
        );
        excluded.insert(FIRST.into());
        assert_eq!(
            run(&excluded, false, temp.path(), None).as_deref(),
            Some(SECOND)
        );
        assert!(run(&excluded, false, &temp.path().join("other"), None).is_none());
        assert!(run(&excluded, false, temp.path(), Some(now + 60_000)).is_none());
        excluded.insert(SECOND.into());
        assert!(run(&excluded, false, temp.path(), None).is_none());
    }
}
