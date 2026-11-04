//! The existing terminals' list worker with a synthetic host and store: the daemon's epoch
//! checked around the list and the store read, saved leases exact or said unknown, bounds,
//! one coalesced worker per list, an application bound kept until a worker's thread ends, and
//! nothing shown from before a newer refresh or after the list was dropped.
#[allow(dead_code)]
#[path = "../src/terminal/host_terminal_list.rs"]
mod host_terminal_list;

use host_terminal_list::{
    Entry, Exited, Index, IndexError, ListError, ListFailure, Lister, Listing, MAX_LISTED,
    MAX_PENDING, MAX_WORKERS, PendingMeta, PendingRow, Polled, Refused, Saved, SavedIndex,
    TerminalList,
};
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tessel_host_protocol::pty::{Exit, Listed, Size, TerminalInfo};
use tessel_host_protocol::{Epoch, TerminalId};

fn epoch(n: u8) -> Epoch {
    Epoch::from_bytes([n; 16])
}

fn id(n: u8) -> TerminalId {
    TerminalId::from_bytes([n; 16])
}

fn info(n: u8) -> TerminalInfo {
    TerminalInfo {
        terminal: id(n),
        pid: 100 + u32::from(n),
        size: Size { rows: 24, cols: 80 },
        seq: u64::from(n),
        attached: 1,
        leased: true,
        retained_bytes: 10,
        exited: None,
    }
}

fn listed(terminals: Vec<TerminalInfo>) -> Listed {
    Listed {
        terminals,
        daemon_rss_bytes: 0,
        retained_bytes: 0,
    }
}

// ---- A synthetic host ------------------------------------------------------------------

#[derive(Default)]
struct HostState {
    /// Successive answers of `epoch()`; the last one repeats.
    epochs: Vec<Option<Epoch>>,
    epoch_calls: usize,
    list: Option<Result<Listed, ListError>>,
    lists: usize,
    gate: Option<Receiver<()>>,
    started: Option<Sender<()>>,
}

#[derive(Default)]
struct FakeHost(Mutex<HostState>);

impl FakeHost {
    fn with(epochs: Vec<Option<Epoch>>, list: Result<Listed, ListError>) -> Arc<Self> {
        Arc::new(Self(Mutex::new(HostState {
            epochs,
            list: Some(list),
            ..HostState::default()
        })))
    }

    /// The next list waits for the returned sender; the receiver hears when it started.
    fn gated(&self) -> (Sender<()>, Receiver<()>) {
        let (release, gate) = mpsc::channel();
        let (started, heard) = mpsc::channel();
        let mut state = self.0.lock().unwrap();
        state.gate = Some(gate);
        state.started = Some(started);
        (release, heard)
    }

    fn lists(&self) -> usize {
        self.0.lock().unwrap().lists
    }
}

impl Lister for FakeHost {
    fn epoch(&self) -> Option<Epoch> {
        let mut state = self.0.lock().unwrap();
        let at = state.epoch_calls.min(state.epochs.len().saturating_sub(1));
        state.epoch_calls += 1;
        state.epochs.get(at).cloned().flatten()
    }

    fn list(&self) -> Result<Listed, ListError> {
        let (gate, started) = {
            let mut state = self.0.lock().unwrap();
            state.lists += 1;
            (state.gate.take(), state.started.take())
        };
        if let Some(started) = started {
            let _ = started.send(());
        }
        if let Some(gate) = gate {
            let _ = gate.recv_timeout(Duration::from_secs(5));
        }
        self.0
            .lock()
            .unwrap()
            .list
            .clone()
            .unwrap_or_else(|| Ok(listed(Vec::new())))
    }
}

// ---- A synthetic store -----------------------------------------------------------------

#[derive(Default)]
struct FakeIndex {
    saved: HashSet<TerminalId>,
    pending: Vec<PendingMeta>,
    error: Option<IndexError>,
    reads: AtomicUsize,
}

impl SavedIndex for FakeIndex {
    fn index(&self, _alias: &str, _epoch: &Epoch) -> Result<Index, IndexError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(Index {
            saved: self.saved.clone(),
            pending: self.pending.clone(),
        })
    }
}

// ---- Helpers ---------------------------------------------------------------------------

fn result(list: &mut TerminalList) -> Polled {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(polled) = list.poll() {
            return polled;
        }
        assert!(Instant::now() < deadline, "no result came");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn listing(list: &mut TerminalList) -> Listing {
    match result(list) {
        Polled::Listed(listing) => listing,
        Polled::Failed { failure, .. } => panic!("failed: {failure:?}"),
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

// ---- Tests -----------------------------------------------------------------------------

#[test]
fn nothing_is_asked_while_not_connected() {
    static POOL: AtomicUsize = AtomicUsize::new(0);
    let host = FakeHost::with(vec![None], Ok(listed(vec![info(1)])));
    let index = Arc::new(FakeIndex::default());
    let mut list = TerminalList::bounded_by("fixture".into(), &POOL);
    list.refresh(host.clone(), index.clone(), || {}).unwrap();
    assert_eq!(
        result(&mut list),
        Polled::Failed {
            generation: 1,
            failure: ListFailure::NotConnected
        }
    );
    assert_eq!(host.lists(), 0, "no list without a connection");
    assert_eq!(index.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn a_restart_during_the_list_shows_nothing() {
    static POOL: AtomicUsize = AtomicUsize::new(0);
    let host = FakeHost::with(
        vec![Some(epoch(1)), Some(epoch(2))],
        Ok(listed(vec![info(1)])),
    );
    let index = Arc::new(FakeIndex::default());
    let mut list = TerminalList::bounded_by("fixture".into(), &POOL);
    list.refresh(host, index.clone(), || {}).unwrap();
    assert!(matches!(
        result(&mut list),
        Polled::Failed {
            failure: ListFailure::EpochChanged,
            ..
        }
    ));
    assert_eq!(
        index.reads.load(Ordering::SeqCst),
        0,
        "the store is not read"
    );
}

#[test]
fn a_restart_during_the_store_read_shows_nothing() {
    static POOL: AtomicUsize = AtomicUsize::new(0);
    let host = FakeHost::with(
        vec![Some(epoch(1)), Some(epoch(1)), Some(epoch(2))],
        Ok(listed(vec![info(1)])),
    );
    let index = Arc::new(FakeIndex::default());
    let mut list = TerminalList::bounded_by("fixture".into(), &POOL);
    list.refresh(host, index.clone(), || {}).unwrap();
    assert!(matches!(
        result(&mut list),
        Polled::Failed {
            failure: ListFailure::EpochChanged,
            ..
        }
    ));
    assert_eq!(index.reads.load(Ordering::SeqCst), 1);
}

#[test]
fn list_errors_are_reported_as_such() {
    static POOL: AtomicUsize = AtomicUsize::new(0);
    for (error, failure) in [
        (ListError::NotConnected, ListFailure::NotConnected),
        (ListError::TimedOut, ListFailure::TimedOut),
        (ListError::Unsupported, ListFailure::Unsupported),
        (ListError::RequestFailed, ListFailure::RequestFailed),
    ] {
        let host = FakeHost::with(vec![Some(epoch(1))], Err(error));
        let mut list = TerminalList::bounded_by("fixture".into(), &POOL);
        list.refresh(host, Arc::new(FakeIndex::default()), || {})
            .unwrap();
        assert_eq!(
            result(&mut list),
            Polled::Failed {
                generation: 1,
                failure
            }
        );
    }
}

#[test]
fn saved_leases_are_marked_exactly_and_entries_keep_the_host_s_facts() {
    static POOL: AtomicUsize = AtomicUsize::new(0);
    let mut ended = info(3);
    ended.exited = Some(Exit {
        seq: 9,
        code: Some(0),
        signal: None,
    });
    let host = FakeHost::with(
        vec![Some(epoch(1))],
        Ok(listed(vec![info(1), info(2), ended])),
    );
    let index = Arc::new(FakeIndex {
        saved: HashSet::from([id(1), id(3)]),
        ..FakeIndex::default()
    });
    let mut list = TerminalList::bounded_by("fixture".into(), &POOL);
    list.refresh(host, index, || {}).unwrap();
    let listing = listing(&mut list);
    assert_eq!(listing.generation, 1);
    assert_eq!(listing.alias, "fixture");
    assert_eq!(listing.epoch, epoch(1));
    assert_eq!(listing.store, Ok(()));
    assert!(!listing.truncated);
    let saved: Vec<Saved> = listing.entries.iter().map(|e| e.saved).collect();
    assert_eq!(saved, [Saved::Yes, Saved::No, Saved::Yes]);
    assert_eq!(
        listing.entries[2],
        Entry {
            terminal: id(3),
            pid: 103,
            size: (24, 80),
            seq: 3,
            attached: 1,
            leased_on_host: true,
            retained_bytes: 10,
            exited: Some(Exited {
                code: Some(0),
                signal: None
            }),
            saved: Saved::Yes,
        }
    );
}

#[test]
fn an_unreadable_store_says_unknown_never_no() {
    static POOL: AtomicUsize = AtomicUsize::new(0);
    for error in [
        IndexError::Locked,
        IndexError::Corrupt,
        IndexError::Unavailable,
    ] {
        let host = FakeHost::with(vec![Some(epoch(1))], Ok(listed(vec![info(1), info(2)])));
        let index = Arc::new(FakeIndex {
            error: Some(error),
            pending: vec![PendingMeta {
                id: "pending-1".into(),
                epoch: epoch(1),
            }],
            ..FakeIndex::default()
        });
        let mut list = TerminalList::bounded_by("fixture".into(), &POOL);
        list.refresh(host, index, || {}).unwrap();
        let listing = listing(&mut list);
        assert_eq!(listing.store, Err(error), "the reason is kept, distinct");
        assert!(
            listing.entries.iter().all(|e| e.saved == Saved::Unknown),
            "never shown as no lease"
        );
        assert!(listing.pending.is_empty());
        assert_eq!(listing.entries.len(), 2, "the host's list is still shown");
    }
}

#[test]
fn listings_and_pending_creates_are_bounded() {
    static POOL: AtomicUsize = AtomicUsize::new(0);
    let many = (0..70u8).map(info).collect();
    let host = FakeHost::with(vec![Some(epoch(1))], Ok(listed(many)));
    let pending = (0..70u8)
        .map(|n| PendingMeta {
            id: format!("pending-{n}"),
            epoch: if n == 0 { epoch(1) } else { epoch(9) },
        })
        .collect();
    let index = Arc::new(FakeIndex {
        pending,
        ..FakeIndex::default()
    });
    let mut list = TerminalList::bounded_by("fixture".into(), &POOL);
    list.refresh(host, index, || {}).unwrap();
    let listing = listing(&mut list);
    assert_eq!(listing.entries.len(), MAX_LISTED);
    assert!(listing.truncated);
    assert_eq!(listing.pending.len(), MAX_PENDING);
    assert!(listing.pending_truncated);
    assert_eq!(
        listing.pending[0],
        PendingRow {
            id: "pending-0".into(),
            epoch: epoch(1),
            current: true
        }
    );
    assert!(
        !listing.pending[1].current,
        "made on a daemon run that is gone"
    );
}

#[test]
fn a_refresh_during_a_pass_runs_one_more_and_only_the_latest_shows() {
    static POOL: AtomicUsize = AtomicUsize::new(0);
    let host = FakeHost::with(vec![Some(epoch(1))], Ok(listed(vec![info(1)])));
    let index = Arc::new(FakeIndex::default());
    let (release, started) = host.gated();
    let ready = Arc::new(AtomicUsize::new(0));
    let mut list = TerminalList::bounded_by("fixture".into(), &POOL);
    let heard = ready.clone();
    list.refresh(host.clone(), index.clone(), move || {
        heard.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    for _ in 0..2 {
        let heard = ready.clone();
        list.refresh(host.clone(), index.clone(), move || {
            heard.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
    }
    assert_eq!(list.generation(), 3);
    assert_eq!(POOL.load(Ordering::SeqCst), 1, "one worker for the list");
    assert!(list.poll().is_none());
    release.send(()).unwrap();
    let listing = listing(&mut list);
    assert_eq!(listing.generation, 3, "the stale pass shows nothing");
    assert_eq!(host.lists(), 2, "one pass, then one more for the refreshes");
    wait_until("the worker to end", || POOL.load(Ordering::SeqCst) == 0);
    assert!(list.poll().is_none(), "one result");
    assert_eq!(ready.load(Ordering::SeqCst), 1);
}

#[test]
fn workers_are_bounded_across_lists_and_hold_their_place_after_a_drop() {
    static POOL: AtomicUsize = AtomicUsize::new(0);
    let index: Arc<dyn SavedIndex> = Arc::new(FakeIndex::default());
    let mut lists = Vec::new();
    let mut releases = Vec::new();
    for _ in 0..MAX_WORKERS {
        let host = FakeHost::with(vec![Some(epoch(1))], Ok(listed(vec![info(1)])));
        let (release, started) = host.gated();
        let mut list = TerminalList::bounded_by("fixture".into(), &POOL);
        list.refresh(host, index.clone(), || {}).unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        lists.push(list);
        releases.push(release);
    }
    let host = FakeHost::with(vec![Some(epoch(1))], Ok(listed(vec![info(1)])));
    let mut extra = TerminalList::bounded_by("fixture".into(), &POOL);
    assert_eq!(
        extra.refresh(host.clone(), index.clone(), || {}),
        Err(Refused::Capacity)
    );
    assert_eq!(extra.generation(), 0, "a refused refresh changes nothing");
    // A dropped list's worker still holds its place until its thread ends.
    drop(lists.remove(0));
    assert_eq!(
        extra.refresh(host.clone(), index.clone(), || {}),
        Err(Refused::Capacity)
    );
    releases.remove(0).send(()).unwrap();
    wait_until("the dropped list's worker to end", || {
        POOL.load(Ordering::SeqCst) < MAX_WORKERS
    });
    extra.refresh(host, index, || {}).unwrap();
    assert_eq!(listing(&mut extra).generation, 1);
    for release in releases {
        release.send(()).unwrap();
    }
    for list in &mut lists {
        let _ = listing(list);
    }
    wait_until("every worker to end", || POOL.load(Ordering::SeqCst) == 0);
}

#[test]
fn nothing_shows_after_the_list_is_dropped() {
    static POOL: AtomicUsize = AtomicUsize::new(0);
    let host = FakeHost::with(vec![Some(epoch(1))], Ok(listed(vec![info(1)])));
    let (release, started) = host.gated();
    let ready = Arc::new(AtomicUsize::new(0));
    let mut list = TerminalList::bounded_by("fixture".into(), &POOL);
    let heard = ready.clone();
    list.refresh(host, Arc::new(FakeIndex::default()), move || {
        heard.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(list);
    release.send(()).unwrap();
    wait_until("the worker to end", || POOL.load(Ordering::SeqCst) == 0);
    assert_eq!(ready.load(Ordering::SeqCst), 0, "nobody is told");
}

#[test]
fn a_new_list_has_nothing_to_show() {
    let mut list = TerminalList::new("fixture".into());
    assert_eq!(list.generation(), 0);
    assert!(list.poll().is_none());
}
#[test]
fn a_refresh_discards_an_unread_completed_result_before_the_next_pass_finishes() {
    static POOL: AtomicUsize = AtomicUsize::new(0);
    let host = FakeHost::with(vec![Some(epoch(1))], Ok(listed(vec![info(1)])));
    let index = Arc::new(FakeIndex::default());
    let mut list = TerminalList::bounded_by("fixture".into(), &POOL);
    let (ready, heard) = mpsc::channel();
    list.refresh(host.clone(), index.clone(), move || {
        let _ = ready.send(());
    })
    .unwrap();
    heard.recv_timeout(Duration::from_secs(5)).unwrap();
    let (release, started) = host.gated();
    list.refresh(host.clone(), index, || {}).unwrap();
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        list.poll().is_none(),
        "the prior generation must no longer be visible"
    );
    release.send(()).unwrap();
    assert_eq!(listing(&mut list).generation, 2);
}
