//! Opening a terminal on a host, on a worker of its own: never on an egui frame, never an RPC
//! from a `Drop` on the UI thread.
//!
//! A new terminal goes: capacity (`Permit`, before any network) -> the live epoch ->
//! reserved persistence capacity -> a record with its deduplication key, encrypted, before
//! anything is sent -> `create_keyed` -> lease promotion queued in the app-owned keeper ->
//! handed to the UI through `host_handoff`. A seat retains reports across that handoff.
//! The unmanaged fixture path completes the record directly on its worker. An existing
//! terminal (after a restart of the desktop) goes: capacity -> same epoch -> `attach_fresh`.
//!
//! Nothing ends a program unless the person cancelled explicitly, the program was created by
//! this very opening, its id is known and the create was unambiguous. A dropped pane, a timed
//! out handoff or an existing terminal only detach, and the lease stays saved. An ambiguous
//! create keeps its pending record and key for a retry in the same run (`Retry`); after a
//! restart, the host's terminals are listed and attached explicitly instead.
use super::host_handoff::{self, Unclaimed};
use super::host_lease_keeper::{Keeper, Place};
use super::host_worker::Permit;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tessel_host_client::terminal::TerminalEvent;
use tessel_host_protocol::pty::Size;
use tessel_host_protocol::{DedupKey, Epoch, LeaseKey, TerminalId};

/// How long the UI has to adopt an opened terminal before it is detached (and kept).
pub const ADOPT_TIMEOUT: Duration = Duration::from_secs(10);

pub type Events = Receiver<TerminalEvent>;

/// What to start on the host.
#[derive(Clone)]
pub struct Spec {
    pub alias: String,
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    pub size: Size,
}

#[derive(Clone)]
pub enum Target {
    New(Spec),
    /// A terminal known from before. `resume`: type with the lease kept in the store (read
    /// on the worker, never handed in by the UI); otherwise only watch. Never takes the lease
    /// from whoever holds it.
    Existing {
        alias: String,
        terminal: TerminalId,
        epoch: Epoch,
        resume: bool,
    },
}

/// Where a lease applies (mapped to `host_lease_store::LeaseSlot` by the adapter).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub alias: String,
    pub epoch: Epoch,
    pub terminal: TerminalId,
}

/// An application subscription retained through the worker-to-pane handoff.
/// The object is deliberately opaque: opening never sees persistence UI state.
pub trait Register: Send + Sync + 'static {
    fn seat(&self, slot: &Slot) -> Result<Box<dyn Send>, LocalError>;
}

#[derive(Clone)]
pub struct Management {
    pub keeper: Arc<Keeper>,
    pub registry: Arc<dyn Register>,
    pub openings: Openings,
}

/// The managed openings in progress, for the application to know when it can quit (with
/// `Keeper::close_admissions` and `Keeper::is_idle`). Each one is counted from before its
/// worker starts (taken synchronously by `open_managed`, `Retry::open` and `cancel_adopted`)
/// until that worker ends, its last cleanup included; a worker that cannot start lets go at
/// once. Share one per application.
#[derive(Clone, Default)]
pub struct Openings(Arc<AtomicUsize>);

impl Openings {
    pub fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }

    fn enter(&self) -> Active {
        self.0.fetch_add(1, Ordering::SeqCst);
        Active(self.0.clone())
    }
}

/// One counted opening (see `Openings`), moved into its worker and dropped with it.
struct Active(Arc<AtomicUsize>);

impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Why a host operation failed, as far as the opening needs to know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostError {
    /// The link reaches another daemon run: what the old one started is gone.
    EpochChanged,
    /// The daemon answered no. For `attach_fresh`, the terminal was not attached. For
    /// `create_keyed`, which creates then attaches, it proves nothing about the create (the
    /// attach may have been refused after it, or an earlier request still creating): the
    /// opening treats it as `Ambiguous`.
    Refused,
    /// No answer, a cut, a protocol or I/O error: something may have been started.
    Ambiguous,
}

/// A store or token operation failed. Deliberately without detail: nothing from the
/// encrypted store reaches the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalError;

/// The host operations, on the opening worker (blocking). Real: a `Client`.
pub trait Host: Clone + Send + 'static {
    type Terminal: Send + 'static;
    fn epoch(&self) -> Option<Epoch>;
    fn request_timeout(&self) -> Duration;
    /// The terminal, its events, and the lease the create granted (from the create's own
    /// reply: it stays at hand even if the terminal object lets go right away).
    fn create_keyed(
        &self,
        key: DedupKey,
        epoch: &Epoch,
        spec: &Spec,
    ) -> Result<(Self::Terminal, Events, LeaseKey), HostError>;
    fn attach_fresh(
        &self,
        terminal: &TerminalId,
        epoch: &Epoch,
        lease: Option<LeaseKey>,
    ) -> Result<(Self::Terminal, Events), HostError>;
    /// Ends a program by id and lease, without its terminal object (`close_terminal`): the
    /// object refuses once it let go. Only for a program this opening created and the person
    /// asked to stop; never for a dropped pane, a timeout or an existing terminal.
    fn close_known(
        &self,
        epoch: &Epoch,
        id: &TerminalId,
        lease: &LeaseKey,
    ) -> Result<(), HostError>;
    fn terminal_id(terminal: &Self::Terminal) -> TerminalId;
    fn lease(terminal: &Self::Terminal) -> Option<LeaseKey>;
}

/// The encrypted store, on the opening worker (blocking). Real: `LeaseStore`.
pub trait Store: Send + Sync + 'static {
    type Token: Token;
    /// Saves a pending record (with a new deduplication key) before anything is sent.
    fn reserve(&self, alias: &str, epoch: &Epoch) -> Result<Self::Token, LocalError>;
    /// Replaces the pending record by the terminal's lease, in one write.
    fn complete(&self, pending: &str, slot: &Slot, lease: &LeaseKey) -> Result<(), LocalError>;
    /// Forgets a pending record whose create is known not to have started anything.
    fn forget_pending(&self, pending: &str) -> Result<(), LocalError>;
    /// Forgets a lease whose program is known to have ended.
    fn remove(&self, slot: &Slot) -> Result<(), LocalError>;
    /// The lease kept for `slot`, if any.
    fn get(&self, slot: &Slot) -> Result<Option<LeaseKey>, LocalError>;
    /// Keeps `lease` for `slot` (an attachment that returned another lease).
    fn put(&self, slot: &Slot, lease: &LeaseKey) -> Result<(), LocalError>;
}

/// A pending record's in-process half. Real: `CreateToken`.
pub trait Token: Send + 'static {
    fn id(&self) -> &str;
    /// The deduplication key, only while it is still safe to send (same run, same epoch, in
    /// the window).
    fn key(
        &mut self,
        epoch: &Epoch,
        now: Instant,
        timeout: Duration,
    ) -> Result<DedupKey, LocalError>;
}

/// A terminal ready for the UI and the command worker (`host_worker::start`).
pub struct Opened<H: Host> {
    pub host: H,
    pub terminal: H::Terminal,
    pub events: Events,
    pub alias: String,
    pub epoch: Epoch,
    pub id: TerminalId,
    /// Started by this opening (else an existing terminal attached again).
    pub created: bool,
    /// The lease was saved before handoff. When false and `queued`, the desk owns
    /// the persistence result and any retry; do not write the lease directly.
    /// Neither this flag nor persistence grants live input authority.
    pub saved: bool,
    /// Persistence has been admitted before this value was offered to the pane.
    pub queued: bool,
    pub registration: Option<Box<dyn Send>>,
    management: Option<Management>,
    /// Pending identity at handoff. Queued work may consume it later; reconcile
    /// against the store before offering any explicit cleanup.
    pub pending: Option<String>,
    /// The lease this terminal was opened with: granted by the create, or kept (or newly
    /// returned) for an existing one. To save it again and to end the program explicitly
    /// (`Host::close_known`); never a proof that the terminal types, which only the terminal
    /// object's own lease says.
    pub recovery_lease: Option<LeaseKey>,
    pub permit: Permit,
}

/// Why an opening ended without a terminal. Fixed reasons, nothing from the host.
pub enum Failure<H: Host, S: Store> {
    /// Every terminal slot is in use; nothing was sent.
    Capacity,
    /// No slot is free for this retry; retain its original key until one is available.
    Deferred(Box<Retry<H, S>>),
    /// The host is not connected; nothing was sent.
    NotConnected,
    /// The host's daemon restarted: an existing terminal is gone, a new one was not started.
    EpochChanged,
    /// The encrypted store could not keep the request; nothing was sent.
    StoreUnavailable,
    /// The opening was cancelled. An ambiguous earlier create can still exist;
    /// cancellation of its retry preserves the pending record for explicit recovery.
    Cancelled,
    /// The existing terminal could not be attached (gone, or the link failed).
    Gone,
    /// Cancelled after the program started, and ending it was not confirmed: it runs on,
    /// detached. `Unended` says which one and holds what ending it later needs.
    CancelNotConfirmed(Box<Unended>),
    /// The terminal opened but was not adopted in time: it runs on, detached, never ended.
    /// `Unended` says which one; its lease is in the store when `saved`, else handed back.
    Detached(Box<Unended>),
    /// A retry came too late to send the same key safely: whatever the first attempt started
    /// is for the host's terminal list.
    Expired,
    /// The worker stopped without saying why.
    Stopped,
    /// The start was not confirmed (a cut, no answer). Its record and key are kept: `Retry`
    /// sends the same request again in this run while the key is still safe; otherwise list
    /// the host's terminals.
    Ambiguous(Box<Retry<H, S>>),
}

impl<H: Host, S: Store> Failure<H, S> {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Capacity => "capacity",
            Self::Deferred(_) => "capacity-retry",
            Self::NotConnected => "not-connected",
            Self::EpochChanged => "epoch-changed",
            Self::StoreUnavailable => "store-unavailable",
            Self::Cancelled => "cancelled",
            Self::Gone => "gone",
            Self::CancelNotConfirmed(_) => "cancel-not-confirmed",
            Self::Detached(_) => "detached",
            Self::Expired => "expired",
            Self::Stopped => "stopped",
            Self::Ambiguous(_) => "ambiguous",
        }
    }
}

/// A program whose end was asked but not confirmed. End it with `close_terminal(client,
/// epoch, id, lease)` from an explicit action: the lease is in the store when `saved`,
/// owned by the keeper when `queued`, otherwise retained here in `lease`.
pub struct Unended {
    pub alias: String,
    pub epoch: Epoch,
    pub id: TerminalId,
    pub saved: bool,
    pub queued: bool,
    /// Informational identity; queued promotion may already have consumed it.
    pub pending: Option<String>,
    pub lease: Option<LeaseKey>,
}

impl Unended {
    fn of<H: Host>(opened: &Opened<H>, saved: bool, pending: Option<String>) -> Self {
        Self {
            alias: opened.alias.clone(),
            epoch: opened.epoch.clone(),
            id: opened.id.clone(),
            saved,
            queued: opened.queued,
            pending: if saved { None } else { pending },
            lease: if saved || opened.queued {
                None
            } else {
                opened.recovery_lease.clone()
            },
        }
    }
}

/// A create to send again with the same key, in the same run.
pub struct Retry<H: Host, S: Store> {
    host: H,
    store: Arc<S>,
    spec: Spec,
    epoch: Epoch,
    token: S::Token,
    adopt: Duration,
    management: Option<Management>,
}

impl<H: Host, S: Store> Retry<H, S> {
    pub fn open(self, ready: impl Fn() + Send + 'static) -> Result<Pending<H, S>, Failure<H, S>> {
        let Some(permit) = Permit::acquire() else {
            return Err(Failure::Deferred(Box::new(self)));
        };
        let active = self
            .management
            .as_ref()
            .map(|management| management.openings.enter());
        let place = match &self.management {
            Some(management) => match management.keeper.reserve_new() {
                Ok(place) => Some(place),
                Err(_) => return Err(Failure::Deferred(Box::new(self))),
            },
            None => None,
        };
        let adopt = self.adopt;
        Ok(spawn(
            permit,
            adopt,
            active,
            ready,
            move |control, permit| {
                create(
                    control,
                    permit,
                    Request {
                        host: self.host,
                        store: self.store,
                        spec: self.spec,
                        epoch: self.epoch,
                        token: self.token,
                        adopt,
                        management: self.management,
                        place,
                    },
                    Attempt::Retry,
                )
            },
        ))
    }
}

pub enum Poll<H: Host, S: Store> {
    Waiting,
    Ready(Opened<H>),
    Failed(Failure<H, S>),
}

/// The UI's side of an opening. `cancel` asks to stop (and to end a program this opening
/// started); dropping it only lets go (a started program is detached and kept). Drop it once
/// `poll` answered `Ready` or `Failed`.
pub struct Pending<H: Host, S: Store> {
    handoff: host_handoff::Receiver<Opened<H>>,
    control: Arc<Control<H, S>>,
}

struct Control<H: Host, S: Store> {
    cancelled: AtomicBool,
    dropped: AtomicBool,
    failure: Mutex<Option<Failure<H, S>>>,
}

impl<H: Host, S: Store> Control<H, S> {
    /// Nobody wants a new program any more (asked to stop, or the pane went).
    fn unwanted(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst) || self.dropped.load(Ordering::SeqCst)
    }
}

impl<H: Host, S: Store> Pending<H, S> {
    pub fn poll(&self) -> Poll<H, S> {
        if let Some(opened) = self.handoff.take() {
            return Poll::Ready(opened);
        }
        let Some(stopped) = self.handoff.stopped() else {
            return Poll::Waiting;
        };
        // The worker writes its answer before it lets go of the handoff, or right after it
        // handled a cancel or a timeout.
        match (self.control.failure.lock().unwrap().take(), stopped) {
            (Some(failure), _) => Poll::Failed(failure),
            // A live receiver sees Dropped only when the worker ended: here, without an answer.
            (None, Unclaimed::Dropped) => Poll::Failed(Failure::Stopped),
            // Still cleaning up after a cancel or a timeout.
            (None, Unclaimed::Cancelled | Unclaimed::TimedOut) => Poll::Waiting,
        }
    }

    pub fn cancel(&self) {
        self.control.cancelled.store(true, Ordering::SeqCst);
        self.handoff.cancel();
    }
}

impl<H: Host, S: Store> Drop for Pending<H, S> {
    fn drop(&mut self) {
        // No RPC here: the worker sees it and detaches what it holds.
        self.control.dropped.store(true, Ordering::SeqCst);
    }
}

/// Starts opening `target`. The capacity is taken here, before any network; everything else
/// runs on the opening worker. `ready` is called (non-blocking) when a terminal waits to be
/// adopted with `Pending::poll`.
pub fn open<H: Host, S: Store>(
    host: H,
    store: Arc<S>,
    target: Target,
    ready: impl Fn() + Send + 'static,
) -> Result<Pending<H, S>, Failure<H, S>> {
    open_with(host, store, target, ADOPT_TIMEOUT, ready)
}

/// As `open`, with the time the UI has to adopt the terminal (tests use a short one).
pub fn open_with<H: Host, S: Store>(
    host: H,
    store: Arc<S>,
    target: Target,
    adopt: Duration,
    ready: impl Fn() + Send + 'static,
) -> Result<Pending<H, S>, Failure<H, S>> {
    open_internal(host, store, target, adopt, None, ready)
}

pub fn open_managed<H: Host, S: Store>(
    host: H,
    store: Arc<S>,
    target: Target,
    management: Management,
    ready: impl Fn() + Send + 'static,
) -> Result<Pending<H, S>, Failure<H, S>> {
    open_internal(host, store, target, ADOPT_TIMEOUT, Some(management), ready)
}

fn open_internal<H: Host, S: Store>(
    host: H,
    store: Arc<S>,
    target: Target,
    adopt: Duration,
    management: Option<Management>,
    ready: impl Fn() + Send + 'static,
) -> Result<Pending<H, S>, Failure<H, S>> {
    let permit = Permit::acquire().ok_or(Failure::Capacity)?;
    let active = management
        .as_ref()
        .map(|management| management.openings.enter());
    Ok(spawn(
        permit,
        adopt,
        active,
        ready,
        move |control, permit| match target {
            Target::New(spec) => {
                let Some(epoch) = host.epoch() else {
                    return Outcome::Failed(Failure::NotConnected);
                };
                if control.unwanted() {
                    return Outcome::Failed(Failure::Cancelled);
                }
                let place = match &management {
                    Some(management) => match management.keeper.reserve_new() {
                        Ok(place) => Some(place),
                        Err(_) => return Outcome::Failed(Failure::Capacity),
                    },
                    None => None,
                };
                // Saved before anything is sent: a lost reply can be recovered with the same key.
                let Ok(token) = store.reserve(&spec.alias, &epoch) else {
                    return Outcome::Failed(Failure::StoreUnavailable);
                };
                let request = Request {
                    host,
                    store,
                    spec,
                    epoch,
                    token,
                    adopt,
                    management,
                    place,
                };
                create(control, permit, request, Attempt::First)
            }
            Target::Existing {
                alias,
                terminal,
                epoch,
                resume,
            } => attach(
                control,
                permit,
                host,
                store,
                Slot {
                    alias,
                    epoch,
                    terminal,
                },
                resume,
                management,
            ),
        },
    ))
}

enum Outcome<H: Host, S: Store> {
    Offer(Box<Opened<H>>, Option<(Arc<S>, Option<String>)>),
    Failed(Failure<H, S>),
}

fn spawn<H: Host, S: Store>(
    permit: Permit,
    adopt: Duration,
    active: Option<Active>,
    ready: impl Fn() + Send + 'static,
    work: impl FnOnce(&Control<H, S>, Permit) -> Outcome<H, S> + Send + 'static,
) -> Pending<H, S> {
    let (sender, handoff) = host_handoff::channel();
    let control = Arc::new(Control {
        cancelled: AtomicBool::new(false),
        dropped: AtomicBool::new(false),
        failure: Mutex::new(None),
    });
    let worker = control.clone();
    let spawned = std::thread::Builder::new()
        .name("host-opening".into())
        .spawn(move || {
            // Counted until this worker ends, its cleanup included.
            let _active = active;
            match work(&worker, permit) {
                Outcome::Offer(opened, cleanup) => {
                    if let Err((reason, opened)) = sender.offer(*opened, adopt, &ready) {
                        let failure = unclaimed(reason, opened, cleanup);
                        let answered = failure.is_some();
                        *worker.failure.lock().unwrap() = failure;
                        if answered {
                            ready();
                        }
                    }
                }
                Outcome::Failed(failure) => {
                    // Written before the sender goes, so `poll` reads it once it sees the end.
                    *worker.failure.lock().unwrap() = Some(failure);
                    drop(sender);
                    ready();
                }
            }
        });
    if spawned.is_err() {
        *control.failure.lock().unwrap() = Some(Failure::Stopped);
    }
    Pending { handoff, control }
}

/// The UI did not adopt the terminal. Only an explicit cancel ends a program this opening
/// started (id known, create unambiguous); anything else detaches it, with its lease kept.
fn unclaimed<H: Host, S: Store>(
    reason: Unclaimed,
    opened: Opened<H>,
    cleanup: Option<(Arc<S>, Option<String>)>,
) -> Option<Failure<H, S>> {
    if reason == Unclaimed::TimedOut {
        // Detached (the terminal goes with `opened`), never ended; the UI is told which one,
        // with its lease when it could not be saved. A pending record stays on disk.
        return Some(Failure::Detached(Box::new(Unended::of(
            &opened,
            opened.saved,
            opened.pending.clone(),
        ))));
    }
    match (reason, opened.created, cleanup) {
        (Unclaimed::Cancelled, true, Some((store, pending))) => {
            let slot = Slot {
                alias: opened.alias.clone(),
                epoch: opened.epoch.clone(),
                terminal: opened.id.clone(),
            };
            let reservation = match &opened.management {
                Some(management) => match management.keeper.reserve(slot.clone()) {
                    Ok(reservation) => Some(reservation),
                    Err(_) => {
                        return Some(Failure::CancelNotConfirmed(Box::new(Unended::of(
                            &opened,
                            opened.saved,
                            pending,
                        ))));
                    }
                },
                None => None,
            };
            let closed = match &opened.recovery_lease {
                Some(lease) => opened.host.close_known(&slot.epoch, &slot.terminal, lease),
                None => Err(HostError::Ambiguous),
            };
            match closed {
                Ok(()) => {
                    if let Some(reservation) = reservation {
                        let _ = reservation.forget(super::host_lease_keeper::Ended::CloseConfirmed);
                    } else {
                        let _ = store.remove(&slot);
                    }
                    if let Some(pending) = pending {
                        let _ = store.forget_pending(&pending);
                    }
                    Some(Failure::Cancelled)
                }
                Err(_) => {
                    // Kept, never a program without its lease: saved now if it was not yet,
                    // else handed back to the UI.
                    let saved = opened.saved
                        || !opened.queued
                            && match (&pending, &opened.recovery_lease) {
                                (Some(pending), Some(lease)) => {
                                    store.complete(pending, &slot, lease).is_ok()
                                }
                                _ => false,
                            };
                    Some(Failure::CancelNotConfirmed(Box::new(Unended::of(
                        &opened, saved, pending,
                    ))))
                }
            }
        }
        // A dropped pane, or an existing terminal: detach only (the terminal goes with
        // `opened`), the lease stays saved.
        _ => (reason == Unclaimed::Cancelled).then_some(Failure::Cancelled),
    }
}

/// Cancels a terminal the UI adopted (`Poll::Ready`) but whose commands it never started (the
/// lease keeper had no room before `host_worker::start`), with the same cleanup as a cancel
/// before adoption: a program this opening created is ended (`Host::close_known`) and, once
/// that is confirmed, its lease and pending record are forgotten; unconfirmed, it is kept with
/// its lease (`CancelNotConfirmed`: saved now if it can be, else handed back). An existing
/// terminal is only detached. It may send an RPC and write the store, so it runs on a worker of
/// its own; the permit stays held until the cleanup is done.
///
/// A managed opening reserves ordered cleanup before asking the host to close.
/// An unmanaged opening performs its store cleanup directly on this worker.
///
/// One answer comes on the returned receiver, then `ready` is called. If nobody waits for it
/// any more, the answer is dropped and the cleanup still runs to its end. If no worker can
/// start, nothing is done and the terminal comes back whole (`Err`), with its lease. A managed
/// terminal's cleanup is counted in its `Openings` until it is done.
pub fn cancel_adopted<H: Host, S: Store>(
    opened: Opened<H>,
    store: Arc<S>,
    ready: impl Fn() + Send + 'static,
) -> Result<Receiver<Failure<H, S>>, Box<Opened<H>>> {
    let (answer, answers) = sync_channel(1);
    let active = opened
        .management
        .as_ref()
        .map(|management| management.openings.enter());
    // Handed over through a shared place: a worker that cannot start drops its closure, and
    // the terminal is then taken back from here instead of being lost with it.
    let handover = Arc::new(Mutex::new(Some(opened)));
    let worker = handover.clone();
    let spawned = std::thread::Builder::new()
        .name("host-opening-cancel".into())
        .spawn(move || {
            // Counted until the cleanup is done (a worker that cannot start lets go at once).
            let _active = active;
            let Some(opened) = worker.lock().unwrap().take() else {
                return;
            };
            let pending = opened.pending.clone();
            let failure = unclaimed(Unclaimed::Cancelled, opened, Some((store, pending)))
                .unwrap_or(Failure::Cancelled);
            let _ = answer.send(failure);
            ready();
        });
    match spawned {
        Ok(_) => Ok(answers),
        Err(_) => Err(Box::new(
            handover
                .lock()
                .unwrap()
                .take()
                .expect("a worker that never started never took it"),
        )),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Attempt {
    First,
    Retry,
}

/// Everything a create needs, kept together for a `Retry`.
struct Request<H: Host, S: Store> {
    host: H,
    store: Arc<S>,
    spec: Spec,
    epoch: Epoch,
    token: S::Token,
    adopt: Duration,
    management: Option<Management>,
    place: Option<Place>,
}

fn create<H: Host, S: Store>(
    control: &Control<H, S>,
    permit: Permit,
    request: Request<H, S>,
    attempt: Attempt,
) -> Outcome<H, S> {
    let Request {
        host,
        store,
        spec,
        epoch,
        mut token,
        adopt,
        management,
        place,
    } = request;
    if control.unwanted() {
        // Before a first send nothing can have started; after an earlier attempt something
        // may have, so its record stays for the host's list.
        if attempt == Attempt::First {
            let _ = store.forget_pending(token.id());
        }
        return Outcome::Failed(Failure::Cancelled);
    }
    match host.epoch() {
        // Another daemon run: the one the record was for is gone with what it started.
        Some(current) if current != epoch => {
            let _ = store.forget_pending(token.id());
            return Outcome::Failed(Failure::EpochChanged);
        }
        Some(_) => {}
        // Not connected proves nothing about a restart.
        None => {
            return Outcome::Failed(match attempt {
                Attempt::First => {
                    let _ = store.forget_pending(token.id());
                    Failure::NotConnected
                }
                Attempt::Retry => Failure::Ambiguous(Box::new(Retry {
                    host,
                    store,
                    spec,
                    epoch,
                    token,
                    adopt,
                    management,
                })),
            });
        }
    }
    let Ok(key) = token.key(&epoch, Instant::now(), host.request_timeout()) else {
        return Outcome::Failed(match attempt {
            // Nothing was ever sent with it.
            Attempt::First => {
                let _ = store.forget_pending(token.id());
                Failure::Stopped
            }
            // Too late to send it again safely: what it started, if anything, is for the list.
            Attempt::Retry => Failure::Expired,
        });
    };
    // Checked again right before sending: a cancel or a closed pane may have come while the
    // epoch or the key was read.
    if control.unwanted() {
        if attempt == Attempt::First {
            let _ = store.forget_pending(token.id());
        }
        return Outcome::Failed(Failure::Cancelled);
    }
    let pending = token.id().to_owned();
    match host.create_keyed(key, &epoch, &spec) {
        Ok((terminal, events, granted)) => {
            let id = H::terminal_id(&terminal);
            let slot = Slot {
                alias: spec.alias.clone(),
                epoch: epoch.clone(),
                terminal: id.clone(),
            };
            let registration = management
                .as_ref()
                .map(|managed| managed.registry.seat(&slot))
                .transpose();
            let registration_failed = registration.is_err();
            // Managed openings always hand off the creation lease using the place
            // reserved before the RPC. No pane drop can discard an unsaved lease.
            let (saved, queued) = if let Some(place) = place {
                place.promote(slot.clone(), pending.clone(), granted.clone());
                (false, true)
            } else {
                (store.complete(&pending, &slot, &granted).is_ok(), false)
            };
            // A pending record that could not be replaced stays, so it can still be completed.
            let still_pending = (!saved).then_some(pending);
            let opened = Opened {
                host,
                terminal,
                events,
                alias: spec.alias,
                epoch,
                id,
                created: true,
                saved,
                queued,
                registration: registration.ok().flatten(),
                management,
                pending: still_pending.clone(),
                recovery_lease: Some(granted),
                permit,
            };
            if registration_failed {
                return Outcome::Failed(Failure::Detached(Box::new(Unended::of(
                    &opened,
                    saved,
                    still_pending,
                ))));
            }
            if control.cancelled.load(Ordering::SeqCst) {
                // Asked to stop while it started: end it here, on the worker.
                let failure = unclaimed(Unclaimed::Cancelled, opened, Some((store, still_pending)));
                return Outcome::Failed(failure.unwrap_or(Failure::Cancelled));
            }
            Outcome::Offer(Box::new(opened), Some((store, still_pending)))
        }
        Err(HostError::EpochChanged) => {
            let _ = store.forget_pending(&pending);
            Outcome::Failed(Failure::EpochChanged)
        }
        // Refused may come from the attach after the create: as unsure as no answer.
        Err(HostError::Refused | HostError::Ambiguous) => {
            Outcome::Failed(Failure::Ambiguous(Box::new(Retry {
                host,
                store,
                spec,
                epoch,
                token,
                adopt,
                management,
            })))
        }
    }
}

fn attach<H: Host, S: Store>(
    control: &Control<H, S>,
    permit: Permit,
    host: H,
    store: Arc<S>,
    slot: Slot,
    resume: bool,
    management: Option<Management>,
) -> Outcome<H, S> {
    if control.unwanted() {
        return Outcome::Failed(Failure::Cancelled);
    }
    match host.epoch() {
        None => return Outcome::Failed(Failure::NotConnected),
        Some(current) if current != slot.epoch => return Outcome::Failed(Failure::EpochChanged),
        Some(_) => {}
    }
    let reservation = match &management {
        Some(management) => match management.keeper.reserve(slot.clone()) {
            Ok(reservation) => Some(reservation),
            Err(_) => return Outcome::Failed(Failure::Capacity),
        },
        None => None,
    };
    let registration = match management
        .as_ref()
        .map(|managed| managed.registry.seat(&slot))
        .transpose()
    {
        Ok(registration) => registration,
        Err(_) => return Outcome::Failed(Failure::Capacity),
    };
    // The lease is read here, on the worker; a store that cannot be read stops the resume
    // rather than attaching another way.
    let kept = if resume {
        match store.get(&slot) {
            Ok(lease) => lease,
            Err(LocalError) => return Outcome::Failed(Failure::StoreUnavailable),
        }
    } else {
        None
    };
    match host.attach_fresh(&slot.terminal, &slot.epoch, kept.clone()) {
        Ok((attached, events)) => {
            let returned = H::lease(&attached);
            let mut queued = false;
            let saved = match &returned {
                Some(lease) if Some(lease) != kept.as_ref() => {
                    if let Some(reservation) = reservation {
                        reservation.keep(lease.clone());
                        queued = true;
                        false
                    } else {
                        store.put(&slot, lease).is_ok()
                    }
                }
                _ => true,
            };
            let recovery_lease = returned.or(kept);
            Outcome::Offer(
                Box::new(Opened {
                    host,
                    terminal: attached,
                    events,
                    alias: slot.alias,
                    epoch: slot.epoch,
                    id: slot.terminal,
                    created: false,
                    saved,
                    queued,
                    registration,
                    management,
                    pending: None,
                    recovery_lease,
                    permit,
                }),
                None,
            )
        }
        Err(HostError::EpochChanged) => Outcome::Failed(Failure::EpochChanged),
        Err(HostError::Refused) => Outcome::Failed(Failure::Gone),
        Err(HostError::Ambiguous) => Outcome::Failed(Failure::NotConnected),
    }
}
