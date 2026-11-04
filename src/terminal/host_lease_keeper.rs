//! Keeping terminal leases in the encrypted store after an opening: a lease a takeover returned
//! is saved, a lease is forgotten only on proof that its program ended. One thread for the
//! application writes the store, in order; nobody waits for it (the UI least of all).
//!
//! Every new request, and every reservation, gets a revision, globally increasing and never
//! given twice; a retry is the same request again and keeps its revision. A slot's latest
//! accepted revision is the only one that may still write: an older request still queued is
//! skipped, and a retry token from an older failure is refused (`Superseded`), so a failed
//! save can never overwrite a newer lease, nor bring back a forgotten one. A request is
//! accepted (revision, slot and queue updated together) or refused with nothing changed.
//!
//! A reservation (`Keeper::reserve`, taken before a takeover or a close is sent) holds a queue
//! place and a revision without being a request yet: until it is committed, older requests and
//! retries for its slot still run. Committed, it is accepted unless a request newer than the
//! reservation was accepted for the slot meanwhile, even one already written: that one decides.
//! One reservation per slot at a time: two takeovers of one program are never in flight
//! together, since the host could answer them in the other order than their revisions.
//!
//! Failures are never silent: a save or a removal that failed is reported with a token to try
//! it again explicitly (`NotKept`, `NotForgotten`). Those reports are kept until read; at most
//! one per slot, so at most `MAX_SLOTS`. Informational reports are capped.
use super::host_opening::{LocalError, Slot};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, Weak};
use tessel_host_protocol::{Epoch, LeaseKey};

/// Slots followed at once (as many as the store keeps).
pub const MAX_SLOTS: usize = 256;
/// Requests waiting to be written.
pub const MAX_QUEUE: usize = 64;
/// Informational reports kept until read; beyond, the oldest informational one goes.
pub const MAX_INFO_REPORTS: usize = 64;

/// What the keeper writes to. Real: the owner's lease store, opened on the keeper's thread.
pub trait LeaseWriter: Send + Sync + 'static {
    fn put(&self, slot: &Slot, lease: &LeaseKey) -> Result<(), LocalError>;
    fn remove(&self, slot: &Slot) -> Result<(), LocalError>;
}

/// The proof a program ended: nothing else forgets its lease.
pub enum Ended {
    /// Its `Exit` event arrived.
    ExitReceived,
    /// Its `close` was answered.
    CloseConfirmed,
    /// The host answered with another epoch: the daemon that ran it restarted.
    EpochChanged { current: Epoch },
}

/// Why a request was not accepted. Nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The queue or the slot table is full: try again later.
    Busy,
    /// `EpochChanged` with the slot's own epoch proves nothing.
    InvalidProof,
    /// A newer request for this slot was accepted since: this one would undo it.
    Superseded,
    /// The keeper is shutting down.
    Closed,
    /// Every revision number was used (never in practice): nothing more is accepted.
    Exhausted,
}

/// The keeper's thread could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeeperError;

/// A lease that could not be saved, to save again with `Keeper::retry_save`. No Debug, no
/// Clone. Dropped unused, it lets its slot go.
pub struct RetrySave {
    token: Option<TokenInner>,
    lease: Option<LeaseKey>,
}

/// A lease that could not be forgotten, to try again with `Keeper::retry_forget`. Its program
/// ended: until this succeeds, the store still holds a lease for nothing. No Debug, no Clone.
pub struct RetryForget {
    token: Option<TokenInner>,
}

struct TokenInner {
    slot: Slot,
    revision: u64,
    shared: Weak<Shared>,
}

impl Drop for TokenInner {
    fn drop(&mut self) {
        // An unused token lets its slot go, if nothing newer took it meanwhile.
        if let Some(shared) = self.shared.upgrade() {
            let mut state = shared.state.lock().unwrap();
            let key = key(&self.slot);
            if state.slots.get(&key) == Some(&self.revision) {
                state.slots.remove(&key);
            }
        }
    }
}

impl RetrySave {
    pub fn slot(&self) -> &Slot {
        &self.token.as_ref().expect("a live token").slot
    }
}

impl RetryForget {
    pub fn slot(&self) -> &Slot {
        &self.token.as_ref().expect("a live token").slot
    }
}

/// A place held by `Keeper::reserve`. Commit it with `keep` or `forget`; dropped unused, it
/// gives the place back and changes nothing else. It works even after the keeper was
/// dropped: the keeper's thread waits for reservations before it ends. No Debug, no Clone.
pub struct Reservation {
    slot: Option<Slot>,
    stamp: u64,
    shared: Arc<Shared>,
}

impl Reservation {
    /// Saves the lease the takeover returned. Always accepted (the place is held); it is
    /// skipped (`Superseded`) only if a newer request for the slot was accepted since the
    /// reservation, which then decides (a confirmed end, a newer lease).
    pub fn keep(mut self, lease: LeaseKey) {
        let slot = self.slot.take().expect("a live reservation");
        commit(&self.shared, slot, self.stamp, Work::Keep(lease));
    }

    /// Forgets the lease on proof that the program ended. Refused only for a proof that
    /// proves nothing (`InvalidProof`); the reservation is then given back.
    pub fn forget(mut self, proof: Ended) -> Result<(), Refusal> {
        let slot = self.slot.as_ref().expect("a live reservation");
        if let Ended::EpochChanged { current } = &proof
            && *current == slot.epoch
        {
            return Err(Refusal::InvalidProof);
        }
        let slot = self.slot.take().expect("a live reservation");
        commit(&self.shared, slot, self.stamp, Work::Forget);
        Ok(())
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            self.shared.state.lock().unwrap().unreserve(&key(&slot));
            self.shared.changed.notify_all();
        }
    }
}

/// Turns a reservation into a job, unless a newer request for its slot was accepted since.
fn commit(shared: &Arc<Shared>, slot: Slot, stamp: u64, work: Work) {
    let superseded = {
        let mut state = shared.state.lock().unwrap();
        let key = key(&slot);
        // Checked before this reservation is given back: `accepted` goes with it.
        let superseded = state.newest(&key).is_some_and(|newest| newest > stamp);
        if !superseded {
            state.slots.insert(key.clone(), stamp);
            state.queue.push_back(Job {
                slot: slot.clone(),
                revision: stamp,
                work,
            });
        }
        state.unreserve(&key);
        superseded
    };
    shared.changed.notify_all();
    if superseded {
        deliver(shared, Report::Superseded(slot), None);
    }
}

pub enum Report {
    Kept(Slot),
    Forgotten(Slot),
    /// Skipped because a newer request for this slot was accepted.
    Superseded(Slot),
    /// The save failed: show it ("access not saved") and offer to try again.
    NotKept(RetrySave),
    /// The removal failed: show it ("an ended terminal's access is still saved") and offer to
    /// try again.
    NotForgotten(RetryForget),
}

impl Report {
    fn durable(&self) -> bool {
        matches!(self, Self::NotKept(_) | Self::NotForgotten(_))
    }

    /// A failure report's slot and revision.
    fn failure(&self) -> Option<(&Slot, u64)> {
        let token = match self {
            Self::NotKept(retry) => retry.token.as_ref(),
            Self::NotForgotten(retry) => retry.token.as_ref(),
            _ => None,
        }?;
        Some((&token.slot, token.revision))
    }
}

pub enum RetryRefused<T> {
    /// Try again later; the token is handed back.
    Busy(T),
    /// A newer request for this slot was accepted since: nothing to do.
    Superseded,
    Closed(T),
}

enum Work {
    Keep(LeaseKey),
    Forget,
}

struct Job {
    slot: Slot,
    revision: u64,
    work: Work,
}

type Key = (String, String, String);

#[derive(Default)]
struct State {
    next_revision: u64,
    /// Each followed slot's latest accepted revision.
    slots: HashMap<Key, u64>,
    queue: VecDeque<Job>,
    /// Queue places held by reservations not committed yet.
    reserved: usize,
    /// Slots with a reservation (at most one each): a reserved slot counts as followed.
    reservations: HashSet<Key>,
    /// For reserved slots only: the newest revision accepted for them, kept even once it is
    /// written and gone from `slots` (or its retry token dropped), so a reservation still
    /// knows a newer request came. Removed with the slot's reservation.
    accepted: HashMap<Key, u64>,
    reports: VecDeque<Report>,
    closed: bool,
}

impl State {
    /// Room for one more job and, for a slot not followed yet, one more slot.
    fn room_for(&self, key: &Key) -> bool {
        let followed = self.slots.contains_key(key) || self.reservations.contains(key);
        let count = self.slots.len()
            + self
                .reservations
                .iter()
                .filter(|k| !self.slots.contains_key(*k))
                .count();
        self.queue.len() + self.reserved < MAX_QUEUE && (followed || count < MAX_SLOTS)
    }

    fn revision(&mut self) -> Option<u64> {
        let revision = self.next_revision.checked_add(1)?;
        self.next_revision = revision;
        Some(revision)
    }

    fn unreserve(&mut self, key: &Key) {
        self.reserved = self.reserved.saturating_sub(1);
        if self.reservations.remove(key) {
            self.accepted.remove(key);
        }
    }

    /// Records an accepted revision for a reserved slot (see `accepted`).
    fn note_accepted(&mut self, key: &Key, revision: u64) {
        if self.reservations.contains(key) {
            let newest = self.accepted.entry(key.clone()).or_insert(revision);
            *newest = (*newest).max(revision);
        }
    }

    /// The newest revision accepted for `key` that is still known.
    fn newest(&self, key: &Key) -> Option<u64> {
        self.accepted
            .get(key)
            .copied()
            .max(self.slots.get(key).copied())
    }
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

fn key(slot: &Slot) -> Key {
    (
        slot.alias.clone(),
        slot.epoch.as_str().to_owned(),
        slot.terminal.as_str().to_owned(),
    )
}

/// The application's lease keeper. Share it (`Arc<Keeper>`) between the UI and the terminal
/// workers; dropping the last one stops accepting requests, and its thread finishes those
/// already accepted, then ends, without waiting for anyone to read the reports.
pub struct Keeper {
    shared: Arc<Shared>,
}

impl Drop for Keeper {
    fn drop(&mut self) {
        self.shared.state.lock().unwrap().closed = true;
        self.shared.changed.notify_all();
    }
}

impl Keeper {
    pub fn start<W: LeaseWriter>(writer: Arc<W>) -> Result<Keeper, KeeperError> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("host-lease-keeper".into())
            .spawn(move || run(&worker, &*writer))
            .map_err(|_| KeeperError)?;
        Ok(Keeper { shared })
    }

    /// Saves a lease a takeover (or an attachment) returned. It is never a proof that the
    /// terminal types. Refused: the lease is handed back, nothing changed.
    pub fn keep(&self, slot: Slot, lease: LeaseKey) -> Result<(), (Refusal, Slot, LeaseKey)> {
        match self.admit(slot, None, Work::Keep(lease)) {
            Ok(()) => Ok(()),
            Err((refusal, slot, Work::Keep(lease))) => Err((refusal, slot, lease)),
            Err((_, _, Work::Forget)) => unreachable!("handed back as given"),
        }
    }

    /// Forgets a lease on proof that its program ended.
    pub fn forget_ended(&self, slot: Slot, proof: Ended) -> Result<(), Refusal> {
        if let Ended::EpochChanged { current } = &proof
            && *current == slot.epoch
        {
            return Err(Refusal::InvalidProof);
        }
        self.admit(slot, None, Work::Forget)
            .map_err(|(refusal, _, _)| refusal)
    }

    pub fn retry_save(&self, mut retry: RetrySave) -> Result<(), RetryRefused<RetrySave>> {
        let (slot, revision) = {
            let token = retry.token.as_ref().expect("a live token");
            (token.slot.clone(), token.revision)
        };
        let lease = retry.lease.take().expect("a live token");
        match self.admit(slot, Some(revision), Work::Keep(lease)) {
            Ok(()) => {
                // Its revision now belongs to the queued retry: its drop must not let it go.
                if let Some(token) = retry.token.as_mut() {
                    token.shared = Weak::new();
                }
                Ok(())
            }
            Err((refusal, _, work)) => {
                if let Work::Keep(lease) = work {
                    retry.lease = Some(lease);
                }
                Err(match refusal {
                    Refusal::Superseded => RetryRefused::Superseded,
                    Refusal::Closed => RetryRefused::Closed(retry),
                    _ => RetryRefused::Busy(retry),
                })
            }
        }
    }

    pub fn retry_forget(&self, mut retry: RetryForget) -> Result<(), RetryRefused<RetryForget>> {
        let (slot, revision) = {
            let token = retry.token.as_ref().expect("a live token");
            (token.slot.clone(), token.revision)
        };
        match self.admit(slot, Some(revision), Work::Forget) {
            Ok(()) => {
                if let Some(token) = retry.token.as_mut() {
                    token.shared = Weak::new();
                }
                Ok(())
            }
            Err((refusal, _, _)) => Err(match refusal {
                Refusal::Superseded => RetryRefused::Superseded,
                Refusal::Closed => RetryRefused::Closed(retry),
                _ => RetryRefused::Busy(retry),
            }),
        }
    }

    /// Before a takeover or a close is sent: holds a queue place and a revision stamp for
    /// `slot`, so the lease it returns (or the removal after it) is always accepted. It does
    /// not change the slot's latest accepted request: an older retry or queued request keeps
    /// its chance until the reservation is committed. Refused: send nothing.
    ///
    /// `Busy` also while the slot already has a reservation: its takeover or close must be
    /// answered first (the host may answer two in either order). Other requests for the slot,
    /// a confirmed end above all, are still accepted meanwhile.
    pub fn reserve(&self, slot: Slot) -> Result<Reservation, Refusal> {
        let mut state = self.shared.state.lock().unwrap();
        if state.closed {
            return Err(Refusal::Closed);
        }
        let key = key(&slot);
        if state.reservations.contains(&key) || !state.room_for(&key) {
            return Err(Refusal::Busy);
        }
        let stamp = state.revision().ok_or(Refusal::Exhausted)?;
        state.reserved += 1;
        state.reservations.insert(key);
        Ok(Reservation {
            slot: Some(slot),
            stamp,
            shared: self.shared.clone(),
        })
    }

    /// The next report, if any. Read them on every frame.
    pub fn report(&self) -> Option<Report> {
        self.shared.state.lock().unwrap().reports.pop_front()
    }

    /// Accepts `work` for `slot` in one step under the lock (checks, a new revision, the
    /// slot's latest revision, the queue), or refuses it with nothing changed and the work
    /// handed back. A retry is accepted only while its revision is still the slot's latest.
    fn admit(
        &self,
        slot: Slot,
        retry_of: Option<u64>,
        work: Work,
    ) -> Result<(), (Refusal, Slot, Work)> {
        let mut state = self.shared.state.lock().unwrap();
        if state.closed {
            return Err((Refusal::Closed, slot, work));
        }
        let key = key(&slot);
        if let Some(revision) = retry_of
            && state.slots.get(&key) != Some(&revision)
        {
            return Err((Refusal::Superseded, slot, work));
        }
        if !state.room_for(&key) {
            return Err((Refusal::Busy, slot, work));
        }
        // A retry keeps the revision of the request it repeats: it is that request again,
        // not a newer one, so a reservation taken since (a fresher lease) still wins.
        let revision = match retry_of {
            Some(revision) => revision,
            None => match state.revision() {
                Some(revision) => revision,
                None => return Err((Refusal::Exhausted, slot, work)),
            },
        };
        state.note_accepted(&key, revision);
        state.slots.insert(key, revision);
        state.queue.push_back(Job {
            slot,
            revision,
            work,
        });
        drop(state);
        self.shared.changed.notify_all();
        Ok(())
    }
}

fn run(shared: &Arc<Shared>, writer: &dyn LeaseWriter) {
    loop {
        let job = {
            let mut state = shared.state.lock().unwrap();
            loop {
                if let Some(job) = state.queue.pop_front() {
                    break job;
                }
                // Reservations still to commit keep the thread, even once closed.
                if state.closed && state.reserved == 0 {
                    return;
                }
                state = shared.changed.wait(state).unwrap();
            }
        };
        let current = {
            let state = shared.state.lock().unwrap();
            state.slots.get(&key(&job.slot)) == Some(&job.revision)
        };
        let report = if !current {
            Report::Superseded(job.slot)
        } else {
            let done = match &job.work {
                Work::Keep(lease) => writer.put(&job.slot, lease),
                Work::Forget => writer.remove(&job.slot),
            };
            let mut state = shared.state.lock().unwrap();
            let key = key(&job.slot);
            let still_latest = state.slots.get(&key) == Some(&job.revision);
            match (done, job.work) {
                (Ok(()), work) => {
                    if still_latest {
                        state.slots.remove(&key);
                    }
                    drop(state);
                    match work {
                        Work::Keep(_) => Report::Kept(job.slot),
                        Work::Forget => Report::Forgotten(job.slot),
                    }
                }
                // A newer request came while this one was written: it decides.
                (Err(_), _) if !still_latest => {
                    drop(state);
                    Report::Superseded(job.slot)
                }
                (Err(_), Work::Keep(lease)) => {
                    drop(state);
                    Report::NotKept(RetrySave {
                        token: Some(TokenInner {
                            slot: job.slot,
                            revision: job.revision,
                            shared: Arc::downgrade(shared),
                        }),
                        lease: Some(lease),
                    })
                }
                (Err(_), Work::Forget) => {
                    drop(state);
                    Report::NotForgotten(RetryForget {
                        token: Some(TokenInner {
                            slot: job.slot,
                            revision: job.revision,
                            shared: Arc::downgrade(shared),
                        }),
                    })
                }
            }
        };
        deliver(shared, report, (current).then_some(job.revision));
    }
}

/// Keeps every failure report until read; informational ones are capped (the oldest goes).
///
/// `revision`: the job this report comes from, when it was its slot's latest. Failure reports of
/// that slot from older revisions are then out of date (their retry would be refused anyway)
/// and are removed, so failures stay at most one per followed slot. Removed reports are
/// dropped after the lock is released: a retry token's drop takes that lock.
fn deliver(shared: &Shared, report: Report, revision: Option<u64>) {
    let mut removed = Vec::new();
    {
        let mut state = shared.state.lock().unwrap();
        if let Some(revision) = revision {
            let slot = match &report {
                Report::Kept(slot) | Report::Forgotten(slot) | Report::Superseded(slot) => slot,
                Report::NotKept(retry) => retry.slot(),
                Report::NotForgotten(retry) => retry.slot(),
            };
            let key = key(slot);
            let mut kept = VecDeque::with_capacity(state.reports.len());
            for old in state.reports.drain(..) {
                let stale = old.failure().is_some_and(|(old_slot, old_revision)| {
                    old_revision < revision && self::key(old_slot) == key
                });
                if stale {
                    removed.push(old);
                } else {
                    kept.push_back(old);
                }
            }
            state.reports = kept;
        }
        if !report.durable() {
            let informational = state.reports.iter().filter(|r| !r.durable()).count();
            if informational >= MAX_INFO_REPORTS
                && let Some(oldest) = state.reports.iter().position(|r| !r.durable())
                && let Some(evicted) = state.reports.remove(oldest)
            {
                removed.push(evicted);
            }
        }
        state.reports.push_back(report);
    }
    drop(removed);
}
