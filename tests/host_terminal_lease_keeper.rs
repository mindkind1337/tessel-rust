//! The lease keeper with a synthetic writer: saves and removals in order, never an older
//! request over a newer one, failures reported with a retry token until read, bounded queues,
//! and a shutdown that finishes accepted work without waiting for anyone.
#[allow(dead_code)]
#[path = "../src/terminal/host_commands.rs"]
mod host_commands;
#[allow(dead_code)]
#[path = "../src/terminal/host_handoff.rs"]
mod host_handoff;
#[allow(dead_code)]
#[path = "../src/terminal/host_lease_keeper.rs"]
mod host_lease_keeper;
#[allow(dead_code)]
#[path = "../src/terminal/host_opening.rs"]
mod host_opening;
#[allow(dead_code)]
#[path = "../src/terminal/host_worker.rs"]
mod host_worker;

use host_lease_keeper::{
    Ended, Keeper, LeaseWriter, MAX_INFO_REPORTS, MAX_QUEUE, MAX_SLOTS, Promoted, Refusal, Report,
    RetryRefused,
};
use host_opening::{LocalError, Slot};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tessel_host_protocol::{Epoch, LeaseKey, TerminalId};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Op {
    Put(TerminalId, LeaseKey),
    Remove(TerminalId),
    Complete(String, TerminalId, LeaseKey),
}

#[derive(Default)]
struct WriterState {
    ops: Vec<Op>,
    /// Results of the next writes, in order; empty means Ok.
    results: Vec<Result<(), LocalError>>,
    blocked: bool,
    entered: Option<Sender<()>>,
    attempts: usize,
}

#[derive(Default)]
struct FakeWriter {
    state: Mutex<WriterState>,
    released: Condvar,
}

impl FakeWriter {
    fn block(&self) -> Receiver<()> {
        let (entered, heard) = mpsc::channel();
        let mut state = self.state.lock().unwrap();
        state.blocked = true;
        state.entered = Some(entered);
        heard
    }

    fn release(&self) {
        self.state.lock().unwrap().blocked = false;
        self.released.notify_all();
    }

    fn fail_next(&self, count: usize) {
        self.state.lock().unwrap().results = vec![Err(LocalError); count];
    }

    fn ops(&self) -> Vec<Op> {
        self.state.lock().unwrap().ops.clone()
    }

    fn attempts(&self) -> usize {
        self.state.lock().unwrap().attempts
    }

    fn write(&self, op: Op) -> Result<(), LocalError> {
        let mut state = self.state.lock().unwrap();
        if let Some(entered) = state.entered.take() {
            let _ = entered.send(());
        }
        while state.blocked {
            state = self.released.wait(state).unwrap();
        }
        state.attempts += 1;
        let result = if state.results.is_empty() {
            Ok(())
        } else {
            state.results.remove(0)
        };
        if result.is_ok() {
            state.ops.push(op);
        }
        result
    }
}

impl LeaseWriter for FakeWriter {
    fn put(&self, slot: &Slot, lease: &LeaseKey) -> Result<(), LocalError> {
        self.write(Op::Put(slot.terminal.clone(), lease.clone()))
    }

    fn remove(&self, slot: &Slot) -> Result<(), LocalError> {
        self.write(Op::Remove(slot.terminal.clone()))
    }

    fn complete(&self, pending: &str, slot: &Slot, lease: &LeaseKey) -> Result<(), LocalError> {
        self.write(Op::Complete(
            pending.to_owned(),
            slot.terminal.clone(),
            lease.clone(),
        ))
    }
}

fn slot(n: u16) -> Slot {
    let mut bytes = [0x10u8; 16];
    bytes[0] = (n & 0xff) as u8;
    bytes[1] = (n >> 8) as u8;
    Slot {
        alias: "fixture".into(),
        epoch: Epoch::from_bytes([1; 16]),
        terminal: TerminalId::from_bytes(bytes),
    }
}

fn lease(n: u8) -> LeaseKey {
    LeaseKey::from_bytes([n; 16])
}

fn keeper() -> (Arc<FakeWriter>, Keeper) {
    let writer = Arc::new(FakeWriter::default());
    let keeper = Keeper::start(writer.clone()).unwrap();
    (writer, keeper)
}

/// The next report, waiting for the keeper's thread.
fn next(keeper: &Keeper) -> Report {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(report) = keeper.report() {
            return report;
        }
        assert!(Instant::now() < deadline, "no report came");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn name(report: &Report) -> &'static str {
    match report {
        Report::Kept(_) => "kept",
        Report::Forgotten(_) => "forgotten",
        Report::Superseded(_) => "superseded",
        Report::NotKept(_) => "not-kept",
        Report::NotForgotten(_) => "not-forgotten",
    }
}

#[test]
fn a_kept_lease_is_saved() {
    let (writer, keeper) = keeper();
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    assert!(matches!(next(&keeper), Report::Kept(s) if s == slot(1)));
    assert_eq!(writer.ops(), [Op::Put(slot(1).terminal, lease(1))]);
}

#[test]
fn a_failed_save_is_reported_with_a_retry_that_saves() {
    let (writer, keeper) = keeper();
    writer.fail_next(1);
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    let Report::NotKept(retry) = next(&keeper) else {
        panic!("expected not kept");
    };
    assert_eq!(retry.slot(), &slot(1));
    assert!(keeper.retry_save(retry).is_ok());
    assert!(matches!(next(&keeper), Report::Kept(_)));
    assert_eq!(writer.ops(), [Op::Put(slot(1).terminal, lease(1))]);
}

#[test]
fn an_older_failed_save_never_overwrites_a_newer_lease() {
    let (writer, keeper) = keeper();
    writer.fail_next(1);
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    let Report::NotKept(old) = next(&keeper) else {
        panic!("expected not kept");
    };
    keeper.keep(slot(1), lease(2)).ok().unwrap();
    assert!(matches!(next(&keeper), Report::Kept(_)));
    assert!(matches!(
        keeper.retry_save(old),
        Err(RetryRefused::Superseded)
    ));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(writer.ops(), [Op::Put(slot(1).terminal, lease(2))]);
}

#[test]
fn a_forgotten_lease_is_never_brought_back_by_an_older_retry() {
    let (writer, keeper) = keeper();
    writer.fail_next(1);
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    let Report::NotKept(old) = next(&keeper) else {
        panic!("expected not kept");
    };
    keeper.forget_ended(slot(1), Ended::CloseConfirmed).unwrap();
    assert!(matches!(next(&keeper), Report::Forgotten(_)));
    assert!(matches!(
        keeper.retry_save(old),
        Err(RetryRefused::Superseded)
    ));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(writer.ops(), [Op::Remove(slot(1).terminal)]);
}

#[test]
fn a_queued_older_request_is_skipped_for_a_newer_accepted_one() {
    let (writer, keeper) = keeper();
    // A first write holds the thread so the next requests wait in the queue.
    let entered = writer.block();
    keeper.keep(slot(9), lease(9)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    keeper.forget_ended(slot(1), Ended::ExitReceived).unwrap();
    writer.release();
    let reports: Vec<&str> = (0..3).map(|_| name(&next(&keeper))).collect();
    assert_eq!(reports, ["kept", "superseded", "forgotten"]);
    assert_eq!(
        writer.ops(),
        [
            Op::Put(slot(9).terminal, lease(9)),
            Op::Remove(slot(1).terminal)
        ]
    );
}

#[test]
fn a_queued_retry_is_skipped_for_a_newer_save() {
    let (writer, keeper) = keeper();
    writer.fail_next(1);
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    let Report::NotKept(old) = next(&keeper) else {
        panic!("expected not kept");
    };
    let entered = writer.block();
    keeper.keep(slot(9), lease(9)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(keeper.retry_save(old).is_ok());
    keeper.keep(slot(1), lease(2)).ok().unwrap();
    writer.release();
    let reports: Vec<&str> = (0..3).map(|_| name(&next(&keeper))).collect();
    assert_eq!(reports, ["kept", "superseded", "kept"]);
    assert_eq!(
        writer.ops(),
        [
            Op::Put(slot(9).terminal, lease(9)),
            Op::Put(slot(1).terminal, lease(2))
        ]
    );
}

#[test]
fn a_failed_removal_is_reported_until_read_and_can_be_retried() {
    let (writer, keeper) = keeper();
    writer.fail_next(1);
    keeper.forget_ended(slot(1), Ended::CloseConfirmed).unwrap();
    let Report::NotForgotten(retry) = next(&keeper) else {
        panic!("expected not forgotten");
    };
    assert!(keeper.retry_forget(retry).is_ok());
    assert!(matches!(next(&keeper), Report::Forgotten(_)));
    assert_eq!(writer.ops(), [Op::Remove(slot(1).terminal)]);
}

#[test]
fn an_epoch_change_must_be_a_real_one() {
    let (_writer, keeper) = keeper();
    assert_eq!(
        keeper.forget_ended(
            slot(1),
            Ended::EpochChanged {
                current: Epoch::from_bytes([1; 16])
            }
        ),
        Err(Refusal::InvalidProof)
    );
    assert_eq!(
        keeper.forget_ended(
            slot(1),
            Ended::EpochChanged {
                current: Epoch::from_bytes([2; 16])
            }
        ),
        Ok(())
    );
}

#[test]
fn a_full_queue_refuses_and_hands_the_lease_back() {
    let (writer, keeper) = keeper();
    let entered = writer.block();
    keeper.keep(slot(0), lease(0)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    for n in 1..=MAX_QUEUE as u16 {
        keeper.keep(slot(n), lease(1)).ok().unwrap();
    }
    let refused = keeper.keep(slot(500), lease(7));
    let Err((Refusal::Busy, back_slot, back_lease)) = refused else {
        panic!("expected busy");
    };
    assert_eq!((back_slot, back_lease), (slot(500), lease(7)));
    writer.release();
}

#[test]
fn unused_retry_tokens_hold_their_slots_until_dropped() {
    let (writer, keeper) = keeper();
    writer.fail_next(MAX_SLOTS);
    let mut tokens = Vec::new();
    for n in 0..MAX_SLOTS as u16 {
        keeper.keep(slot(n), lease(1)).ok().unwrap();
        let Report::NotKept(retry) = next(&keeper) else {
            panic!("expected not kept");
        };
        tokens.push(retry);
    }
    let refused = keeper.keep(slot(1000), lease(1));
    assert!(matches!(refused, Err((Refusal::Busy, _, _))));
    drop(tokens.pop());
    assert!(keeper.keep(slot(1000), lease(1)).is_ok());
    assert!(matches!(next(&keeper), Report::Kept(_)));
}

#[test]
fn unread_failures_are_all_kept_and_information_is_capped() {
    let (writer, keeper) = keeper();
    writer.fail_next(1);
    keeper.keep(slot(0), lease(1)).ok().unwrap();
    for n in 1..=(MAX_INFO_REPORTS as u16 + 10) {
        keeper.keep(slot(n), lease(1)).ok().unwrap();
        // Stay under the queue bound.
        let deadline = Instant::now() + Duration::from_secs(5);
        while writer.ops().len() < n as usize {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    std::thread::sleep(Duration::from_millis(50));
    let mut reports = Vec::new();
    while let Some(report) = keeper.report() {
        reports.push(name(&report));
    }
    assert_eq!(reports.iter().filter(|r| **r == "not-kept").count(), 1);
    assert_eq!(
        reports.iter().filter(|r| **r == "kept").count(),
        MAX_INFO_REPORTS
    );
}

#[test]
fn dropping_the_keeper_finishes_accepted_work_without_anyone_reading() {
    let (writer, keeper) = keeper();
    let entered = writer.block();
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    keeper.keep(slot(2), lease(2)).ok().unwrap();
    keeper.forget_ended(slot(3), Ended::ExitReceived).unwrap();
    drop(keeper);
    writer.release();
    let deadline = Instant::now() + Duration::from_secs(5);
    while writer.ops().len() < 3 {
        assert!(Instant::now() < deadline, "accepted work was dropped");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        writer.ops(),
        [
            Op::Put(slot(1).terminal, lease(1)),
            Op::Put(slot(2).terminal, lease(2)),
            Op::Remove(slot(3).terminal)
        ]
    );
}

#[test]
fn repeated_failures_on_one_slot_keep_only_the_latest_report() {
    let (writer, keeper) = keeper();
    writer.fail_next(301);
    for n in 1..=300usize {
        keeper.keep(slot(1), lease((n % 250) as u8)).ok().unwrap();
        // One at a time, under the queue bound; nobody reads the reports.
        let deadline = Instant::now() + Duration::from_secs(5);
        while writer.attempts() < n {
            assert!(Instant::now() < deadline, "the keeper stalled");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    keeper.keep(slot(2), lease(9)).ok().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while writer.attempts() < 301 {
        assert!(Instant::now() < deadline, "the keeper stalled");
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(50));
    let mut failures = Vec::new();
    while let Some(report) = keeper.report() {
        match report {
            Report::NotKept(retry) => failures.push(retry),
            other => panic!("unexpected {}", name(&other)),
        }
    }
    let slots: Vec<Slot> = failures.iter().map(|r| r.slot().clone()).collect();
    assert_eq!(
        slots,
        [slot(1), slot(2)],
        "one failure per slot, the latest"
    );
    // The latest one is still current: its retry is accepted and saves.
    let latest = failures.remove(0);
    assert!(keeper.retry_save(latest).is_ok());
    assert!(matches!(next(&keeper), Report::Kept(s) if s == slot(1)));
    assert_eq!(writer.ops(), [Op::Put(slot(1).terminal, lease(50))]);
}

// ---- Reservations -----------------------------------------------------------------------

#[test]
fn a_reservation_is_accepted_even_when_the_queue_is_full() {
    let (writer, keeper) = keeper();
    let reservation = keeper.reserve(slot(1)).ok().unwrap();
    let entered = writer.block();
    keeper.keep(slot(9), lease(9)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    // The reservation holds one place: the others fill the rest.
    for n in 10..(10 + MAX_QUEUE as u16 - 1) {
        keeper.keep(slot(n), lease(1)).ok().unwrap();
    }
    assert!(matches!(
        keeper.keep(slot(500), lease(1)),
        Err((Refusal::Busy, _, _))
    ));
    assert!(matches!(keeper.reserve(slot(501)), Err(Refusal::Busy)));
    reservation.keep(lease(2));
    writer.release();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !writer.ops().contains(&Op::Put(slot(1).terminal, lease(2))) {
        assert!(
            Instant::now() < deadline,
            "the reserved lease was not written"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_confirmed_end_after_a_reservation_skips_its_lease() {
    let (writer, keeper) = keeper();
    let reservation = keeper.reserve(slot(1)).ok().unwrap();
    keeper.forget_ended(slot(1), Ended::CloseConfirmed).unwrap();
    assert!(matches!(next(&keeper), Report::Forgotten(_)));
    reservation.keep(lease(2));
    assert!(matches!(next(&keeper), Report::Superseded(_)));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        writer.ops(),
        [Op::Remove(slot(1).terminal)],
        "never brought back"
    );
}

#[test]
fn a_dropped_reservation_leaves_an_older_retry_working() {
    let (writer, keeper) = keeper();
    writer.fail_next(1);
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    let Report::NotKept(old) = next(&keeper) else {
        panic!("expected not kept");
    };
    drop(keeper.reserve(slot(1)).ok().unwrap());
    assert!(keeper.retry_save(old).is_ok());
    assert!(matches!(next(&keeper), Report::Kept(_)));
    assert_eq!(writer.ops(), [Op::Put(slot(1).terminal, lease(1))]);
}

#[test]
fn a_dropped_reservation_leaves_a_queued_keep_running() {
    let (writer, keeper) = keeper();
    let entered = writer.block();
    keeper.keep(slot(9), lease(9)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    drop(keeper.reserve(slot(1)).ok().unwrap());
    writer.release();
    let reports: Vec<&str> = (0..2).map(|_| name(&next(&keeper))).collect();
    assert_eq!(reports, ["kept", "kept"]);
    assert_eq!(
        writer.ops(),
        [
            Op::Put(slot(9).terminal, lease(9)),
            Op::Put(slot(1).terminal, lease(1))
        ]
    );
}

#[test]
fn a_fresh_lease_wins_over_an_older_retry() {
    let (writer, keeper) = keeper();
    writer.fail_next(1);
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    let Report::NotKept(old) = next(&keeper) else {
        panic!("expected not kept");
    };
    let reservation = keeper.reserve(slot(1)).ok().unwrap();
    assert!(
        keeper.retry_save(old).is_ok(),
        "still the latest accepted request"
    );
    reservation.keep(lease(2));
    let deadline = Instant::now() + Duration::from_secs(5);
    while writer.ops().last() != Some(&Op::Put(slot(1).terminal, lease(2))) {
        assert!(
            Instant::now() < deadline,
            "the fresh lease must be the one kept"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        writer.ops().last(),
        Some(&Op::Put(slot(1).terminal, lease(2)))
    );
}

#[test]
fn a_reservation_committed_after_the_keeper_was_dropped_is_written() {
    let (writer, keeper) = keeper();
    let reservation = keeper.reserve(slot(1)).ok().unwrap();
    drop(keeper);
    std::thread::sleep(Duration::from_millis(50));
    reservation.keep(lease(3));
    let deadline = Instant::now() + Duration::from_secs(5);
    while writer.ops().is_empty() {
        assert!(Instant::now() < deadline, "the reserved work was dropped");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(writer.ops(), [Op::Put(slot(1).terminal, lease(3))]);
}

#[test]
fn reservations_are_bounded_and_given_back_when_dropped() {
    let (_writer, keeper) = keeper();
    let held: Vec<_> = (0..MAX_QUEUE as u16)
        .map(|n| keeper.reserve(slot(n)).ok().unwrap())
        .collect();
    assert!(matches!(keeper.reserve(slot(900)), Err(Refusal::Busy)));
    drop(held);
    assert!(keeper.reserve(slot(900)).is_ok());
}

#[test]
fn one_reservation_per_slot_until_it_is_committed_or_dropped() {
    let (writer, keeper) = keeper();
    let first = keeper.reserve(slot(1)).ok().unwrap();
    assert!(matches!(keeper.reserve(slot(1)), Err(Refusal::Busy)));
    // Another slot is not held back.
    drop(keeper.reserve(slot(2)).ok().unwrap());
    first.keep(lease(1));
    assert!(matches!(next(&keeper), Report::Kept(_)));
    drop(keeper.reserve(slot(1)).ok().unwrap());
    keeper.reserve(slot(1)).ok().unwrap().keep(lease(2));
    assert!(matches!(next(&keeper), Report::Kept(_)));
    assert_eq!(
        writer.ops(),
        [
            Op::Put(slot(1).terminal, lease(1)),
            Op::Put(slot(1).terminal, lease(2))
        ]
    );
}

#[test]
fn a_confirmed_end_is_accepted_while_the_slot_is_reserved() {
    let (writer, keeper) = keeper();
    let reservation = keeper.reserve(slot(1)).ok().unwrap();
    assert_eq!(keeper.forget_ended(slot(1), Ended::ExitReceived), Ok(()));
    assert!(matches!(next(&keeper), Report::Forgotten(_)));
    drop(reservation);
    assert_eq!(writer.ops(), [Op::Remove(slot(1).terminal)]);
}

#[test]
fn a_newer_failed_end_still_decides_once_its_retry_is_dropped() {
    let (writer, keeper) = keeper();
    let reservation = keeper.reserve(slot(1)).ok().unwrap();
    writer.fail_next(1);
    keeper.forget_ended(slot(1), Ended::ExitReceived).unwrap();
    let Report::NotForgotten(retry) = next(&keeper) else {
        panic!("expected not forgotten");
    };
    drop(retry);
    reservation.keep(lease(2));
    assert!(matches!(next(&keeper), Report::Superseded(_)));
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        writer.ops().is_empty(),
        "an ended terminal's lease is not saved"
    );
}

// ---- Promotes ---------------------------------------------------------------------------

fn complete(pending: &str, n: u16, lease_n: u8) -> Op {
    Op::Complete(pending.to_owned(), slot(n).terminal, lease(lease_n))
}

#[test]
fn a_promote_replaces_the_pending_record_by_the_lease() {
    let (writer, keeper) = keeper();
    keeper
        .promote(slot(1), "pending-1".into(), lease(1))
        .ok()
        .unwrap();
    assert!(matches!(next(&keeper), Report::Kept(s) if s == slot(1)));
    assert_eq!(writer.ops(), [complete("pending-1", 1, 1)]);
}

#[test]
fn a_failed_promote_is_retried_as_a_promote() {
    let (writer, keeper) = keeper();
    writer.fail_next(1);
    keeper
        .promote(slot(1), "pending-1".into(), lease(1))
        .ok()
        .unwrap();
    let Report::NotKept(retry) = next(&keeper) else {
        panic!("expected not kept");
    };
    assert!(keeper.retry_save(retry).is_ok());
    assert!(matches!(next(&keeper), Report::Kept(_)));
    assert_eq!(writer.ops(), [complete("pending-1", 1, 1)]);
}

#[test]
fn a_refused_promote_hands_everything_back() {
    let (writer, keeper) = keeper();
    let entered = writer.block();
    keeper.keep(slot(0), lease(0)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    for n in 1..=MAX_QUEUE as u16 {
        keeper.keep(slot(n), lease(1)).ok().unwrap();
    }
    let Err(refused) = keeper.promote(slot(500), "pending-500".into(), lease(7)) else {
        panic!("expected busy");
    };
    let (reason, back_slot, back_pending, back_lease) = *refused;
    assert_eq!(reason, Refusal::Busy);
    assert_eq!(
        (back_slot, back_pending.as_str(), back_lease),
        (slot(500), "pending-500", lease(7))
    );
    writer.release();
}

#[test]
fn a_promote_skipped_for_a_newer_save_leaves_its_pending_record() {
    let (writer, keeper) = keeper();
    let entered = writer.block();
    keeper.keep(slot(9), lease(9)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    keeper
        .promote(slot(1), "pending-1".into(), lease(1))
        .ok()
        .unwrap();
    keeper.keep(slot(1), lease(2)).ok().unwrap();
    writer.release();
    let reports: Vec<&str> = (0..3).map(|_| name(&next(&keeper))).collect();
    assert_eq!(reports, ["kept", "superseded", "kept"]);
    // Nothing touched the pending record: no complete, and no other write for it.
    assert_eq!(
        writer.ops(),
        [
            Op::Put(slot(9).terminal, lease(9)),
            Op::Put(slot(1).terminal, lease(2))
        ]
    );
}

#[test]
fn an_older_failed_promote_is_refused_after_a_newer_save() {
    let (writer, keeper) = keeper();
    writer.fail_next(1);
    keeper
        .promote(slot(1), "pending-1".into(), lease(1))
        .ok()
        .unwrap();
    let Report::NotKept(old) = next(&keeper) else {
        panic!("expected not kept");
    };
    keeper.keep(slot(1), lease(2)).ok().unwrap();
    assert!(matches!(next(&keeper), Report::Kept(_)));
    assert!(matches!(
        keeper.retry_save(old),
        Err(RetryRefused::Superseded)
    ));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(writer.ops(), [Op::Put(slot(1).terminal, lease(2))]);
}

#[test]
fn a_confirmed_end_after_a_promote_is_never_undone() {
    let (writer, keeper) = keeper();
    let entered = writer.block();
    keeper.keep(slot(9), lease(9)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    keeper
        .promote(slot(1), "pending-1".into(), lease(1))
        .ok()
        .unwrap();
    keeper.forget_ended(slot(1), Ended::ExitReceived).unwrap();
    writer.release();
    let reports: Vec<&str> = (0..3).map(|_| name(&next(&keeper))).collect();
    assert_eq!(reports, ["kept", "superseded", "forgotten"]);
    assert_eq!(
        writer.ops(),
        [
            Op::Put(slot(9).terminal, lease(9)),
            Op::Remove(slot(1).terminal)
        ]
    );
}

#[test]
fn a_busy_promote_retry_keeps_its_pending_record() {
    let (writer, keeper) = keeper();
    writer.fail_next(1);
    keeper
        .promote(slot(1), "pending-1".into(), lease(1))
        .ok()
        .unwrap();
    let Report::NotKept(retry) = next(&keeper) else {
        panic!("expected not kept");
    };
    // Fill the queue behind a blocked write, so the retry is refused and handed back.
    let entered = writer.block();
    keeper.keep(slot(900), lease(9)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    for n in 10..(10 + MAX_QUEUE as u16) {
        keeper.keep(slot(n), lease(1)).ok().unwrap();
    }
    let Err(RetryRefused::Busy(retry)) = keeper.retry_save(retry) else {
        panic!("expected busy");
    };
    writer.release();
    let deadline = Instant::now() + Duration::from_secs(5);
    while writer.ops().len() < 1 + MAX_QUEUE {
        assert!(Instant::now() < deadline, "the queue did not drain");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Handed back whole: the retry still completes the same pending record.
    assert!(keeper.retry_save(retry).is_ok());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !writer.ops().contains(&complete("pending-1", 1, 1)) {
        assert!(Instant::now() < deadline, "the promote was not retried");
        std::thread::sleep(Duration::from_millis(5));
    }
}

// ---- Places and creation leases ---------------------------------------------------------

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_place_promotes_on_a_fresh_slot() {
    let (writer, keeper) = keeper();
    let place = keeper.reserve_new().ok().unwrap();
    assert_eq!(
        place.promote(slot(1), "pending-1".into(), lease(1)),
        Promoted::Queued
    );
    assert!(matches!(next(&keeper), Report::Kept(s) if s == slot(1)));
    assert_eq!(writer.ops(), [complete("pending-1", 1, 1)]);
}

#[test]
fn a_takeover_that_fails_leaves_the_creation_lease_saved() {
    let (writer, keeper) = keeper();
    let place = keeper.reserve_new().ok().unwrap();
    let takeover = keeper.reserve(slot(1)).ok().unwrap();
    assert_eq!(
        place.promote(slot(1), "pending-1".into(), lease(1)),
        Promoted::Queued,
        "a reservation in flight proves no newer lease"
    );
    drop(takeover);
    assert!(matches!(next(&keeper), Report::Kept(_)));
    assert_eq!(writer.ops(), [complete("pending-1", 1, 1)]);
}

#[test]
fn a_committed_takeover_wins_over_a_queued_creation_lease_whatever_the_stamps() {
    for takeover_first in [true, false] {
        let (writer, keeper) = keeper();
        let entered = writer.block();
        keeper.keep(slot(9), lease(9)).ok().unwrap();
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let (place, takeover) = if takeover_first {
            let takeover = keeper.reserve(slot(1)).ok().unwrap();
            (keeper.reserve_new().ok().unwrap(), takeover)
        } else {
            let place = keeper.reserve_new().ok().unwrap();
            (place, keeper.reserve(slot(1)).ok().unwrap())
        };
        assert_eq!(
            place.promote(slot(1), "pending-1".into(), lease(1)),
            Promoted::Queued
        );
        takeover.keep(lease(2));
        writer.release();
        let reports: Vec<&str> = (0..3).map(|_| name(&next(&keeper))).collect();
        assert_eq!(reports, ["kept", "superseded", "kept"]);
        assert_eq!(
            writer.ops(),
            [
                Op::Put(slot(9).terminal, lease(9)),
                Op::Put(slot(1).terminal, lease(2))
            ]
        );
    }
}

#[test]
fn a_takeover_after_the_creation_lease_was_written_overwrites_it() {
    let (writer, keeper) = keeper();
    let place = keeper.reserve_new().ok().unwrap();
    let takeover = keeper.reserve(slot(1)).ok().unwrap();
    assert_eq!(
        place.promote(slot(1), "pending-1".into(), lease(1)),
        Promoted::Queued
    );
    assert!(matches!(next(&keeper), Report::Kept(_)));
    takeover.keep(lease(2));
    assert!(matches!(next(&keeper), Report::Kept(_)));
    assert_eq!(
        writer.ops(),
        [
            complete("pending-1", 1, 1),
            Op::Put(slot(1).terminal, lease(2))
        ]
    );
}

#[test]
fn a_confirmed_close_after_a_creation_lease_forgets_it() {
    let (writer, keeper) = keeper();
    let entered = writer.block();
    keeper.keep(slot(9), lease(9)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    let place = keeper.reserve_new().ok().unwrap();
    let close = keeper.reserve(slot(1)).ok().unwrap();
    assert_eq!(
        place.promote(slot(1), "pending-1".into(), lease(1)),
        Promoted::Queued
    );
    assert!(close.forget(Ended::CloseConfirmed).is_ok());
    writer.release();
    let reports: Vec<&str> = (0..3).map(|_| name(&next(&keeper))).collect();
    assert_eq!(reports, ["kept", "superseded", "forgotten"]);
    assert_eq!(
        writer.ops(),
        [
            Op::Put(slot(9).terminal, lease(9)),
            Op::Remove(slot(1).terminal)
        ]
    );
}

#[test]
fn an_end_written_while_the_place_was_held_supersedes_a_late_promote() {
    let (writer, keeper) = keeper();
    let place = keeper.reserve_new().ok().unwrap();
    keeper.forget_ended(slot(1), Ended::ExitReceived).unwrap();
    assert!(matches!(next(&keeper), Report::Forgotten(_)));
    assert_eq!(
        place.promote(slot(1), "pending-1".into(), lease(1)),
        Promoted::Superseded
    );
    assert!(matches!(next(&keeper), Report::Superseded(_)));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        writer.ops(),
        [Op::Remove(slot(1).terminal)],
        "never brought back"
    );
}

#[test]
fn a_queued_save_supersedes_a_promote() {
    let (writer, keeper) = keeper();
    let entered = writer.block();
    keeper.keep(slot(9), lease(9)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    keeper.keep(slot(1), lease(2)).ok().unwrap();
    let place = keeper.reserve_new().ok().unwrap();
    assert_eq!(
        place.promote(slot(1), "pending-1".into(), lease(1)),
        Promoted::Superseded
    );
    writer.release();
    wait_for("both saves", || writer.ops().len() == 2);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        writer.ops(),
        [
            Op::Put(slot(9).terminal, lease(9)),
            Op::Put(slot(1).terminal, lease(2))
        ]
    );
}

#[test]
fn a_retry_of_a_creation_lease_after_a_takeover_is_superseded() {
    let (writer, keeper) = keeper();
    writer.fail_next(1);
    let place = keeper.reserve_new().ok().unwrap();
    assert_eq!(
        place.promote(slot(1), "pending-1".into(), lease(1)),
        Promoted::Queued
    );
    let Report::NotKept(retry) = next(&keeper) else {
        panic!("expected not kept");
    };
    let takeover = keeper.reserve(slot(1)).ok().unwrap();
    takeover.keep(lease(2));
    assert!(matches!(next(&keeper), Report::Kept(_)));
    assert!(matches!(
        keeper.retry_save(retry),
        Err(RetryRefused::Superseded)
    ));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(writer.ops(), [Op::Put(slot(1).terminal, lease(2))]);
}

#[test]
fn a_takeover_that_wins_clears_the_creation_lease_failure_left_unread() {
    let (writer, keeper) = keeper();
    writer.fail_next(1);
    // Taken before the place: its stamp is older than the creation lease's.
    let takeover = keeper.reserve(slot(1)).ok().unwrap();
    let place = keeper.reserve_new().ok().unwrap();
    assert_eq!(
        place.promote(slot(1), "pending-1".into(), lease(1)),
        Promoted::Queued
    );
    wait_for("the failed creation write", || writer.attempts() == 1);
    // Its failure report is delivered right after the write returned.
    std::thread::sleep(Duration::from_millis(50));
    takeover.keep(lease(2));
    wait_for("the takeover's save", || !writer.ops().is_empty());
    std::thread::sleep(Duration::from_millis(50));
    let mut reports = Vec::new();
    while let Some(report) = keeper.report() {
        reports.push(name(&report));
    }
    assert_eq!(reports, ["kept"], "the out-of-date failure went with it");
    assert_eq!(writer.ops(), [Op::Put(slot(1).terminal, lease(2))]);
}

#[test]
fn a_direct_promote_after_an_accepted_save_is_skipped_and_reported() {
    let (writer, keeper) = keeper();
    let entered = writer.block();
    keeper.keep(slot(9), lease(9)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    keeper.keep(slot(1), lease(2)).ok().unwrap();
    assert!(
        keeper
            .promote(slot(1), "pending-1".into(), lease(1))
            .is_ok()
    );
    assert!(matches!(next(&keeper), Report::Superseded(_)));
    writer.release();
    let reports: Vec<&str> = (0..2).map(|_| name(&next(&keeper))).collect();
    assert_eq!(reports, ["kept", "kept"]);
    assert_eq!(
        writer.ops(),
        [
            Op::Put(slot(9).terminal, lease(9)),
            Op::Put(slot(1).terminal, lease(2))
        ]
    );
}

#[test]
fn places_hold_queue_places() {
    let (_writer, keeper) = keeper();
    let held: Vec<_> = (0..MAX_QUEUE)
        .map(|_| keeper.reserve_new().ok().unwrap())
        .collect();
    assert!(matches!(keeper.reserve_new(), Err(Refusal::Busy)));
    assert!(matches!(keeper.reserve(slot(1)), Err(Refusal::Busy)));
    assert!(matches!(
        keeper.keep(slot(1), lease(1)),
        Err((Refusal::Busy, _, _))
    ));
    drop(held);
    assert!(keeper.reserve_new().is_ok());
}

#[test]
fn places_and_tombstones_hold_slot_table_places() {
    let (writer, keeper) = keeper();
    writer.fail_next(MAX_SLOTS - 1);
    let mut tokens = Vec::new();
    for n in 0..(MAX_SLOTS - 1) as u16 {
        keeper.keep(slot(n), lease(1)).ok().unwrap();
        let Report::NotKept(retry) = next(&keeper) else {
            panic!("expected not kept");
        };
        tokens.push(retry);
    }
    let place = keeper.reserve_new().ok().unwrap();
    assert!(
        matches!(
            keeper.keep(slot(1000), lease(1)),
            Err((Refusal::Busy, _, _))
        ),
        "no place left for a new slot"
    );
    assert!(matches!(keeper.reserve_new(), Err(Refusal::Busy)));
    assert!(
        keeper.keep(slot(0), lease(2)).is_ok(),
        "a followed slot needs no new place"
    );
    assert!(matches!(next(&keeper), Report::Kept(_)));
    // Written, that save still holds its place as a tombstone while the place is held.
    assert!(matches!(
        keeper.keep(slot(1000), lease(1)),
        Err((Refusal::Busy, _, _))
    ));
    drop(place);
    assert!(keeper.keep(slot(1000), lease(1)).is_ok());
    drop(tokens);
}

#[test]
fn tombstones_go_with_the_oldest_place() {
    let (_writer, keeper) = keeper();
    let first = keeper.reserve_new().ok().unwrap();
    let second = keeper.reserve_new().ok().unwrap();
    keeper.forget_ended(slot(1), Ended::ExitReceived).unwrap();
    assert!(matches!(next(&keeper), Report::Forgotten(_)));
    drop(first);
    assert_eq!(
        second.promote(slot(1), "pending-1".into(), lease(1)),
        Promoted::Superseded,
        "the end is newer than the place still held"
    );
    // No place held any more: the tombstone is gone, a new place starts clean.
    let third = keeper.reserve_new().ok().unwrap();
    assert_eq!(
        third.promote(slot(1), "pending-1".into(), lease(1)),
        Promoted::Queued
    );
}

#[test]
fn a_place_promoted_after_the_keeper_was_dropped_is_written() {
    let (writer, keeper) = keeper();
    let place = keeper.reserve_new().ok().unwrap();
    drop(keeper);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        place.promote(slot(1), "pending-1".into(), lease(1)),
        Promoted::Queued
    );
    wait_for("the creation lease", || !writer.ops().is_empty());
    assert_eq!(writer.ops(), [complete("pending-1", 1, 1)]);
}

#[test]
fn idle_only_once_nothing_is_queued_reserved_placed_or_written() {
    let (writer, keeper) = keeper();
    assert!(keeper.is_idle());
    let entered = writer.block();
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!keeper.is_idle(), "a write in flight");
    writer.release();
    wait_for("idle", || keeper.is_idle());
    assert!(
        matches!(keeper.report(), Some(Report::Kept(_))),
        "idle only once its report is delivered"
    );
    let reservation = keeper.reserve(slot(2)).ok().unwrap();
    assert!(!keeper.is_idle(), "a reservation held");
    drop(reservation);
    let place = keeper.reserve_new().ok().unwrap();
    assert!(!keeper.is_idle(), "a place held");
    drop(place);
    assert!(keeper.is_idle());
}

#[test]
fn closed_admissions_refuse_new_requests_but_take_what_is_held() {
    let (writer, keeper) = keeper();
    let reservation = keeper.reserve(slot(1)).ok().unwrap();
    let place = keeper.reserve_new().ok().unwrap();
    writer.fail_next(1);
    keeper.keep(slot(3), lease(3)).ok().unwrap();
    let Report::NotKept(retry) = next(&keeper) else {
        panic!("expected not kept");
    };
    keeper.close_admissions();
    keeper.close_admissions();
    assert!(keeper.admissions_closed());
    // Nothing new that would come before a request to a host.
    assert!(matches!(keeper.reserve(slot(2)), Err(Refusal::Closed)));
    assert!(matches!(keeper.reserve_new(), Err(Refusal::Closed)));
    assert!(!keeper.is_idle(), "what is held still counts");
    // What is held commits; saves, removals and retries are still taken.
    reservation.keep(lease(1));
    assert_eq!(
        place.promote(slot(4), "pending-4".into(), lease(4)),
        Promoted::Queued
    );
    assert!(keeper.keep(slot(5), lease(5)).is_ok());
    assert!(keeper.forget_ended(slot(6), Ended::ExitReceived).is_ok());
    assert!(keeper.retry_save(retry).is_ok());
    wait_for("idle", || keeper.is_idle());
    let ops = writer.ops();
    for op in [
        Op::Put(slot(1).terminal, lease(1)),
        complete("pending-4", 4, 4),
        Op::Put(slot(5).terminal, lease(5)),
        Op::Remove(slot(6).terminal),
        Op::Put(slot(3).terminal, lease(3)),
    ] {
        assert!(ops.contains(&op), "{op:?} written");
    }
}
