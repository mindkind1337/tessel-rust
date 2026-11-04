//! Background-only model reads. Callers must validate provider/profile/session
//! authority before submitting a job and return metadata, never transcript text.
//! A scope change invalidates even an already completed but unconsumed result.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
};

const MAX_WORKERS: usize = 2;
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
pub const MAX_READ: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Start,
    Io,
    TooLarge,
    InvalidText,
}

struct Permit;
impl Permit {
    fn acquire() -> Option<Self> {
        ACTIVE
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_WORKERS).then_some(n + 1)
            })
            .ok()
            .map(|_| Self)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

struct Pending<T> {
    scope: Arc<()>,
    receiver: Receiver<Result<T, Failure>>,
}

/// One outstanding job per pane, two process-wide. Invalidating or dropping a
/// pane never frees a still-running job's permit, so slow disks cannot create
/// unbounded detached threads. OS filesystem calls themselves are not timed out.
pub struct Worker<K, T> {
    key: Option<K>,
    scope: Arc<()>,
    pending: Option<Pending<T>>,
}
impl<K, T> Default for Worker<K, T> {
    fn default() -> Self {
        Self {
            key: None,
            scope: Arc::new(()),
            pending: None,
        }
    }
}
impl<K: Eq, T: Send + 'static> Worker<K, T> {
    /// Include profile, cwd, conversation and terminal incarnation in the key.
    /// Rebinding A -> B -> A still rejects the old A request.
    pub fn bind(&mut self, key: K) {
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.scope = Arc::new(());
        }
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn start(
        &mut self,
        job: impl FnOnce() -> Result<T, Failure> + Send + 'static,
        wake: impl FnOnce() + Send + 'static,
    ) -> Result<bool, Failure> {
        if self.key.is_none() || self.busy() {
            return Ok(false);
        }
        let Some(permit) = Permit::acquire() else {
            return Ok(false);
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("pane-model-read".into())
            .spawn(move || {
                let result = job();
                let _ = sender.send(result);
                drop(permit);
                wake();
            })
            .map_err(|_| Failure::Start)?;
        self.pending = Some(Pending {
            scope: self.scope.clone(),
            receiver,
        });
        Ok(true)
    }

    /// Nonblocking; failures are reported without replacing the caller's cache.
    pub fn poll(&mut self) -> Option<Result<T, Failure>> {
        let pending = self.pending.as_ref()?;
        let result = match pending.receiver.try_recv() {
            Ok(value) => value,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err(Failure::Io),
        };
        let current = Arc::ptr_eq(&pending.scope, &self.scope);
        self.pending = None;
        current.then_some(result)
    }
}

/// For bounded settings documents. Oversize input is rejected, never silently
/// truncated into a different configuration. Do not call on the UI thread.
pub fn read_document(path: &Path) -> Result<String, Failure> {
    let mut file = regular_file(path)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_READ as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Io)?;
    if bytes.len() > MAX_READ {
        return Err(Failure::TooLarge);
    }
    String::from_utf8(bytes).map_err(|_| Failure::InvalidText)
}

/// Read complete JSONL records from a bounded tail. Drop the first partial
/// record and any unfinished final record (another process may be appending).
pub fn read_jsonl_tail(path: &Path, limit: usize) -> Result<String, Failure> {
    if limit == 0 || limit > MAX_READ {
        return Err(Failure::TooLarge);
    }
    let mut file = regular_file(path)?;
    let length = file.metadata().map_err(|_| Failure::Io)?.len();
    let start = length.saturating_sub(limit as u64);
    // One byte before the window distinguishes a record boundary from a suffix.
    file.seek(SeekFrom::Start(start.saturating_sub(1)))
        .map_err(|_| Failure::Io)?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + u64::from(start > 0))
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Io)?;
    let first = if start > 0 {
        match bytes.iter().position(|b| *b == b'\n') {
            Some(index) => index + 1,
            None => return Ok(String::new()),
        }
    } else {
        0
    };
    let end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    String::from_utf8(bytes[first.min(end)..end].to_vec()).map_err(|_| Failure::InvalidText)
}

fn regular_file(path: &Path) -> Result<File, Failure> {
    // Preflight avoids opening FIFOs/devices in ordinary local provider trees.
    // This is not a sandbox or an authority check; callers own path validation.
    if !std::fs::metadata(path).map_err(|_| Failure::Io)?.is_file() {
        return Err(Failure::Io);
    }
    let file = File::open(path).map_err(|_| Failure::Io)?;
    if !file.metadata().map_err(|_| Failure::Io)?.is_file() {
        return Err(Failure::Io);
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn stale_results_and_aba_are_rejected_without_spawning_more_jobs() {
        let mut worker = Worker::default();
        worker.bind("profile-a/conversation-1");
        let (finish, wait) = mpsc::channel();
        let (done, completed) = mpsc::channel();
        assert!(
            worker
                .start(
                    move || {
                        wait.recv().unwrap();
                        Ok(7)
                    },
                    move || {
                        done.send(()).unwrap();
                    }
                )
                .unwrap()
        );
        worker.bind("profile-b/conversation-2");
        worker.bind("profile-a/conversation-1");
        assert!(!worker.start(|| Ok(9), || {}).unwrap());
        assert!(worker.poll().is_none());
        finish.send(()).unwrap();
        completed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(worker.poll().is_none());
        assert!(!worker.busy());
        let (done, completed) = mpsc::channel();
        assert!(
            worker
                .start(
                    || Ok(11),
                    move || {
                        done.send(()).unwrap();
                    }
                )
                .unwrap()
        );
        completed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(worker.poll(), Some(Ok(11)));

        // Closing/replacing panes must not bypass the global worker ceiling.
        let mut blocked = Vec::new();
        for _ in 0..MAX_WORKERS {
            let (finish, wait) = mpsc::channel();
            let (done, completed) = mpsc::channel();
            let mut pane = Worker::default();
            pane.bind("another-pane");
            assert!(
                pane.start(
                    move || {
                        wait.recv().unwrap();
                        Ok(())
                    },
                    move || {
                        done.send(()).unwrap();
                    }
                )
                .unwrap()
            );
            drop(pane);
            blocked.push((finish, completed));
        }
        assert!(!worker.start(|| Ok(12), || {}).unwrap());
        for (finish, completed) in blocked {
            finish.send(()).unwrap();
            completed.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        let (done, completed) = mpsc::channel();
        assert!(
            worker
                .start(
                    || Err(Failure::Io),
                    move || {
                        done.send(()).unwrap();
                    }
                )
                .unwrap()
        );
        completed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(worker.poll(), Some(Err(Failure::Io)));
    }

    #[test]
    fn documents_and_tails_are_bounded_and_do_not_parse_partial_records() {
        let path =
            std::env::temp_dir().join(format!("tessel-model-worker-{}.tmp", std::process::id()));
        std::fs::write(&path, "old\nα\nlast\npartial").unwrap();
        assert_eq!(read_jsonl_tail(&path, 13).unwrap(), "last\n");
        assert_eq!(read_jsonl_tail(&path, MAX_READ).unwrap(), "old\nα\nlast\n");
        assert_eq!(read_jsonl_tail(&path, 0), Err(Failure::TooLarge));
        std::fs::write(&path, vec![b'x'; MAX_READ + 1]).unwrap();
        assert_eq!(read_document(&path), Err(Failure::TooLarge));
        assert_eq!(read_jsonl_tail(&path, 10).unwrap(), "");
        std::fs::write(&path, [0xff]).unwrap();
        assert_eq!(read_document(&path), Err(Failure::InvalidText));
        std::fs::remove_file(path).unwrap();
    }
}
