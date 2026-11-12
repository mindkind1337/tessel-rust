//! Bounded single-flight transcript previews. Filesystem work never runs on the UI thread.
use crate::side_panels_model::{ActivityMessage, mask_transcript_secrets};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime},
};

const WINDOW: usize = 256 * 1024;
const MAX_BODY: usize = 32 * 1024;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_MESSAGES: usize = 2000;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Input {
    pub root: PathBuf,
    pub conversation: String,
    pub pane: u64,
    pub title: String,
}
struct Cached {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
    rows: Arc<[ActivityMessage]>,
    limited: bool,
}
#[derive(Default)]
struct Output {
    rows: Vec<ActivityMessage>,
    cache: BTreeMap<Input, Cached>,
    limited: bool,
}
struct Job {
    generation: u64,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Output>,
}

#[derive(Default)]
pub(crate) struct Loader {
    inputs: Option<Vec<Input>>,
    generation: u64,
    job: Option<Job>,
    due: Option<Instant>,
    pending: bool,
    paused: bool,
    rows: Arc<[ActivityMessage]>,
    cache: BTreeMap<Input, Cached>,
    pub limited: bool,
    pub revision: u64,
}
impl Drop for Loader {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl Loader {
    #[cfg(test)]
    pub(crate) fn requested(&self) -> bool {
        self.inputs.is_some()
    }
    pub(crate) fn loading(&self) -> bool {
        !self.paused && (self.pending || self.job.is_some())
    }
    pub(crate) fn needs_frame(&self) -> bool {
        !self.paused && (self.loading() || self.due.is_some_and(|due| Instant::now() >= due))
    }
    pub(crate) fn pause(&mut self) {
        if !self.paused
            && let Some(job) = &self.job
        {
            job.cancel.store(true, Ordering::Relaxed);
            self.generation = self.generation.wrapping_add(1);
            self.pending = true;
        }
        self.paused = true;
    }
    fn prune_cache(&mut self) {
        let inputs = self.inputs.as_deref().unwrap_or_default();
        self.cache
            .retain(|key, _| inputs.iter().take(100).any(|input| same_source(input, key)));
    }
    pub(crate) fn update(&mut self, inputs: Vec<Input>, now: Instant) -> Arc<[ActivityMessage]> {
        self.paused = false;
        if self.inputs.as_ref() != Some(&inputs) {
            if let Some(job) = &self.job {
                job.cancel.store(true, Ordering::Relaxed);
            }
            self.generation = self.generation.wrapping_add(1);
            self.revision = self.revision.wrapping_add(1);
            self.pending = !inputs.is_empty();
            self.rows = self
                .rows
                .iter()
                .filter(|row| {
                    inputs.iter().take(100).any(|input| {
                        row.thread_key == thread_key(input)
                            && self
                                .inputs
                                .as_deref()
                                .unwrap_or_default()
                                .iter()
                                .any(|old| same_source(old, input))
                    })
                })
                .cloned()
                .collect::<Vec<_>>()
                .into();
            self.inputs = Some(inputs);
            self.prune_cache();
            self.limited = false;
            self.due = None;
        }
        if let Some(job) = &self.job {
            match job.rx.try_recv() {
                Ok(output) => {
                    self.cache = output.cache;
                    if job.generation == self.generation {
                        self.rows = output.rows.into();
                        self.limited = output.limited;
                        self.revision = self.revision.wrapping_add(1);
                        self.due = Some(now + Duration::from_secs(2));
                    }
                    self.job = None;
                    self.prune_cache();
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    if job.generation == self.generation {
                        self.limited = true;
                        self.due = Some(now + Duration::from_secs(5));
                    }
                    self.job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.job.is_none() && (self.pending || self.due.is_some_and(|due| now >= due)) {
            self.pending = false;
            self.due = None;
            let inputs = self.inputs.as_ref().expect("initialized preview").clone();
            let cache = std::mem::take(&mut self.cache);
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let (tx, rx) = mpsc::sync_channel(1);
            let spawn = std::thread::Builder::new()
                .name("conversation-preview".into())
                .spawn(move || {
                    let _ = tx.send(scan(inputs, cache, &flag));
                });
            if spawn.is_ok() {
                self.job = Some(Job {
                    generation: self.generation,
                    cancel,
                    rx,
                });
            } else {
                self.limited = true;
                self.due = Some(now + Duration::from_secs(5));
            }
        }
        self.rows.clone()
    }
}

fn same_source(a: &Input, b: &Input) -> bool {
    a.root == b.root && a.conversation == b.conversation && a.pane == b.pane
}
fn thread_key(input: &Input) -> String {
    format!("claude:{}:{}", input.pane, input.conversation)
}
fn stopped(cancel: &AtomicBool, start: Instant) -> bool {
    cancel.load(Ordering::Relaxed) || start.elapsed() >= Duration::from_secs(3)
}
fn locate(
    input: &Input,
    cancel: &AtomicBool,
    start: Instant,
    visited: &mut usize,
) -> Option<PathBuf> {
    if !crate::claude::valid_uuid(&input.conversation) {
        return None;
    }
    for entry in fs::read_dir(input.root.join("projects")).ok()? {
        if stopped(cancel, start) || *visited >= 10_000 {
            return None;
        }
        *visited += 1;
        let entry = entry.ok()?;
        if !entry.file_type().ok()?.is_dir() {
            continue;
        }
        let path = entry.path().join(format!("{}.jsonl", input.conversation));
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink()) {
            return Some(path);
        }
    }
    None
}
fn scan(inputs: Vec<Input>, mut previous: BTreeMap<Input, Cached>, cancel: &AtomicBool) -> Output {
    let start = Instant::now();
    let mut output = Output::default();
    let mut visited = 0;
    let mut bytes: usize = 0;
    previous.retain(|key, _| inputs.iter().take(100).any(|input| same_source(input, key)));
    for (index, input) in inputs.into_iter().enumerate() {
        if index >= 100 || stopped(cancel, start) {
            output.limited = true;
            break;
        }
        let source_key = previous
            .keys()
            .find(|key| same_source(key, &input))
            .cloned();
        let old = source_key.and_then(|key| previous.remove(&key));
        let path = old
            .as_ref()
            .map(|c| c.path.clone())
            .or_else(|| locate(&input, cancel, start, &mut visited));
        let Some(path) = path else {
            output.limited = true;
            continue;
        };
        let Ok(meta) = fs::symlink_metadata(&path) else {
            output.limited = true;
            continue;
        };
        if !meta.is_file() || meta.file_type().is_symlink() {
            output.limited = true;
            continue;
        }
        let modified = meta.modified().ok();
        let cached = if let Some(old) = old
            .filter(|old| modified.is_some() && old.size == meta.len() && old.modified == modified)
        {
            old
        } else {
            let turns = crate::prompt_before::read_claude_window_messages_bounded(
                &path,
                WINDOW,
                crate::side_panels_model::now_unix_ms(),
                crate::prompt_before::ScanOptions {
                    chunk: 64 * 1024,
                    budget: 1024 * 1024,
                    line_max: 256 * 1024,
                    stop_at: 0,
                },
            );
            let Ok(turns) = turns else {
                output.limited = true;
                continue;
            };
            let mut limited = false;
            let rows = turns
                .into_iter()
                .enumerate()
                .take(MAX_MESSAGES + 1)
                .map(|(i, turn)| {
                    let mut body = mask_transcript_secrets(&turn.body);
                    if body.len() > MAX_BODY {
                        let mut end = MAX_BODY;
                        while !body.is_char_boundary(end) {
                            end -= 1;
                        }
                        body.truncate(end);
                        limited = true;
                    }
                    ActivityMessage {
                        id: 1_000_000u64
                            .saturating_add(input.pane.saturating_mul(10_000))
                            .saturating_add(i as u64),
                        thread_key: thread_key(&input),
                        thread_title: input.title.clone(),
                        pane_id: Some(input.pane),
                        recipient: if turn.sender == "You" {
                            input.title.clone()
                        } else {
                            "You".into()
                        },
                        sender: turn.sender,
                        body,
                        timestamp_ms: turn.timestamp_ms,
                    }
                })
                .collect::<Vec<_>>();
            Cached {
                path,
                size: meta.len(),
                modified,
                rows: rows.into(),
                limited,
            }
        };
        output.limited |= cached.limited;
        let mut accepted = Vec::new();
        for row in cached.rows.iter() {
            if output.rows.len() >= MAX_MESSAGES || bytes.saturating_add(row.body.len()) > MAX_BYTES
            {
                output.limited = true;
                break;
            }
            bytes += row.body.len();
            let mut row = row.clone();
            row.thread_title = input.title.clone();
            if row.sender == "You" {
                row.recipient = input.title.clone();
            }
            output.rows.push(row.clone());
            accepted.push(row);
        }
        let truncated = accepted.len() != cached.rows.len();
        if truncated {
            break;
        }
        output.cache.insert(
            input,
            Cached {
                rows: accepted.into(),
                ..cached
            },
        );
    }
    output.cache.extend(previous);
    let mut cache_bytes = 0usize;
    output.cache.retain(|_, cached| {
        let size = cached.rows.iter().map(|row| row.body.len()).sum::<usize>();
        if cache_bytes.saturating_add(size) > MAX_BYTES {
            return false;
        }
        cache_bytes += size;
        true
    });
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(root: PathBuf) -> Input {
        Input {
            root,
            conversation: "12345678-1234-1234-1234-123456789abc".into(),
            pane: 7,
            title: "Synthetic".into(),
        }
    }
    fn record(body: &str) -> String {
        format!(
            "{}\n",
            serde_json::json!({"type":"user","message":{"role":"user","content":body}})
        )
    }
    #[test]
    fn preview_reads_masks_bounds_and_reuses_unchanged_transcript() {
        let tmp = tempfile::tempdir().unwrap();
        let input = input(tmp.path().to_owned());
        let dir = tmp.path().join("projects/synthetic");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.jsonl", input.conversation));
        fs::write(&path, record(&"é".repeat(MAX_BODY))).unwrap();
        let cancel = AtomicBool::new(false);
        let first = scan(vec![input.clone()], BTreeMap::new(), &cancel);
        assert_eq!(first.rows.len(), 1);
        assert_eq!(first.rows[0].body.len(), MAX_BODY);
        assert!(first.limited);
        let timestamp = first.rows[0].timestamp_ms;
        let second = scan(vec![input.clone()], first.cache, &cancel);
        assert_eq!(second.rows[0].timestamp_ms, timestamp);
        fs::write(&path, record("replacement prompt")).unwrap();
        let third = scan(vec![input], second.cache, &cancel);
        assert_eq!(third.rows[0].body, "replacement prompt");
        assert!(!third.limited);
    }
    #[test]
    fn preview_rejects_stale_completion_and_pauses_without_parallel_job() {
        let mut loader = Loader::default();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(1);
        loader.job = Some(Job {
            generation: 0,
            cancel: cancel.clone(),
            rx,
        });
        loader.update(vec![], Instant::now());
        assert!(cancel.load(Ordering::Relaxed));
        assert!(loader.job.is_some());
        tx.send(Output {
            limited: true,
            ..Output::default()
        })
        .ok()
        .unwrap();
        loader.update(vec![], Instant::now());
        assert!(!loader.limited);
        assert!(loader.job.is_none());
        assert!(loader.due.is_none());
        let (tx, rx) = mpsc::sync_channel(1);
        loader.job = Some(Job {
            generation: loader.generation,
            cancel: cancel.clone(),
            rx,
        });
        loader.pause();
        assert!(!loader.needs_frame());
        assert!(!loader.loading());
        assert!(loader.pending);
        drop(tx);
    }

    #[test]
    fn renamed_input_keeps_cached_preview_and_removed_root_drops_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let original = input(tmp.path().to_owned());
        let dir = tmp.path().join("projects/synthetic");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(format!("{}.jsonl", original.conversation)),
            record("preview"),
        )
        .unwrap();
        let first = scan(
            vec![original.clone()],
            BTreeMap::new(),
            &AtomicBool::new(false),
        );
        let stamp = first.rows[0].timestamp_ms;
        let mut renamed = original.clone();
        renamed.title = "Renamed".into();
        let second = scan(vec![renamed.clone()], first.cache, &AtomicBool::new(false));
        assert_eq!(second.rows[0].timestamp_ms, stamp);
        assert_eq!(second.rows[0].thread_title, "Renamed");
        assert_eq!(second.rows[0].recipient, "Renamed");
        let cancelled = scan(vec![renamed.clone()], second.cache, &AtomicBool::new(true));
        assert_eq!(cancelled.cache.len(), 1);
        let (tx, rx) = mpsc::sync_channel(1);
        let mut loader = Loader::default();
        loader.inputs = Some(vec![original]);
        loader.rows = second.rows.into();
        loader.job = Some(Job {
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            rx,
        });
        assert_eq!(
            loader.update(vec![renamed.clone()], Instant::now()).len(),
            1
        );
        renamed.root = tmp.path().join("different-account");
        assert!(loader.update(vec![renamed], Instant::now()).is_empty());
        drop(tx);
    }
    #[test]
    fn preview_cancel_and_invalid_id_never_read_transcripts() {
        let cancelled = AtomicBool::new(true);
        assert!(
            scan(
                vec![input(PathBuf::from("missing"))],
                BTreeMap::new(),
                &cancelled
            )
            .limited
        );
        let mut invalid = input(PathBuf::from("missing"));
        invalid.conversation = "../../outside".into();
        assert!(locate(&invalid, &AtomicBool::new(false), Instant::now(), &mut 0).is_none());
    }

    #[test]
    fn global_message_limit_does_not_cache_incomplete_file() {
        let tmp = tempfile::tempdir().unwrap();
        let input = input(tmp.path().to_owned());
        let dir = tmp.path().join("projects/synthetic");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(format!("{}.jsonl", input.conversation)),
            record("tiny").repeat(MAX_MESSAGES + 1),
        )
        .unwrap();
        let result = scan(vec![input], BTreeMap::new(), &AtomicBool::new(false));
        assert_eq!(result.rows.len(), MAX_MESSAGES);
        assert!(result.limited);
        assert!(result.cache.is_empty());
    }
    #[test]
    fn bounded_recovery_does_not_attach_answers_to_an_unknown_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("synthetic.jsonl");
        let answer = "{\"type\":\"assistant\",\"message\":{\"content\":\"hidden answer\"}}\n";
        fs::write(
            &path,
            format!("{}{}\n{}", record("old prompt"), " ".repeat(4096), answer),
        )
        .unwrap();
        let rows = crate::prompt_before::read_claude_window_messages_bounded(
            &path,
            answer.len() + 1,
            0,
            crate::prompt_before::ScanOptions {
                chunk: 128,
                budget: 256,
                line_max: 128,
                stop_at: 0,
            },
        )
        .unwrap();
        assert!(rows.is_empty());
    }
}
