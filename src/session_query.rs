//! Single-flight, cancellable transcript filtering for the native sessions panel.
use crate::side_panels_model::{
    SessionHistoryEntry, SessionHistoryScope, filter_sessions, mask_transcript_secrets,
};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    pub entries: Vec<SessionHistoryEntry>,
    pub scope: SessionHistoryScope,
    pub active: Option<PathBuf>,
    pub roots: Vec<PathBuf>,
    pub agent: Option<String>,
    pub query: String,
    pub transcripts: bool,
}

#[derive(Default)]
pub(crate) struct ResultSet {
    pub entries: Vec<SessionHistoryEntry>,
    pub limited: bool,
}

struct Job {
    generation: u64,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<ResultSet>,
}

#[derive(Default)]
pub(crate) struct Search {
    key: Option<Key>,
    generation: u64,
    due: Option<Instant>,
    job: Option<Job>,
    result: ResultSet,
}

impl Drop for Search {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Search {
    fn cancel(&self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn invalidate(&mut self) {
        self.cancel();
        self.generation = self.generation.wrapping_add(1);
        self.key = None;
        self.due = None;
        self.result = ResultSet::default();
        // Keep the receiver until the old worker exits: never spawn concurrent scans.
    }

    pub(crate) fn pause(&mut self) {
        if self.pending() {
            self.invalidate();
        }
        // A completed result stays cached while its panel is hidden.
    }

    pub(crate) fn limited(&self) -> bool {
        self.result.limited
    }

    pub(crate) fn pending(&self) -> bool {
        self.key.is_some()
            && (self.due.is_some()
                || self
                    .job
                    .as_ref()
                    .is_some_and(|j| j.generation == self.generation))
    }

    pub(crate) fn update(&mut self, key: Key, now: Instant) -> &ResultSet {
        if self.key.as_ref() != Some(&key) {
            self.invalidate();
            self.result.entries = metadata_matches(&key, &key.query);
            if key.transcripts && !key.query.trim().is_empty() {
                self.due = Some(now + Duration::from_millis(180));
            }
            self.key = Some(key);
        }
        if let Some(job) = &self.job {
            match job.result.try_recv() {
                Ok(result) => {
                    if job.generation == self.generation {
                        self.result = result;
                    }
                    self.job = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    if job.generation == self.generation {
                        self.result.limited = true;
                    }
                    self.job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.job.is_none() && self.due.is_some_and(|due| now >= due) {
            self.due = None;
            let key = self.key.as_ref().expect("scheduled search").clone();
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let (tx, rx) = mpsc::sync_channel(1);
            let spawned = std::thread::Builder::new()
                .name("session-query".into())
                .spawn(move || {
                    let _ = tx.send(scan(&key, &flag, Limits::default()));
                });
            if spawned.is_ok() {
                self.job = Some(Job {
                    generation: self.generation,
                    cancel,
                    result: rx,
                });
            } else {
                self.result.limited = true;
            }
        }
        &self.result
    }
}

fn metadata_matches(key: &Key, query: &str) -> Vec<SessionHistoryEntry> {
    filter_sessions(
        &key.entries,
        key.scope,
        key.active.as_deref(),
        &key.roots,
        key.agent.as_deref(),
        query,
        false,
    )
}

#[derive(Clone, Copy)]
struct Limits {
    files: usize,
    bytes: usize,
    per_file: usize,
    time: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            files: 256,
            bytes: 16 * 1024 * 1024,
            per_file: 256 * 1024,
            time: Duration::from_secs(3),
        }
    }
}

fn scan(key: &Key, cancel: &AtomicBool, limits: Limits) -> ResultSet {
    let start = Instant::now();
    let query = key.query.trim().to_ascii_lowercase();
    let scoped = metadata_matches(key, "");
    let metadata = metadata_matches(key, &query);
    let mut result = ResultSet::default();
    let mut files = 0;
    let mut bytes = 0;
    for mut entry in scoped {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if metadata.iter().any(|e| e == &entry) {
            result.entries.push(entry);
            continue;
        }
        let Some(path) = &entry.file_path else {
            continue;
        };
        if files >= limits.files || bytes >= limits.bytes || start.elapsed() >= limits.time {
            result.limited = true;
            continue; // Preserve metadata hits even after the I/O budget is exhausted.
        }
        files += 1;
        let Ok(mut file) = File::open(path) else {
            result.limited = true;
            continue;
        };
        let metadata = match file.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => {
                result.limited = true;
                continue;
            }
        };
        let cap = limits.per_file.min(limits.bytes - bytes);
        // Recent messages live at the end of an append-only transcript.
        let offset = metadata.len().saturating_sub(cap as u64);
        if offset > 0 {
            result.limited = true;
            if file.seek(SeekFrom::Start(offset)).is_err() {
                continue;
            }
        }
        let mut body = Vec::new();
        let mut reader = file.take(cap as u64 + 1);
        let mut chunk = [0u8; 8192];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return result;
            }
            if start.elapsed() >= limits.time {
                result.limited = true;
                break;
            }
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => body.extend_from_slice(&chunk[..n]),
                Err(_) => {
                    result.limited = true;
                    break;
                }
            }
        }
        bytes = bytes.saturating_add(body.len());
        if body.len() > cap {
            result.limited = true;
            body.truncate(cap);
        }
        let body = if offset > 0 {
            body.iter()
                .position(|byte| *byte == b'\n')
                .map(|end| &body[end + 1..])
                .unwrap_or_default()
        } else {
            &body
        };
        let text = String::from_utf8_lossy(body);
        if let Some(line) = text
            .lines()
            .find(|line| line.to_ascii_lowercase().contains(&query))
        {
            entry.transcript_snippet = Some(
                mask_transcript_secrets(line.trim())
                    .chars()
                    .take(140)
                    .collect(),
            );
            result.entries.push(entry);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(dir: &std::path::Path) -> Key {
        let path = dir.join("transcript.txt");
        std::fs::write(&path, "ordinary line\nneedle sk-12345678901234567890\n").unwrap();
        Key {
            entries: vec![SessionHistoryEntry {
                profile_label: String::new(),
                profile_binding: None,
                id: "one".into(),
                agent_id: "test".into(),
                title: "Fixture".into(),
                folder: dir.into(),
                updated_at_ms: 1,
                file_path: Some(path),
                open_pane_id: None,
                transcript_snippet: None,
            }],
            scope: SessionHistoryScope::All,
            active: None,
            roots: vec![],
            agent: None,
            query: "needle".into(),
            transcripts: true,
        }
    }

    #[test]
    fn session_query_scans_bounded_files_and_masks_snippets() {
        let dir = tempfile::tempdir().unwrap();
        let key = key(dir.path());
        let flag = AtomicBool::new(false);
        let result = scan(&key, &flag, Limits::default());
        assert!(!result.limited);
        assert_eq!(result.entries.len(), 1);
        let snippet = result.entries[0].transcript_snippet.as_ref().unwrap();
        assert!(snippet.contains("needle"));
        assert!(!snippet.contains("12345678901234567890"));
        let path = key.entries[0].file_path.as_ref().unwrap();
        std::fs::write(path, format!("{}\nrecent needle\n", "x".repeat(300))).unwrap();
        let tail = scan(
            &key,
            &flag,
            Limits {
                per_file: 40,
                ..Limits::default()
            },
        );
        assert!(tail.limited);
        assert_eq!(tail.entries.len(), 1);
        assert_eq!(
            tail.entries[0].transcript_snippet.as_deref(),
            Some("recent needle")
        );
        for limits in [
            Limits {
                files: 0,
                ..Limits::default()
            },
            Limits {
                bytes: 4,
                ..Limits::default()
            },
            Limits {
                per_file: 4,
                ..Limits::default()
            },
            Limits {
                time: Duration::ZERO,
                ..Limits::default()
            },
        ] {
            let result = scan(&key, &flag, limits);
            assert!(result.limited);
            assert!(result.entries.is_empty());
        }
        flag.store(true, Ordering::Relaxed);
        assert!(scan(&key, &flag, Limits::default()).entries.is_empty());
    }

    #[test]
    fn session_query_debounces_reuses_results_and_refreshes_unchanged_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let key = key(dir.path());
        let mut search = Search::default();
        let start = Instant::now();
        assert!(search.update(key.clone(), start).entries.is_empty());
        assert!(search.pending());
        assert!(search.job.is_none());
        search.update(key.clone(), start + Duration::from_millis(180));
        let deadline = Instant::now() + Duration::from_secs(5);
        while search.pending() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
            search.update(key.clone(), Instant::now());
        }
        assert_eq!(search.result.entries.len(), 1);
        // A cached render must not reopen the file.
        std::fs::remove_file(key.entries[0].file_path.as_ref().unwrap()).unwrap();
        assert_eq!(search.update(key.clone(), Instant::now()).entries.len(), 1);
        assert!(search.job.is_none());
        search.pause();
        assert_eq!(search.update(key.clone(), Instant::now()).entries.len(), 1);
        search.invalidate();
        assert!(search.update(key, Instant::now()).entries.is_empty());
        assert!(search.pending());
    }

    #[test]
    fn session_query_obsolete_completion_cannot_replace_current_metadata_results() {
        let dir = tempfile::tempdir().unwrap();
        let key = key(dir.path());
        let mut search = Search::default();
        let now = Instant::now();
        search.update(key.clone(), now);
        // Hold a synthetic worker open: a replacement must not spawn alongside it.
        let (tx, rx) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        search.job = Some(Job {
            generation: search.generation,
            cancel: cancel.clone(),
            result: rx,
        });
        search.due = None;
        let mut next = key.clone();
        next.query = "fixture".into();
        next.transcripts = false;
        assert_eq!(search.update(next.clone(), now).entries.len(), 1);
        assert!(cancel.load(Ordering::Relaxed));
        assert!(!search.pending());
        tx.send(ResultSet {
            entries: vec![],
            limited: true,
        })
        .unwrap();
        let result = search.update(next, now);
        assert_eq!(result.entries.len(), 1);
        assert!(!result.limited);
        assert!(search.job.is_none());

        search.update(key.clone(), now);
        let (tx, rx) = mpsc::sync_channel(1);
        search.job = Some(Job {
            generation: search.generation,
            cancel,
            result: rx,
        });
        let mut another = key;
        another.query = "new query".into();
        search.update(another.clone(), now + Duration::from_secs(1));
        search.update(another, now + Duration::from_secs(2));
        assert!(
            search.due.is_some(),
            "single-flight worker must block replacement startup"
        );
        drop(tx);
        search.invalidate();
        assert!(!search.pending());
    }
}
