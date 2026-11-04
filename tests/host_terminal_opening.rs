//! Opening a native terminal, with a synthetic host and store (no server, no encrypted file):
//! capacity before any network, the pending record before any create, the lease saved before
//! the UI adopts, and what each failure, cancel, drop or timeout leaves behind.
#[allow(dead_code)]
#[path = "../src/terminal/host_commands.rs"]
mod host_commands;
#[allow(dead_code)]
#[path = "../src/terminal/host_handoff.rs"]
mod host_handoff;
#[allow(dead_code)]
#[path = "../src/terminal/host_opening.rs"]
mod host_opening;
#[allow(dead_code)]
#[path = "../src/terminal/host_worker.rs"]
mod host_worker;

use host_opening::{
    Failure, Host, HostError, LocalError, Opened, Pending, Poll, Slot, Spec, Store, Target, Token,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tessel_host_protocol::pty::Size;
use tessel_host_protocol::{DedupKey, Epoch, LeaseKey, TerminalId};

/// Terminal slots are process-wide: one test at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const ALIAS: &str = "fixture";

fn epoch(n: u8) -> Epoch {
    Epoch::from_bytes([n; 16])
}

// ---- A synthetic host -------------------------------------------------------------------

struct FakeTerminal {
    id: TerminalId,
    lease: Option<LeaseKey>,
}

#[derive(Default)]
struct HostState {
    epoch: Option<Epoch>,
    /// Results of successive creates (Ok: a new terminal); empty means Ok.
    creates: Vec<Result<(), HostError>>,
    keys: Vec<DedupKey>,
    attach: Option<Result<Option<LeaseKey>, HostError>>,
    attached_with: Vec<Option<LeaseKey>>,
    close: Option<Result<(), HostError>>,
    closes: usize,
    /// The created terminal object already let go (a snapshot it refused): its own lease is
    /// None, the create's answer still carries the granted one.
    let_go_on_create: bool,
    /// Programs ended by id and lease.
    closed: Vec<(TerminalId, LeaseKey)>,
    /// When set, a create waits here until the test lets it go.
    gate: Option<Receiver<()>>,
    /// Told when a create started.
    started: Option<Sender<()>>,
}

#[derive(Clone)]
struct FakeHost(Arc<Mutex<HostState>>);

impl FakeHost {
    fn connected() -> Self {
        Self(Arc::new(Mutex::new(HostState {
            epoch: Some(epoch(1)),
            ..HostState::default()
        })))
    }

    fn state(&self) -> MutexGuard<'_, HostState> {
        self.0.lock().unwrap()
    }

    /// Creates wait until the returned sender lets them go; the receiver hears when one starts.
    fn gated(&self) -> (Sender<()>, Receiver<()>) {
        let (release, gate) = mpsc::channel();
        let (started, heard) = mpsc::channel();
        let mut state = self.state();
        state.gate = Some(gate);
        state.started = Some(started);
        (release, heard)
    }
}

fn events() -> host_opening::Events {
    mpsc::sync_channel(1).1
}

impl Host for FakeHost {
    type Terminal = FakeTerminal;

    fn epoch(&self) -> Option<Epoch> {
        self.state().epoch.clone()
    }

    fn request_timeout(&self) -> Duration {
        Duration::from_secs(15)
    }

    fn create_keyed(
        &self,
        key: DedupKey,
        _epoch: &Epoch,
        _spec: &Spec,
    ) -> Result<(FakeTerminal, host_opening::Events, LeaseKey), HostError> {
        let (gate, started) = {
            let mut state = self.state();
            state.keys.push(key);
            (state.gate.take(), state.started.take())
        };
        if let Some(started) = started {
            let _ = started.send(());
        }
        if let Some(gate) = gate {
            let _ = gate.recv_timeout(Duration::from_secs(5));
        }
        let mut state = self.state();
        let result = if state.creates.is_empty() {
            Ok(())
        } else {
            state.creates.remove(0)
        };
        result.map(|()| {
            let n = state.keys.len() as u8;
            let granted = LeaseKey::from_bytes([0x80 + n; 16]);
            (
                FakeTerminal {
                    id: TerminalId::from_bytes([0x40 + n; 16]),
                    lease: (!state.let_go_on_create).then(|| granted.clone()),
                },
                events(),
                granted,
            )
        })
    }

    fn attach_fresh(
        &self,
        terminal: &TerminalId,
        _epoch: &Epoch,
        lease: Option<LeaseKey>,
    ) -> Result<(FakeTerminal, host_opening::Events), HostError> {
        let mut state = self.state();
        state.attached_with.push(lease.clone());
        let granted = state.attach.clone().unwrap_or(Ok(lease))?;
        Ok((
            FakeTerminal {
                id: terminal.clone(),
                lease: granted,
            },
            events(),
        ))
    }

    fn close_known(
        &self,
        _epoch: &Epoch,
        id: &TerminalId,
        lease: &LeaseKey,
    ) -> Result<(), HostError> {
        let mut state = self.state();
        state.closes += 1;
        state.closed.push((id.clone(), lease.clone()));
        state.close.unwrap_or(Ok(()))
    }

    fn terminal_id(terminal: &FakeTerminal) -> TerminalId {
        terminal.id.clone()
    }

    fn lease(terminal: &FakeTerminal) -> Option<LeaseKey> {
        terminal.lease.clone()
    }
}

// ---- A synthetic store ------------------------------------------------------------------

#[derive(Default)]
struct StoreState {
    pending: HashMap<String, DedupKey>,
    leases: Vec<(Slot, LeaseKey)>,
    fail_reserve: bool,
    fail_complete: bool,
    fail_get: bool,
    reserves: usize,
    /// Keys a token still hands out (counts down; the key is refused at 0).
    key_uses: Option<usize>,
}

#[derive(Default)]
struct FakeStore(Mutex<StoreState>);

impl FakeStore {
    fn state(&self) -> MutexGuard<'_, StoreState> {
        self.0.lock().unwrap()
    }

    fn lease(&self, slot: &Slot) -> Option<LeaseKey> {
        self.state()
            .leases
            .iter()
            .find(|(s, _)| s == slot)
            .map(|(_, l)| l.clone())
    }
}

struct FakeToken {
    id: String,
    key: DedupKey,
    uses: Arc<Mutex<Option<usize>>>,
}

impl Token for FakeToken {
    fn id(&self) -> &str {
        &self.id
    }

    fn key(
        &mut self,
        _epoch: &Epoch,
        _now: Instant,
        _timeout: Duration,
    ) -> Result<DedupKey, LocalError> {
        let mut uses = self.uses.lock().unwrap();
        match uses.as_mut() {
            Some(0) => Err(LocalError),
            Some(n) => {
                *n -= 1;
                Ok(self.key.clone())
            }
            None => Ok(self.key.clone()),
        }
    }
}

impl Store for FakeStore {
    type Token = FakeToken;

    fn reserve(&self, _alias: &str, _epoch: &Epoch) -> Result<FakeToken, LocalError> {
        let mut state = self.state();
        if state.fail_reserve {
            return Err(LocalError);
        }
        state.reserves += 1;
        let id = format!("pending-{}", state.reserves);
        let key = DedupKey::from_bytes([state.reserves as u8; 16]);
        state.pending.insert(id.clone(), key.clone());
        Ok(FakeToken {
            id,
            key,
            uses: Arc::new(Mutex::new(state.key_uses)),
        })
    }

    fn complete(&self, pending: &str, slot: &Slot, lease: &LeaseKey) -> Result<(), LocalError> {
        let mut state = self.state();
        if state.fail_complete || state.pending.remove(pending).is_none() {
            return Err(LocalError);
        }
        state.leases.push((slot.clone(), lease.clone()));
        Ok(())
    }

    fn forget_pending(&self, pending: &str) -> Result<(), LocalError> {
        self.state().pending.remove(pending);
        Ok(())
    }

    fn remove(&self, slot: &Slot) -> Result<(), LocalError> {
        self.state().leases.retain(|(s, _)| s != slot);
        Ok(())
    }

    fn get(&self, slot: &Slot) -> Result<Option<LeaseKey>, LocalError> {
        if self.state().fail_get {
            return Err(LocalError);
        }
        Ok(self.lease(slot))
    }

    fn put(&self, slot: &Slot, lease: &LeaseKey) -> Result<(), LocalError> {
        let mut state = self.state();
        state.leases.retain(|(s, _)| s != slot);
        state.leases.push((slot.clone(), lease.clone()));
        Ok(())
    }
}

// ---- Helpers ----------------------------------------------------------------------------

fn spec() -> Spec {
    Spec {
        alias: ALIAS.into(),
        argv: vec!["sh".into()],
        cwd: None,
        env: BTreeMap::new(),
        size: Size { rows: 24, cols: 80 },
    }
}

type Opening = Pending<FakeHost, FakeStore>;

fn open_new(host: &FakeHost, store: &Arc<FakeStore>, adopt: Duration) -> (Opening, Receiver<()>) {
    let (ready, heard) = mpsc::channel();
    let ready = Mutex::new(ready);
    let pending = host_opening::open_with(
        host.clone(),
        store.clone(),
        Target::New(spec()),
        adopt,
        move || {
            let _ = ready.lock().unwrap().send(());
        },
    )
    .unwrap_or_else(|failure| panic!("refused at once: {}", failure.code()));
    (pending, heard)
}

enum Settled {
    Ready(Opened<FakeHost>),
    Failed(Failure<FakeHost, FakeStore>),
}

fn settle(pending: &Opening) -> Settled {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match pending.poll() {
            Poll::Ready(opened) => return Settled::Ready(opened),
            Poll::Failed(failure) => return Settled::Failed(failure),
            Poll::Waiting => {
                assert!(Instant::now() < deadline, "the opening never settled");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

fn failed(pending: &Opening) -> Failure<FakeHost, FakeStore> {
    match settle(pending) {
        Settled::Failed(failure) => failure,
        Settled::Ready(_) => panic!("opened, expected a failure"),
    }
}

fn opened(pending: &Opening) -> Opened<FakeHost> {
    match settle(pending) {
        Settled::Ready(opened) => opened,
        Settled::Failed(failure) => panic!("failed: {}", failure.code()),
    }
}

fn slot_of(opened: &Opened<FakeHost>) -> Slot {
    Slot {
        alias: opened.alias.clone(),
        epoch: opened.epoch.clone(),
        terminal: opened.id.clone(),
    }
}

/// Waits until `done`, for what a worker finishes after the UI let go.
fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

// ---- New terminals ----------------------------------------------------------------------

#[test]
fn a_new_terminal_is_saved_before_it_is_adopted() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    let (pending, _ready) = open_new(&host, &store, Duration::from_secs(5));
    let opened = opened(&pending);
    assert!(opened.created && opened.saved && opened.pending.is_none());
    let slot = slot_of(&opened);
    assert_eq!(store.lease(&slot), FakeHost::lease(&opened.terminal));
    assert!(
        store.state().pending.is_empty(),
        "the record became the lease"
    );
    assert_eq!(
        host.state().keys,
        [DedupKey::from_bytes([1; 16])],
        "the record's key"
    );
}

#[test]
fn no_free_slot_refuses_before_any_network() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    let held: Vec<_> = std::iter::from_fn(host_worker::Permit::acquire).collect();
    assert!(!held.is_empty());
    let refused = host_opening::open(host.clone(), store.clone(), Target::New(spec()), || {});
    assert!(matches!(refused, Err(Failure::Capacity)));
    assert_eq!(store.state().reserves, 0);
    assert!(host.state().keys.is_empty());
    drop(held);
}

#[test]
fn a_full_worker_pool_preserves_the_original_retry_key() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    host.state().creates = vec![Err(HostError::Ambiguous)];
    let (pending, _) = open_new(&host, &store, Duration::from_secs(5));
    let Failure::Ambiguous(retry) = failed(&pending) else {
        panic!("expected ambiguous");
    };
    drop(pending);
    let held: Vec<_> = std::iter::from_fn(host_worker::Permit::acquire).collect();
    let Err(Failure::Deferred(retry)) = retry.open(|| {}) else {
        panic!("capacity must preserve the retry");
    };
    assert_eq!(host.state().keys.len(), 1);
    assert_eq!(store.state().pending.len(), 1);
    drop(held);
    let again = retry
        .open(|| {})
        .unwrap_or_else(|failure| panic!("{}", failure.code()));
    assert!(opened(&again).saved);
    let keys = host.state().keys.clone();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0], keys[1]);
}

#[test]
fn nothing_is_sent_without_a_connection_or_a_saved_record() {
    let _serial = serial();
    let store = Arc::new(FakeStore::default());
    let offline = FakeHost::connected();
    offline.state().epoch = None;
    let (pending, _) = open_new(&offline, &store, Duration::from_secs(5));
    assert!(matches!(failed(&pending), Failure::NotConnected));
    assert_eq!(store.state().reserves, 0);

    let host = FakeHost::connected();
    store.state().fail_reserve = true;
    let (pending, _) = open_new(&host, &store, Duration::from_secs(5));
    assert!(matches!(failed(&pending), Failure::StoreUnavailable));
    assert!(host.state().keys.is_empty(), "no create without its record");
}

#[test]
fn a_restarted_daemon_forgets_the_record() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    host.state().creates = vec![Err(HostError::EpochChanged)];
    let (pending, _) = open_new(&host, &store, Duration::from_secs(5));
    assert!(matches!(failed(&pending), Failure::EpochChanged));
    assert!(store.state().pending.is_empty());
}

#[test]
fn a_refused_or_unanswered_create_keeps_its_record_and_retries_with_the_same_key() {
    let _serial = serial();
    for first in [HostError::Refused, HostError::Ambiguous] {
        let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
        host.state().creates = vec![Err(first)];
        let (pending, _) = open_new(&host, &store, Duration::from_secs(5));
        let Failure::Ambiguous(retry) = failed(&pending) else {
            panic!("{first:?} is not proof that nothing started");
        };
        assert_eq!(store.state().pending.len(), 1, "the record stays");
        drop(pending);

        let (ready, _heard) = mpsc::channel::<()>();
        let ready = Mutex::new(ready);
        let again = retry
            .open(move || {
                let _ = ready.lock().unwrap().send(());
            })
            .unwrap_or_else(|failure| panic!("{}", failure.code()));
        let opened = opened(&again);
        assert!(opened.saved);
        let keys = host.state().keys.clone();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0], keys[1], "the same key, never a new one");
        assert!(store.state().pending.is_empty());
    }
}

#[test]
fn a_retry_past_the_safe_window_is_expired_and_keeps_the_record() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    host.state().creates = vec![Err(HostError::Ambiguous)];
    store.state().key_uses = Some(1);
    let (pending, _) = open_new(&host, &store, Duration::from_secs(5));
    let Failure::Ambiguous(retry) = failed(&pending) else {
        panic!("expected ambiguous");
    };
    let again = retry
        .open(|| {})
        .unwrap_or_else(|failure| panic!("{}", failure.code()));
    assert!(matches!(failed(&again), Failure::Expired));
    assert_eq!(host.state().keys.len(), 1, "never sent again");
    assert_eq!(store.state().pending.len(), 1, "left for the host's list");
}

#[test]
fn a_lease_that_cannot_be_saved_keeps_the_program_and_says_so() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    store.state().fail_complete = true;
    let (pending, _) = open_new(&host, &store, Duration::from_secs(5));
    let opened = opened(&pending);
    assert!(!opened.saved);
    assert_eq!(
        opened.pending.as_deref(),
        Some("pending-1"),
        "for saving again"
    );
    assert_eq!(host.state().closes, 0, "never ended for a storage failure");
    assert_eq!(store.state().pending.len(), 1);
}

// ---- Cancel, drop, timeout --------------------------------------------------------------

#[test]
fn cancelling_while_it_starts_ends_the_new_program_and_forgets_it() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    let (release, started) = host.gated();
    let (pending, _) = open_new(&host, &store, Duration::from_secs(5));
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    pending.cancel();
    release.send(()).unwrap();
    assert!(matches!(failed(&pending), Failure::Cancelled));
    assert_eq!(host.state().closes, 1);
    assert_eq!(
        host.state().closed[0].1,
        LeaseKey::from_bytes([0x81; 16]),
        "ended with the lease the create granted"
    );
    assert!(store.state().leases.is_empty() && store.state().pending.is_empty());
}

#[test]
fn a_terminal_that_let_go_at_once_still_saves_and_ends_with_its_creation_lease() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    host.state().let_go_on_create = true;
    let (pending, _) = open_new(&host, &store, Duration::from_secs(5));
    let opened = opened(&pending);
    let granted = LeaseKey::from_bytes([0x81; 16]);
    assert!(opened.saved, "saved with the granted lease");
    assert_eq!(
        FakeHost::lease(&opened.terminal),
        None,
        "the object types no more"
    );
    assert_eq!(opened.recovery_lease, Some(granted.clone()));
    assert_eq!(store.lease(&slot_of(&opened)), Some(granted.clone()));
    drop(opened);

    // Cancelled while it started: ended by id with that lease, though the object let go.
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    host.state().let_go_on_create = true;
    let (release, started) = host.gated();
    let (pending, _) = open_new(&host, &store, Duration::from_secs(5));
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    pending.cancel();
    release.send(()).unwrap();
    assert!(matches!(failed(&pending), Failure::Cancelled));
    assert_eq!(
        host.state().closed,
        [(TerminalId::from_bytes([0x41; 16]), granted)]
    );
}

#[test]
fn an_unconfirmed_end_keeps_the_program_identified_with_its_lease() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    host.state().close = Some(Err(HostError::Ambiguous));
    let (release, started) = host.gated();
    let (pending, _) = open_new(&host, &store, Duration::from_secs(5));
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    pending.cancel();
    release.send(()).unwrap();
    let Failure::CancelNotConfirmed(unended) = failed(&pending) else {
        panic!("expected an unconfirmed end");
    };
    assert!(unended.saved && unended.lease.is_none());
    let slot = Slot {
        alias: unended.alias.clone(),
        epoch: unended.epoch.clone(),
        terminal: unended.id.clone(),
    };
    assert!(
        store.lease(&slot).is_some(),
        "its lease is kept for ending it later"
    );
}

#[test]
fn an_unconfirmed_end_with_an_unsaved_lease_hands_the_lease_back() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    host.state().close = Some(Err(HostError::Ambiguous));
    store.state().fail_complete = true;
    let (release, started) = host.gated();
    let (pending, _) = open_new(&host, &store, Duration::from_secs(5));
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    pending.cancel();
    release.send(()).unwrap();
    let Failure::CancelNotConfirmed(unended) = failed(&pending) else {
        panic!("expected an unconfirmed end");
    };
    assert!(!unended.saved);
    assert!(
        unended.lease.is_some() && unended.pending.is_some(),
        "never lost"
    );
}

#[test]
fn closing_the_pane_while_it_starts_detaches_and_keeps_the_program() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    let (release, started) = host.gated();
    let (pending, _) = open_new(&host, &store, Duration::from_secs(5));
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(pending);
    release.send(()).unwrap();
    eventually("the lease to be saved", || store.state().leases.len() == 1);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(host.state().closes, 0, "a dropped pane ends nothing");
}

#[test]
fn closing_the_pane_before_anything_is_sent_starts_nothing() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    // The worker reads the epoch first: hold the host there until the pane is gone.
    let guard = host.state();
    let (pending, _) = {
        let (ready, heard) = mpsc::channel::<()>();
        let ready = Mutex::new(ready);
        let pending = host_opening::open_with(
            host.clone(),
            store.clone(),
            Target::New(spec()),
            Duration::from_secs(5),
            move || {
                let _ = ready.lock().unwrap().send(());
            },
        )
        .unwrap_or_else(|failure| panic!("{}", failure.code()));
        (pending, heard)
    };
    drop(pending);
    drop(guard);
    std::thread::sleep(Duration::from_millis(200));
    assert!(host.state().keys.is_empty(), "no program nobody wants");
    assert!(store.state().pending.is_empty());
}

#[test]
fn cancelling_before_adopting_ends_only_a_new_program() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    let (pending, ready) = open_new(&host, &store, Duration::from_secs(5));
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    pending.cancel();
    assert!(matches!(failed(&pending), Failure::Cancelled));
    assert_eq!(host.state().closes, 1);
    assert!(store.state().leases.is_empty());
}

#[test]
fn not_adopting_in_time_detaches_and_keeps_the_lease() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    let (pending, _) = open_new(&host, &store, Duration::from_millis(100));
    std::thread::sleep(Duration::from_millis(400));
    let Failure::Detached(unended) = failed(&pending) else {
        panic!("expected a detached terminal");
    };
    assert!(unended.saved && unended.lease.is_none() && unended.pending.is_none());
    assert_eq!(host.state().closes, 0);
    assert_eq!(store.state().leases.len(), 1);
}

#[test]
fn not_adopting_an_unsaved_terminal_in_time_hands_back_its_identity_and_lease() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    store.state().fail_complete = true;
    let (pending, _) = open_new(&host, &store, Duration::from_millis(100));
    std::thread::sleep(Duration::from_millis(400));
    let Failure::Detached(unended) = failed(&pending) else {
        panic!("expected a detached terminal");
    };
    assert!(!unended.saved);
    assert!(unended.lease.is_some(), "never lost");
    assert_eq!(unended.pending.as_deref(), Some("pending-1"));
    assert_eq!(host.state().closes, 0, "never ended");
    assert_eq!(store.state().pending.len(), 1, "its record stays on disk");
}

/// An ambiguous first attempt, then `prepare` on the host, then the retry.
fn retry_after(prepare: impl FnOnce(&FakeHost)) -> (FakeHost, Arc<FakeStore>, Opening) {
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    host.state().creates = vec![Err(HostError::Ambiguous)];
    let (pending, _) = open_new(&host, &store, Duration::from_secs(5));
    let Failure::Ambiguous(retry) = failed(&pending) else {
        panic!("expected ambiguous");
    };
    prepare(&host);
    let again = retry
        .open(|| {})
        .unwrap_or_else(|failure| panic!("{}", failure.code()));
    (host, store, again)
}

#[test]
fn a_retry_while_disconnected_keeps_its_record_and_can_retry_again() {
    let _serial = serial();
    let (host, store, again) = retry_after(|host| host.state().epoch = None);
    let Failure::Ambiguous(retry) = failed(&again) else {
        panic!("a disconnection proves nothing: still retryable");
    };
    assert_eq!(
        host.state().keys.len(),
        1,
        "nothing sent while disconnected"
    );
    assert_eq!(store.state().pending.len(), 1);
    host.state().epoch = Some(epoch(1));
    let third = retry
        .open(|| {})
        .unwrap_or_else(|failure| panic!("{}", failure.code()));
    assert!(opened(&third).saved);
    let keys = host.state().keys.clone();
    assert_eq!((keys.len(), keys[0] == keys[1]), (2, true), "the same key");
}

#[test]
fn a_retry_on_a_restarted_daemon_forgets_its_record() {
    let _serial = serial();
    let (host, store, again) = retry_after(|host| host.state().epoch = Some(epoch(2)));
    assert!(matches!(failed(&again), Failure::EpochChanged));
    assert_eq!(host.state().keys.len(), 1, "nothing sent to the new daemon");
    assert!(store.state().pending.is_empty());
}

#[test]
fn a_cancelled_retry_keeps_its_record() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    host.state().creates = vec![Err(HostError::Ambiguous)];
    let (pending, _) = open_new(&host, &store, Duration::from_secs(5));
    let Failure::Ambiguous(retry) = failed(&pending) else {
        panic!("expected ambiguous");
    };
    // Hold the host so the retry waits before it can send, then cancel it.
    let guard = host.state();
    let again = retry
        .open(|| {})
        .unwrap_or_else(|failure| panic!("{}", failure.code()));
    again.cancel();
    drop(guard);
    assert!(matches!(failed(&again), Failure::Cancelled));
    assert_eq!(host.state().keys.len(), 1, "not sent again");
    assert_eq!(
        store.state().pending.len(),
        1,
        "the first attempt may have started something: its record stays"
    );
}

// ---- Existing terminals -----------------------------------------------------------------

fn existing(resume: bool) -> Target {
    Target::Existing {
        alias: ALIAS.into(),
        terminal: TerminalId::from_bytes([0x33; 16]),
        epoch: epoch(1),
        resume,
    }
}

fn existing_slot() -> Slot {
    Slot {
        alias: ALIAS.into(),
        epoch: epoch(1),
        terminal: TerminalId::from_bytes([0x33; 16]),
    }
}

#[test]
fn resuming_reads_the_lease_on_the_worker_and_watching_takes_nothing() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    let kept = LeaseKey::from_bytes([0x55; 16]);
    store.put(&existing_slot(), &kept).unwrap();

    let pending = host_opening::open(host.clone(), store.clone(), existing(true), || {})
        .unwrap_or_else(|failure| panic!("{}", failure.code()));
    let resumed = opened(&pending);
    assert!(!resumed.created && resumed.saved);
    assert_eq!(FakeHost::lease(&resumed.terminal), Some(kept.clone()));

    let pending = host_opening::open(host.clone(), store.clone(), existing(false), || {})
        .unwrap_or_else(|failure| panic!("{}", failure.code()));
    let watching = opened(&pending);
    assert_eq!(FakeHost::lease(&watching.terminal), None);
    assert_eq!(host.state().attached_with, [Some(kept), None]);
}

#[test]
fn an_attach_disconnect_is_retryable_and_not_reported_as_gone() {
    let _serial = serial();
    for (error, code) in [
        (HostError::Ambiguous, "not-connected"),
        (HostError::Refused, "gone"),
        (HostError::EpochChanged, "epoch-changed"),
    ] {
        let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
        host.state().attach = Some(Err(error));
        let pending = host_opening::open(host, store, existing(true), || {})
            .unwrap_or_else(|failure| panic!("{}", failure.code()));
        assert_eq!(failed(&pending).code(), code);
    }
}

#[test]
fn an_unreadable_store_stops_a_resume() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    store.state().fail_get = true;
    let pending = host_opening::open(host.clone(), store, existing(true), || {})
        .unwrap_or_else(|failure| panic!("{}", failure.code()));
    assert!(matches!(failed(&pending), Failure::StoreUnavailable));
    assert!(
        host.state().attached_with.is_empty(),
        "no attachment another way"
    );
}

#[test]
fn an_attachment_returning_another_lease_keeps_it() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    let fresh = LeaseKey::from_bytes([0x66; 16]);
    host.state().attach = Some(Ok(Some(fresh.clone())));
    let pending = host_opening::open(host, store.clone(), existing(true), || {})
        .unwrap_or_else(|failure| panic!("{}", failure.code()));
    let _opened = opened(&pending);
    assert_eq!(store.lease(&existing_slot()), Some(fresh));
}

#[test]
fn an_existing_terminal_is_never_ended_by_a_cancel_and_a_new_epoch_attaches_nothing() {
    let _serial = serial();
    let (host, store) = (FakeHost::connected(), Arc::new(FakeStore::default()));
    let (ready, heard) = mpsc::channel();
    let ready = Mutex::new(ready);
    let pending = host_opening::open_with(
        host.clone(),
        store.clone(),
        existing(false),
        Duration::from_secs(5),
        move || {
            let _ = ready.lock().unwrap().send(());
        },
    )
    .unwrap_or_else(|failure| panic!("{}", failure.code()));
    heard.recv_timeout(Duration::from_secs(5)).unwrap();
    pending.cancel();
    assert!(matches!(failed(&pending), Failure::Cancelled));
    assert_eq!(host.state().closes, 0);

    host.state().epoch = Some(epoch(2));
    let pending = host_opening::open(host.clone(), store, existing(true), || {})
        .unwrap_or_else(|failure| panic!("{}", failure.code()));
    assert!(matches!(failed(&pending), Failure::EpochChanged));
    assert_eq!(host.state().attached_with.len(), 1, "only the first attach");
}
