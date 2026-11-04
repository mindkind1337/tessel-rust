//! A host's existing terminals, for the picker: `terminal.list`, and what the encrypted store
//! knows about them (which have a saved lease, which creates are still pending), read off the
//! UI thread.
//!
//! Read only: it never creates, attaches or resumes anything, and no lease leaves the store
//! (terminal ids only, pending creates as metadata without their deduplication key).
//!
//! - The listing counts only if the daemon's epoch is the same before the list, after it, and
//!   after the store was read (that read can take a while): otherwise `EpochChanged`, nothing
//!   shown.
//! - A store that cannot be read is said so (`store`, `Saved::Unknown`): never "no lease".
//! - Bounded: at most `MAX_WORKERS` list workers in the application, each holding its permit
//!   until its thread ends, even if its `TerminalList` was dropped; one worker per list, a
//!   refresh during a pass runs one more pass at its end; one result kept, the latest
//!   generation's, never one from before a newer refresh nor any after the list was dropped;
//!   `MAX_LISTED` terminals and `MAX_PENDING` pending creates per listing.
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tessel_host_protocol::pty::Listed;
use tessel_host_protocol::{Epoch, TerminalId};

/// Terminals kept from one listing (the daemon's own limit).
pub const MAX_LISTED: usize = 64;
/// Pending creates kept from one listing.
pub const MAX_PENDING: usize = 64;
/// List workers running at once in the application.
pub const MAX_WORKERS: usize = 4;

static WORKERS: AtomicUsize = AtomicUsize::new(0);

/// The host side, on the list worker. Real: the host connection's client.
pub trait Lister: Send + Sync + 'static {
    /// The live daemon's epoch; `None` while not connected.
    fn epoch(&self) -> Option<Epoch>;
    /// `terminal.list` (blocking).
    fn list(&self) -> Result<Listed, ListError>;
}

/// Why `terminal.list` gave nothing, as far as the picker needs to know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListError {
    NotConnected,
    TimedOut,
    /// The daemon does not know `terminal.list`.
    Unsupported,
    RequestFailed,
}

/// The store side, on the list worker. Real: the owner's encrypted lease store.
pub trait SavedIndex: Send + Sync + 'static {
    /// In one read of the store: the terminals of `alias` on `epoch` that have a saved lease
    /// (their ids, never the lease), and the pending creates of `alias` only (metadata, never
    /// the deduplication key). The filtering by alias is this method's.
    fn index(&self, alias: &str, epoch: &Epoch) -> Result<Index, IndexError>;
}

#[derive(Debug, Default)]
pub struct Index {
    pub saved: HashSet<TerminalId>,
    /// Pending creates of the alias asked for.
    pub pending: Vec<PendingMeta>,
}

/// A pending create, as metadata: what an explicit cleanup or recovery needs to name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingMeta {
    pub id: String,
    pub epoch: Epoch,
}

/// Why the store could not be read. Distinct, and never shown as "no lease".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexError {
    /// Another Tessel window holds it.
    Locked,
    /// It cannot be read; it was left as it is.
    Corrupt,
    Unavailable,
}

/// Whether the store holds a lease for a listed terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saved {
    Yes,
    No,
    /// The store could not be read: maybe.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exited {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

/// One listed terminal. Holds no lease.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub terminal: TerminalId,
    pub pid: u32,
    /// `(rows, cols)`.
    pub size: (u16, u16),
    pub seq: u64,
    pub attached: u32,
    /// Someone holds its lease on the host (maybe this desktop, maybe not).
    pub leased_on_host: bool,
    pub retained_bytes: u64,
    /// The program ended and the daemon still keeps it for a while.
    pub exited: Option<Exited>,
    pub saved: Saved,
}

/// A pending create of this alias. `current`: made on the daemon run listed (else its daemon
/// restarted: whatever it started is gone).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingRow {
    pub id: String,
    pub epoch: Epoch,
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub generation: u64,
    pub alias: String,
    /// The daemon run listed: compare it with the live epoch before acting on an entry.
    pub epoch: Epoch,
    pub entries: Vec<Entry>,
    /// The daemon listed more than `MAX_LISTED`.
    pub truncated: bool,
    /// Whether the store was read; when not, every entry is `Saved::Unknown`, no pending row.
    pub store: Result<(), IndexError>,
    pub pending: Vec<PendingRow>,
    pub pending_truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListFailure {
    NotConnected,
    TimedOut,
    /// The daemon restarted while listing: nothing of it is shown.
    EpochChanged,
    Unsupported,
    RequestFailed,
}

impl From<ListError> for ListFailure {
    fn from(error: ListError) -> Self {
        match error {
            ListError::NotConnected => Self::NotConnected,
            ListError::TimedOut => Self::TimedOut,
            ListError::Unsupported => Self::Unsupported,
            ListError::RequestFailed => Self::RequestFailed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Polled {
    Listed(Listing),
    Failed {
        generation: u64,
        failure: ListFailure,
    },
}

/// A refresh that did not start; nothing changed. Try again explicitly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Every list worker of the application is busy.
    Capacity,
    /// The worker thread could not start.
    NoThread,
}

type Ready = Arc<dyn Fn() + Send + Sync>;
type Sources = (Arc<dyn Lister>, Arc<dyn SavedIndex>, Ready);

struct Inner {
    alias: String,
    /// The latest refresh asked for.
    generation: u64,
    running: bool,
    /// A refresh came during the pass: one more pass at its end.
    rerun: bool,
    dropped: bool,
    sources: Option<Sources>,
    /// The latest generation's result, until polled.
    result: Option<Polled>,
}

/// A worker's place in the application bound, held until its thread ends.
struct WorkerPermit(&'static AtomicUsize);

impl WorkerPermit {
    fn acquire(pool: &'static AtomicUsize) -> Option<Self> {
        pool.try_update(Ordering::SeqCst, Ordering::SeqCst, |running| {
            (running < MAX_WORKERS).then_some(running + 1)
        })
        .ok()
        .map(|_| Self(pool))
    }
}

impl Drop for WorkerPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The picker's side, one per host. Dropping it stops showing results: a worker still running
/// finishes its call, deposits nothing, and gives its permit back when its thread ends.
pub struct TerminalList {
    inner: Arc<Mutex<Inner>>,
    pool: &'static AtomicUsize,
}

impl TerminalList {
    pub fn new(alias: String) -> Self {
        Self::bounded_by(alias, &WORKERS)
    }

    /// As `new`, counted in `pool` instead of the application's bound (tests).
    pub fn bounded_by(alias: String, pool: &'static AtomicUsize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                alias,
                generation: 0,
                running: false,
                rerun: false,
                dropped: false,
                sources: None,
                result: None,
            })),
            pool,
        }
    }

    /// Lists again, off the UI thread; `ready` is called (non-blocking) when a result waits for
    /// `poll`. A refresh during a pass makes that pass's result stale and runs one more pass at
    /// its end. Refused: nothing changed (no new generation).
    pub fn refresh(
        &mut self,
        lister: Arc<dyn Lister>,
        index: Arc<dyn SavedIndex>,
        ready: impl Fn() + Send + Sync + 'static,
    ) -> Result<(), Refused> {
        let ready: Ready = Arc::new(ready);
        let mut inner = self.inner.lock().unwrap();
        if inner.running {
            inner.generation += 1;
            inner.result = None;
            inner.sources = Some((lister, index, ready));
            inner.rerun = true;
            return Ok(());
        }
        let permit = WorkerPermit::acquire(self.pool).ok_or(Refused::Capacity)?;
        inner.generation += 1;
        inner.sources = Some((lister, index, ready));
        inner.running = true;
        let shared = self.inner.clone();
        let spawned = std::thread::Builder::new()
            .name("host-terminal-list".into())
            .spawn(move || work(&shared, permit));
        if spawned.is_err() {
            // The closure, with the permit, was dropped: nothing runs.
            inner.generation -= 1;
            inner.running = false;
            inner.sources = None;
            return Err(Refused::NoThread);
        }
        inner.result = None;
        Ok(())
    }

    /// The latest refresh's result, once, if it came. Call it on every frame; no I/O.
    pub fn poll(&mut self) -> Option<Polled> {
        self.inner.lock().unwrap().result.take()
    }

    /// The latest refresh asked for (0 before the first).
    pub fn generation(&self) -> u64 {
        self.inner.lock().unwrap().generation
    }
}

impl Drop for TerminalList {
    fn drop(&mut self) {
        let mut inner = self.inner.lock().unwrap();
        inner.dropped = true;
        inner.result = None;
        inner.sources = None;
    }
}

fn work(shared: &Mutex<Inner>, permit: WorkerPermit) {
    // Held until this thread ends, whatever happens to the list.
    let _permit = permit;
    loop {
        let (generation, alias, lister, index, ready) = {
            let mut inner = shared.lock().unwrap();
            let Some((lister, index, ready)) = inner.sources.clone().filter(|_| !inner.dropped)
            else {
                inner.running = false;
                return;
            };
            inner.rerun = false;
            (inner.generation, inner.alias.clone(), lister, index, ready)
        };
        let polled = pass(generation, &alias, &*lister, &*index);
        let again = {
            let mut inner = shared.lock().unwrap();
            if inner.dropped {
                inner.running = false;
                return;
            }
            if generation == inner.generation {
                inner.result = Some(polled);
            }
            if !inner.rerun {
                inner.running = false;
            }
            inner.rerun
        };
        if !again {
            ready();
            return;
        }
    }
}

fn pass(generation: u64, alias: &str, lister: &dyn Lister, index: &dyn SavedIndex) -> Polled {
    let failed = |failure| Polled::Failed {
        generation,
        failure,
    };
    let Some(epoch) = lister.epoch() else {
        return failed(ListFailure::NotConnected);
    };
    let listed = match lister.list() {
        Ok(listed) => listed,
        Err(error) => return failed(error.into()),
    };
    if lister.epoch().as_ref() != Some(&epoch) {
        return failed(ListFailure::EpochChanged);
    }
    let read = index.index(alias, &epoch);
    // The store read can take a while: the daemon may have restarted meanwhile.
    if lister.epoch().as_ref() != Some(&epoch) {
        return failed(ListFailure::EpochChanged);
    }
    let (saved, store, pending) = match read {
        Ok(read) => (Some(read.saved), Ok(()), read.pending),
        Err(error) => (None, Err(error), Vec::new()),
    };
    let truncated = listed.terminals.len() > MAX_LISTED;
    let entries = listed
        .terminals
        .into_iter()
        .take(MAX_LISTED)
        .map(|info| Entry {
            saved: match &saved {
                None => Saved::Unknown,
                Some(saved) if saved.contains(&info.terminal) => Saved::Yes,
                Some(_) => Saved::No,
            },
            terminal: info.terminal,
            pid: info.pid,
            size: (info.size.rows, info.size.cols),
            seq: info.seq,
            attached: info.attached,
            leased_on_host: info.leased,
            retained_bytes: info.retained_bytes,
            exited: info.exited.map(|exit| Exited {
                code: exit.code,
                signal: exit.signal,
            }),
        })
        .collect();
    let pending_truncated = pending.len() > MAX_PENDING;
    let pending = pending
        .into_iter()
        .take(MAX_PENDING)
        .map(|meta| PendingRow {
            current: meta.epoch == epoch,
            id: meta.id,
            epoch: meta.epoch,
        })
        .collect();
    Polled::Listed(Listing {
        generation,
        alias: alias.to_owned(),
        epoch,
        entries,
        truncated,
        store,
        pending,
        pending_truncated,
    })
}
