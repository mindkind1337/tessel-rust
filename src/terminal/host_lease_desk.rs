//! The application's lease desk: the one reader of the lease keeper's reports, and the holder
//! of what a pane cannot keep for itself. It is not tied to a pane, so closing one loses
//! nothing.
//!
//! - Failure tokens (`RetrySave`, `RetryForget`) live here, one per slot (the keeper keeps at
//!   most one failure per slot), shared by every pane that shows the slot. Any of them can
//!   retry, with the token's own revision: a retry never overwrites a newer request.
//! - A proof of end the keeper had no room for waits here (one per slot, at most `MAX_SLOTS`)
//!   and is admitted again on every frame. A proof of end may take a new revision: once a
//!   program ended, no lease of it is worth keeping.
//! - A refusal hands ownership back to the caller; nothing is dropped silently. A proof that
//!   finds no room even here is handed back, and a caller that cannot keep it (a pane being
//!   dropped) counts it (`count_unrecorded_end`) for an explicit cleanup: what stays on disk is
//!   then a lease worth nothing any more, never one still needed.
//! - Bounded: tokens by the keeper (`MAX_SLOTS`), waiting proofs by `MAX_SLOTS` (each tried at
//!   most once per frame), other entries by the panes that show a slot. That last bound is the
//!   UI's own (each pane registers the one slot it shows); a count that cannot grow is refused,
//!   never wrapped. Settled entries no pane shows are removed on every frame.
//! - Lock order: the desk's lock, then the keeper's. The keeper never calls the desk.
use super::host_lease_keeper::{
    Ended, Keeper, MAX_SLOTS, Refusal, Report, RetryForget, RetryRefused, RetrySave,
};
use super::host_opening::Slot;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type Key = (String, String, String);

fn key(slot: &Slot) -> Key {
    (
        slot.alias.clone(),
        slot.epoch.as_str().to_owned(),
        slot.terminal.as_str().to_owned(),
    )
}

/// The same proof again, for another admission (`forget_ended` takes it).
fn copy(proof: &Ended) -> Ended {
    match proof {
        Ended::ExitReceived => Ended::ExitReceived,
        Ended::CloseConfirmed => Ended::CloseConfirmed,
        Ended::EpochChanged { current } => Ended::EpochChanged {
            current: current.clone(),
        },
    }
}

/// What a pane shows about its slot's saved access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// Nothing reported yet.
    Unknown,
    /// A newer request superseded this access; its result is no longer retained here.
    Superseded,
    /// A retry is queued: its report follows.
    Retrying,
    Saved,
    /// The save failed: offer `DeskHandle::retry`.
    NotSaved,
    /// The program ended and its lease is forgotten.
    Forgotten,
    /// The removal failed: offer `DeskHandle::retry`.
    NotForgotten,
    /// The program ended; recording it waits for room in the keeper, retried on every frame.
    Ending,
}

/// What an explicit retry did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retried {
    /// Queued: its report follows.
    Queued,
    /// The keeper is full: the token stays here; try again later.
    Busy,
    /// A newer request for the slot was accepted since: nothing left to retry.
    Superseded,
    /// No failure to retry for this slot.
    Nothing,
}

/// `DeskHandle::register` could not count one more pane for a slot (never in practice).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooManyPanes;

/// A proof of end the desk did not take.
pub enum ProofRefused {
    /// `EpochChanged` with the slot's own epoch proves nothing: nothing to keep.
    Invalid,
    /// No room, even here: handed back as given.
    Full(Slot, Ended),
}

/// What is not settled yet: ask before the application quits while it is not empty.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unsettled {
    /// Saves that failed: the access would not survive a restart.
    pub not_saved: usize,
    /// Removals that failed: an ended program's lease is still on disk.
    pub not_forgotten: usize,
    /// Proofs of end still waiting for room in the keeper.
    pub ends_waiting: usize,
    /// Proofs of end nobody could keep: their leases need an explicit cleanup.
    pub unrecorded_ends: usize,
}

impl Unsettled {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

// No Debug: the tokens hold leases.
enum Status {
    Superseded,
    Retrying,
    Saved,
    NotSaved(RetrySave),
    Forgotten,
    NotForgotten(RetryForget),
}

struct Entry {
    slot: Slot,
    status: Option<Status>,
    proof: Option<Ended>,
    panes: usize,
}

impl Entry {
    /// Nothing to keep: no pane shows it, no token, no waiting proof.
    fn settled(&self) -> bool {
        self.panes == 0
            && self.proof.is_none()
            && !matches!(
                self.status,
                Some(Status::NotSaved(_) | Status::NotForgotten(_))
            )
    }
}

#[derive(Default)]
struct Desk {
    entries: HashMap<Key, Entry>,
    unrecorded_ends: usize,
}

impl Desk {
    fn entry(&mut self, slot: &Slot) -> &mut Entry {
        self.entries.entry(key(slot)).or_insert_with(|| Entry {
            slot: slot.clone(),
            status: None,
            proof: None,
            panes: 0,
        })
    }

    fn waiting_proofs(&self) -> usize {
        self.entries.values().filter(|e| e.proof.is_some()).count()
    }
}

/// Owned by the application, next to the panes (declare the panes first, so they let go
/// before the desk does). Give each pane a `DeskHandle`.
pub struct LeaseDesk {
    handle: DeskHandle,
}

/// A pane's way to the desk. Cheap to clone.
#[derive(Clone)]
pub struct DeskHandle {
    keeper: Arc<Keeper>,
    desk: Arc<Mutex<Desk>>,
}

/// Keeps a slot's reports visible while an opening travels from its worker to
/// its pane. Dropping a failed opening removes only its subscription, not retries.
pub struct Seat {
    handle: DeskHandle,
    slot: Slot,
}

impl Drop for Seat {
    fn drop(&mut self) {
        self.handle.unregister(&self.slot);
    }
}

impl super::host_opening::Register for DeskHandle {
    fn seat(&self, slot: &Slot) -> Result<Box<dyn Send>, super::host_opening::LocalError> {
        DeskHandle::seat(self, slot)
            .map(|seat| Box::new(seat) as Box<dyn Send>)
            .map_err(|_| super::host_opening::LocalError)
    }
}

impl LeaseDesk {
    pub fn new(keeper: Arc<Keeper>) -> Self {
        Self {
            handle: DeskHandle {
                keeper,
                desk: Arc::new(Mutex::new(Desk::default())),
            },
        }
    }

    pub fn handle(&self) -> DeskHandle {
        self.handle.clone()
    }

    /// Once per frame, on the UI thread, without I/O: reads every keeper report (the desk is
    /// their only reader), admits the waiting proofs again, and lets settled entries go.
    /// Returns whether this frame admitted a deferred cleanup write.
    pub fn frame(&self) -> bool {
        let handle = &self.handle;
        let mut desk = handle.desk.lock().unwrap();
        // In the keeper's order: the last report of a slot is its state.
        while let Some(report) = handle.keeper.report() {
            match report {
                Report::Kept(slot) => desk.entry(&slot).status = Some(Status::Saved),
                Report::Forgotten(slot) => {
                    let entry = desk.entry(&slot);
                    entry.status = Some(Status::Forgotten);
                    entry.proof = None;
                }
                // Its newer request may already have been drained and pruned before
                // this opening registered. Never invent a saved or failed result.
                Report::Superseded(slot) => {
                    let entry = desk.entry(&slot);
                    if entry.status.is_none() {
                        entry.status = Some(Status::Superseded);
                    }
                }
                Report::NotKept(retry) => {
                    let slot = retry.slot().clone();
                    desk.entry(&slot).status = Some(Status::NotSaved(retry));
                }
                Report::NotForgotten(retry) => {
                    let slot = retry.slot().clone();
                    desk.entry(&slot).status = Some(Status::NotForgotten(retry));
                }
            }
        }
        // Every waiting proof is tried, once, whatever the others answered: one refused for
        // room (a new slot while the keeper's table is full) must not hold back another that
        // has room (a slot the keeper already follows needs no new place in its table).
        let mut tried = 0;
        let mut admitted = false;
        for entry in desk.entries.values_mut() {
            let Some(proof) = &entry.proof else {
                continue;
            };
            if tried == MAX_SLOTS {
                break;
            }
            tried += 1;
            match handle.keeper.forget_ended(entry.slot.clone(), copy(proof)) {
                Ok(()) => {
                    entry.proof = None;
                    admitted = true;
                }
                // Checked when it was handed in; nothing to keep.
                Err(Refusal::InvalidProof) => entry.proof = None,
                // No room (or never: the desk holds the keeper open): kept for the next frame.
                Err(_) => {}
            }
        }
        desk.entries.retain(|_, entry| !entry.settled());
        admitted
    }
}

impl DeskHandle {
    /// Explicit user action, including after panes have closed. No I/O here;
    /// retry tokens are admitted back to the keeper or remain owned by the desk.
    pub fn retry_failures(&self) {
        let slots: Vec<_> = self
            .desk
            .lock()
            .unwrap()
            .entries
            .values()
            .filter(|entry| {
                matches!(
                    entry.status,
                    Some(Status::NotSaved(_) | Status::NotForgotten(_))
                )
            })
            .map(|entry| entry.slot.clone())
            .collect();
        for slot in slots {
            self.retry(&slot);
        }
    }

    pub fn seat(&self, slot: &Slot) -> Result<Seat, TooManyPanes> {
        self.register(slot)?;
        Ok(Seat {
            handle: self.clone(),
            slot: slot.clone(),
        })
    }
    /// A pane shows `slot`. Pair it with `unregister`. The panes are counted with a checked
    /// addition: a count that cannot grow is refused, never wrapped.
    pub fn register(&self, slot: &Slot) -> Result<(), TooManyPanes> {
        let mut desk = self.desk.lock().unwrap();
        let entry = desk.entry(slot);
        entry.panes = entry.panes.checked_add(1).ok_or(TooManyPanes)?;
        Ok(())
    }

    /// The pane no longer shows `slot`. A failure or a waiting proof stays.
    pub fn unregister(&self, slot: &Slot) {
        let mut desk = self.desk.lock().unwrap();
        let key = key(slot);
        if let Some(entry) = desk.entries.get_mut(&key) {
            entry.panes = entry.panes.saturating_sub(1);
            if entry.settled() {
                desk.entries.remove(&key);
            }
        }
    }

    pub fn view(&self, slot: &Slot) -> View {
        let desk = self.desk.lock().unwrap();
        let Some(entry) = desk.entries.get(&key(slot)) else {
            return View::Unknown;
        };
        if entry.proof.is_some() {
            return View::Ending;
        }
        match entry.status {
            None => View::Unknown,
            Some(Status::Superseded) => View::Superseded,
            Some(Status::Retrying) => View::Retrying,
            Some(Status::Saved) => View::Saved,
            Some(Status::NotSaved(_)) => View::NotSaved,
            Some(Status::Forgotten) => View::Forgotten,
            Some(Status::NotForgotten(_)) => View::NotForgotten,
        }
    }

    /// Retries the slot's failed save or removal, from any pane that shows it, with the one
    /// token the desk holds. Refused for room: the token stays here.
    pub fn retry(&self, slot: &Slot) -> Retried {
        let mut desk = self.desk.lock().unwrap();
        let Some(entry) = desk.entries.get_mut(&key(slot)) else {
            return Retried::Nothing;
        };
        match entry.status.take() {
            Some(Status::NotSaved(token)) => match self.keeper.retry_save(token) {
                Ok(()) => {
                    entry.status = Some(Status::Retrying);
                    Retried::Queued
                }
                Err(RetryRefused::Busy(token) | RetryRefused::Closed(token)) => {
                    entry.status = Some(Status::NotSaved(token));
                    Retried::Busy
                }
                Err(RetryRefused::Superseded) => Retried::Superseded,
            },
            Some(Status::NotForgotten(token)) => match self.keeper.retry_forget(token) {
                Ok(()) => {
                    entry.status = Some(Status::Retrying);
                    Retried::Queued
                }
                Err(RetryRefused::Busy(token) | RetryRefused::Closed(token)) => {
                    entry.status = Some(Status::NotForgotten(token));
                    Retried::Busy
                }
                Err(RetryRefused::Superseded) => Retried::Superseded,
            },
            other => {
                entry.status = other;
                Retried::Nothing
            }
        }
    }

    /// Records that `slot`'s program ended: admitted now if the keeper has room, else kept here
    /// and admitted again on every frame. A proof already waiting for the slot says the same.
    /// No room even here: handed back (`Full`), for the caller to keep or to count.
    pub fn hand_proof(&self, slot: Slot, proof: Ended) -> Result<(), ProofRefused> {
        if let Ended::EpochChanged { current } = &proof
            && *current == slot.epoch
        {
            return Err(ProofRefused::Invalid);
        }
        let mut desk = self.desk.lock().unwrap();
        if desk
            .entries
            .get(&key(&slot))
            .is_some_and(|entry| entry.proof.is_some())
        {
            return Ok(());
        }
        match self.keeper.forget_ended(slot.clone(), copy(&proof)) {
            Ok(()) => return Ok(()),
            Err(Refusal::InvalidProof) => return Err(ProofRefused::Invalid),
            Err(_) => {}
        }
        if desk.waiting_proofs() >= MAX_SLOTS {
            return Err(ProofRefused::Full(slot, proof));
        }
        desk.entry(&slot).proof = Some(proof);
        Ok(())
    }

    /// A proof of end nobody could keep (handed back `Full` to a pane being dropped): its lease
    /// stays on disk, worth nothing, until an explicit cleanup. Counted in `unsettled`.
    pub fn count_unrecorded_end(&self) {
        let mut desk = self.desk.lock().unwrap();
        desk.unrecorded_ends = desk.unrecorded_ends.saturating_add(1);
    }

    /// What is not settled yet, for the exit dialog.
    pub fn unsettled(&self) -> Unsettled {
        let desk = self.desk.lock().unwrap();
        let mut unsettled = Unsettled {
            unrecorded_ends: desk.unrecorded_ends,
            ..Unsettled::default()
        };
        for entry in desk.entries.values() {
            match entry.status {
                Some(Status::NotSaved(_)) => unsettled.not_saved += 1,
                Some(Status::NotForgotten(_)) => unsettled.not_forgotten += 1,
                _ => {}
            }
            if entry.proof.is_some() {
                unsettled.ends_waiting += 1;
            }
        }
        unsettled
    }
}
