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
//!
//! A creation lease (`promote`, or `Place::promote` from a place taken with `reserve_new`
//! before the create was sent, when the terminal's id is not known yet) replaces the create's
//! pending record by the lease in one write (`LeaseWriter::complete`). It is the oldest lease
//! of its terminal, so it ranks below every other request: it never makes a save or a removal
//! `Superseded`, a reservation committed after it wins whatever the stamps, and it is itself
//! skipped (the pending record left untouched, for an explicit cleanup) once a save or a
//! removal for its slot was accepted. A reservation still in flight does not count: it proves
//! no newer lease, and a takeover that fails must leave the creation lease saved.
//!
//! While a place is held, every accepted save or removal leaves a tombstone (its slot and
//! revision), even once written, so a late `Place::promote` still sees it. Tombstones older
//! than the oldest held place go when places are given back. The slot table counts followed
//! slots, reservations, tombstones and places together, within `MAX_SLOTS`.
//!
//! `is_idle` says when nothing is queued, reserved or being written (reports included), for
//! the application to know it can quit without cutting a write. Before quitting it calls
//! `close_admissions`: no new reservation or place (they come before a request to the host),
//! while what is already held, and saves, removals and retries, are still taken.
use super::host_opening::{LocalError, Slot};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
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
    /// Replaces the pending record `pending` by the slot's lease, in one write
    /// (`host_opening::Store::complete`).
    fn complete(&self, pending: &str, slot: &Slot, lease: &LeaseKey) -> Result<(), LocalError>;
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
/// Clone. Dropped unused, it lets its slot go (a pending record it would have replaced stays).
pub struct RetrySave {
    token: Option<Box<TokenInner>>,
    lease: Option<LeaseKey>,
    /// From a promote: the pending record the retry replaces.
    pending: Option<String>,
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
            if state.latest(&key) == Some(self.revision) {
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
    /// skipped (`Superseded`) only if a newer save or removal for the slot was accepted since
    /// the reservation, which then decides (a confirmed end, a newer lease). A creation lease
    /// never makes it skip: the takeover's lease is newer whatever the stamps.
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

/// Turns a reservation into a job, unless a newer save or removal for its slot was accepted
/// since. A creation lease never counts: the takeover's lease (or its end) is newer.
fn commit(shared: &Arc<Shared>, slot: Slot, stamp: u64, work: Work) {
    let superseded = {
        let mut state = shared.state.lock().unwrap();
        let key = key(&slot);
        // Checked before this reservation is given back: `accepted` goes with it.
        let superseded = state.newest_firm(&key).is_some_and(|newest| newest > stamp);
        if !superseded {
            state.note_accepted(&key, stamp, false);
            state.slots.insert(
                key.clone(),
                Latest {
                    revision: stamp,
                    create: false,
                },
            );
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

/// A place taken with `Keeper::reserve_new` before a create is sent, when the terminal's id
/// is not known yet: a queue place, a place in the slot table, and a stamp from before the
/// request. Commit it with `promote`; dropped unused, it gives both places back. It works even
/// after the keeper was dropped. No Debug, no Clone.
pub struct Place {
    stamp: Option<u64>,
    shared: Arc<Shared>,
}

/// What `Place::promote` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Promoted {
    /// Queued: its report follows.
    Queued,
    /// A save or a removal for the slot was accepted since the place was taken (or is queued,
    /// or failed and holds the slot): it decides. The pending record is left untouched.
    Superseded,
}

impl Place {
    /// Saves the lease the create granted, replacing the create's pending record in one write.
    /// Always admitted (the places are held), unless superseded (see `Promoted`). It never
    /// overrides anything: a takeover in flight that commits later wins, and one dropped
    /// leaves the creation lease saved.
    pub fn promote(mut self, slot: Slot, pending: String, lease: LeaseKey) -> Promoted {
        let stamp = self.stamp.take().expect("a live place");
        let shared = self.shared.clone();
        let superseded = {
            let mut state = shared.state.lock().unwrap();
            let key = key(&slot);
            let superseded = state.firm(&key);
            if !superseded {
                state.slots.insert(
                    key,
                    Latest {
                        revision: stamp,
                        create: true,
                    },
                );
                state.queue.push_back(Job {
                    slot: slot.clone(),
                    revision: stamp,
                    work: Work::Promote {
                        pending: pending.into_boxed_str(),
                        lease,
                    },
                });
            }
            state.release_place(stamp);
            superseded
        };
        shared.changed.notify_all();
        if superseded {
            deliver(&shared, Report::Superseded(slot), None);
            Promoted::Superseded
        } else {
            Promoted::Queued
        }
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        if let Some(stamp) = self.stamp.take() {
            self.shared.state.lock().unwrap().release_place(stamp);
            self.shared.changed.notify_all();
        }
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
            Self::NotKept(retry) => retry.token.as_deref(),
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
    Promote { pending: Box<str>, lease: LeaseKey },
    Forget,
}

struct Job {
    slot: Slot,
    revision: u64,
    work: Work,
}

type Key = (String, String, String);

/// A slot's latest accepted request: its revision, and whether it is a creation lease.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Latest {
    revision: u64,
    create: bool,
}

#[derive(Default)]
struct State {
    next_revision: u64,
    /// Each followed slot's latest accepted request.
    slots: HashMap<Key, Latest>,
    queue: VecDeque<Job>,
    /// Queue places held by reservations and places not committed yet.
    reserved: usize,
    /// Slots with a reservation (at most one each): a reserved slot counts as followed.
    reservations: HashSet<Key>,
    /// Stamps of the places held (`reserve_new`); the oldest is the tombstones' horizon.
    places: BTreeSet<u64>,
    /// Tombstones: the newest save or removal accepted for a slot, kept even once written
    /// and gone from `slots` (or its retry token dropped), while the slot is reserved or a
    /// place older than it is held. Never a creation lease.
    accepted: HashMap<Key, u64>,
    /// A job taken from the queue and not yet reported.
    in_flight: bool,
    /// No new reservation or place (`close_admissions`).
    admissions_closed: bool,
    reports: VecDeque<Report>,
    closed: bool,
}

impl State {
    fn latest(&self, key: &Key) -> Option<u64> {
        self.slots.get(key).map(|latest| latest.revision)
    }

    fn queue_room(&self) -> bool {
        self.queue.len() + self.reserved < MAX_QUEUE
    }

    fn in_table(&self, key: &Key) -> bool {
        self.slots.contains_key(key)
            || self.reservations.contains(key)
            || self.accepted.contains_key(key)
    }

    /// Places used in the slot table: followed slots, reservations, tombstones and places.
    fn table_len(&self) -> usize {
        let reserved = self
            .reservations
            .iter()
            .filter(|k| !self.slots.contains_key(*k))
            .count();
        let tombstones = self
            .accepted
            .keys()
            .filter(|k| !self.slots.contains_key(*k) && !self.reservations.contains(*k))
            .count();
        self.slots.len() + reserved + tombstones + self.places.len()
    }

    /// Room for one more job and, for a slot not in the table yet, one more place in it.
    fn room_for(&self, key: &Key) -> bool {
        self.queue_room() && (self.in_table(key) || self.table_len() < MAX_SLOTS)
    }

    fn revision(&mut self) -> Option<u64> {
        let revision = self.next_revision.checked_add(1)?;
        self.next_revision = revision;
        Some(revision)
    }

    fn unreserve(&mut self, key: &Key) {
        self.reserved = self.reserved.saturating_sub(1);
        if self.reservations.remove(key) {
            self.prune_tombstones();
        }
    }

    fn release_place(&mut self, stamp: u64) {
        if self.places.remove(&stamp) {
            self.reserved = self.reserved.saturating_sub(1);
            self.prune_tombstones();
        }
    }

    /// Keeps the tombstones a reservation or a held place may still need.
    fn prune_tombstones(&mut self) {
        let horizon = self.places.first().copied();
        let State {
            accepted,
            reservations,
            ..
        } = self;
        accepted.retain(|key, revision| {
            reservations.contains(key) || horizon.is_some_and(|oldest| *revision > oldest)
        });
    }

    /// Leaves a tombstone for an accepted save or removal, while a reservation of its slot or a
    /// place is held (see `accepted`). A creation lease leaves none.
    fn note_accepted(&mut self, key: &Key, revision: u64, create: bool) {
        if !create && (self.reservations.contains(key) || !self.places.is_empty()) {
            let newest = self.accepted.entry(key.clone()).or_insert(revision);
            *newest = (*newest).max(revision);
        }
    }

    /// The newest save or removal accepted for `key` that is still known; creation leases
    /// never count.
    fn newest_firm(&self, key: &Key) -> Option<u64> {
        let latest = self
            .slots
            .get(key)
            .filter(|latest| !latest.create)
            .map(|latest| latest.revision);
        self.accepted.get(key).copied().max(latest)
    }

    /// A save or a removal for `key` is queued, failed and holds the slot, or was accepted
    /// while a reservation or a place was held: a creation lease comes after it.
    fn firm(&self, key: &Key) -> bool {
        self.slots.get(key).is_some_and(|latest| !latest.create) || self.accepted.contains_key(key)
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
            Err(_) => unreachable!("handed back as given"),
        }
    }

    /// Saves the lease a create granted when the opening could not save it (`Opened::saved`
    /// false with `pending`): replaces that pending record by the lease, in one write. A
    /// creation lease ranks below every other request (see the module): when a save or a
    /// removal for the slot was already accepted, it is skipped (`Ok`, reported `Superseded`,
    /// the pending record left as it is). Refused for room: everything is handed back,
    /// nothing changed. Prefer `reserve_new` before the create and `Place::promote`.
    pub fn promote(
        &self,
        slot: Slot,
        pending: String,
        lease: LeaseKey,
    ) -> Result<(), Box<(Refusal, Slot, String, LeaseKey)>> {
        match self.admit(
            slot,
            None,
            Work::Promote {
                pending: pending.into_boxed_str(),
                lease,
            },
        ) {
            Ok(()) => Ok(()),
            Err((refusal, slot, Work::Promote { pending, lease })) => {
                Err(Box::new((refusal, slot, pending.into_string(), lease)))
            }
            Err(_) => unreachable!("handed back as given"),
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
        // A failed promote is retried as a promote: its pending record still stands.
        let work = match retry.pending.take() {
            Some(pending) => Work::Promote {
                pending: pending.into_boxed_str(),
                lease,
            },
            None => Work::Keep(lease),
        };
        match self.admit(slot, Some(revision), work) {
            Ok(()) => {
                // Its revision now belongs to the queued retry: its drop must not let it go.
                if let Some(token) = retry.token.as_mut() {
                    token.shared = Weak::new();
                }
                Ok(())
            }
            Err((refusal, _, work)) => {
                match work {
                    Work::Keep(lease) => retry.lease = Some(lease),
                    Work::Promote { pending, lease } => {
                        retry.lease = Some(lease);
                        retry.pending = Some(pending.into_string());
                    }
                    Work::Forget => {}
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
        if state.closed || state.admissions_closed {
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

    /// Before a create is sent, when the terminal's id is not known yet: a queue place, a
    /// place in the slot table and a stamp, for `Place::promote` once the create answered.
    /// Refused (`Busy`: no room in the queue or the table): send nothing.
    pub fn reserve_new(&self) -> Result<Place, Refusal> {
        let mut state = self.shared.state.lock().unwrap();
        if state.closed || state.admissions_closed {
            return Err(Refusal::Closed);
        }
        if !state.queue_room() || state.table_len() >= MAX_SLOTS {
            return Err(Refusal::Busy);
        }
        let stamp = state.revision().ok_or(Refusal::Exhausted)?;
        state.reserved += 1;
        state.places.insert(stamp);
        Ok(Place {
            stamp: Some(stamp),
            shared: self.shared.clone(),
        })
    }

    /// Before the application quits (once the person confirmed it): from now on `reserve`
    /// and `reserve_new` answer `Closed`, so no new request to a host can need a write. The
    /// reservations and places already held still commit, and saves, removals and retries are
    /// still accepted (none sends anything to a host). Idempotent; never undone.
    pub fn close_admissions(&self) {
        self.shared.state.lock().unwrap().admissions_closed = true;
        self.shared.changed.notify_all();
    }

    /// `close_admissions` was called.
    pub fn admissions_closed(&self) -> bool {
        self.shared.state.lock().unwrap().admissions_closed
    }

    /// Nothing queued, reserved, placed or being written, and the last write's report is
    /// delivered: the application can quit without cutting a write. Failure reports not read
    /// yet are not counted here (the application holds them).
    pub fn is_idle(&self) -> bool {
        let state = self.shared.state.lock().unwrap();
        state.queue.is_empty() && state.reserved == 0 && !state.in_flight
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
        let create = matches!(work, Work::Promote { .. });
        if let Some(revision) = retry_of
            && state.latest(&key) != Some(revision)
        {
            return Err((Refusal::Superseded, slot, work));
        }
        // A creation lease comes after any save or removal already accepted for its slot:
        // skipped, the pending record left as it is.
        if create && retry_of.is_none() && state.firm(&key) {
            drop(state);
            deliver(&self.shared, Report::Superseded(slot), None);
            return Ok(());
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
        state.note_accepted(&key, revision, create);
        state.slots.insert(key, Latest { revision, create });
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
                    state.in_flight = true;
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
            state.latest(&key(&job.slot)) == Some(job.revision)
        };
        let report = if !current {
            Report::Superseded(job.slot)
        } else {
            let done = match &job.work {
                Work::Keep(lease) => writer.put(&job.slot, lease),
                Work::Promote { pending, lease } => writer.complete(pending, &job.slot, lease),
                Work::Forget => writer.remove(&job.slot),
            };
            let mut state = shared.state.lock().unwrap();
            let key = key(&job.slot);
            let still_latest = state.latest(&key) == Some(job.revision);
            match (done, job.work) {
                (Ok(()), work) => {
                    if still_latest {
                        state.slots.remove(&key);
                    }
                    drop(state);
                    match work {
                        Work::Keep(_) | Work::Promote { .. } => Report::Kept(job.slot),
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
                        token: Some(Box::new(TokenInner {
                            slot: job.slot,
                            revision: job.revision,
                            shared: Arc::downgrade(shared),
                        })),
                        lease: Some(lease),
                        pending: None,
                    })
                }
                (Err(_), Work::Promote { pending, lease }) => {
                    drop(state);
                    Report::NotKept(RetrySave {
                        token: Some(Box::new(TokenInner {
                            slot: job.slot,
                            revision: job.revision,
                            shared: Arc::downgrade(shared),
                        })),
                        lease: Some(lease),
                        pending: Some(pending.into_string()),
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
        // Idle only once the report is delivered.
        shared.state.lock().unwrap().in_flight = false;
        shared.changed.notify_all();
    }
}

/// Keeps every failure report until read; informational ones are capped (the oldest goes).
///
/// `revision`: the job this report comes from, when it was its slot's latest. Failure reports of
/// that slot with any other revision are then out of date (their retry would be refused: it
/// needs its revision to be the latest) and are removed, so failures stay at most one per
/// followed slot. A creation lease's failure can carry a newer revision than the takeover
/// that won over it, hence any other, not only older ones. Removed reports are dropped after
/// the lock is released: a retry token's drop takes that lock.
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
                    old_revision != revision && self::key(old_slot) == key
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
