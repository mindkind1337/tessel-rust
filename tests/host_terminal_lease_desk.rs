//! The lease desk with a real keeper and a synthetic writer: one failure token per slot shared
//! by its panes, retries with the token's own revision, proofs of end that wait for room,
//! bounds that hand ownership back, and what stays unsettled for the exit dialog.
#[allow(dead_code)]
#[path = "../src/terminal/host_commands.rs"]
mod host_commands;
#[allow(dead_code)]
#[path = "../src/terminal/host_handoff.rs"]
mod host_handoff;
#[allow(dead_code)]
#[path = "../src/terminal/host_lease_desk.rs"]
mod host_lease_desk;
#[allow(dead_code)]
#[path = "../src/terminal/host_lease_keeper.rs"]
mod host_lease_keeper;
#[allow(dead_code)]
#[path = "../src/terminal/host_opening.rs"]
mod host_opening;
#[allow(dead_code)]
#[path = "../src/terminal/host_worker.rs"]
mod host_worker;

use host_lease_desk::{LeaseDesk, ProofRefused, Retried, View};
use host_lease_keeper::{Ended, Keeper, LeaseWriter, MAX_QUEUE, MAX_SLOTS};
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

    fn write(&self, op: Op) -> Result<(), LocalError> {
        let mut state = self.state.lock().unwrap();
        if let Some(entered) = state.entered.take() {
            let _ = entered.send(());
        }
        while state.blocked {
            state = self.released.wait(state).unwrap();
        }
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
    let mut bytes = [0x20u8; 16];
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

fn setup() -> (Arc<FakeWriter>, Arc<Keeper>, LeaseDesk) {
    let writer = Arc::new(FakeWriter::default());
    let keeper = Arc::new(Keeper::start(writer.clone()).unwrap());
    let desk = LeaseDesk::new(keeper.clone());
    (writer, keeper, desk)
}

/// Runs frames until `done`.
fn until(desk: &LeaseDesk, what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        desk.frame();
        if done() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Holds the keeper's writer on one write and fills its queue behind it.
fn fill(writer: &FakeWriter, keeper: &Keeper) {
    let entered = writer.block();
    keeper.keep(slot(900), lease(9)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    for n in 10..(10 + MAX_QUEUE as u16) {
        keeper.keep(slot(n), lease(1)).ok().unwrap();
    }
}

#[test]
fn panes_of_one_slot_share_its_single_failure_token() {
    let (writer, keeper, desk) = setup();
    let (a, b) = (desk.handle(), desk.handle());
    a.register(&slot(1)).unwrap();
    b.register(&slot(1)).unwrap();
    writer.fail_next(1);
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    until(&desk, "the failure", || a.view(&slot(1)) == View::NotSaved);
    assert_eq!(b.view(&slot(1)), View::NotSaved);
    assert_eq!(b.retry(&slot(1)), Retried::Queued);
    assert_eq!(
        a.retry(&slot(1)),
        Retried::Nothing,
        "one token, already used"
    );
    assert_eq!(a.view(&slot(1)), View::Retrying);
    until(&desk, "the save", || a.view(&slot(1)) == View::Saved);
    assert_eq!(b.view(&slot(1)), View::Saved);
    assert_eq!(writer.ops(), [Op::Put(slot(1).terminal, lease(1))]);
}

#[test]
fn a_failure_outlives_every_pane_that_showed_it() {
    let (writer, keeper, desk) = setup();
    let pane = desk.handle();
    pane.register(&slot(1)).unwrap();
    writer.fail_next(1);
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    until(&desk, "the failure", || {
        pane.view(&slot(1)) == View::NotSaved
    });
    pane.unregister(&slot(1));
    desk.frame();
    assert_eq!(
        pane.view(&slot(1)),
        View::NotSaved,
        "kept for the exit dialog"
    );
    assert_eq!(pane.unsettled().not_saved, 1);
    assert_eq!(pane.retry(&slot(1)), Retried::Queued);
    until(&desk, "the save", || writer.ops().len() == 1);
    until(&desk, "nothing left", || pane.unsettled().is_empty());
    assert_eq!(
        pane.view(&slot(1)),
        View::Unknown,
        "settled and shown nowhere"
    );
}

#[test]
fn a_retry_without_room_keeps_its_token() {
    let (writer, keeper, desk) = setup();
    let pane = desk.handle();
    pane.register(&slot(1)).unwrap();
    writer.fail_next(1);
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    until(&desk, "the failure", || {
        pane.view(&slot(1)) == View::NotSaved
    });
    fill(&writer, &keeper);
    assert_eq!(pane.retry(&slot(1)), Retried::Busy);
    assert_eq!(pane.view(&slot(1)), View::NotSaved, "the token stayed here");
    writer.release();
    until(&desk, "the queue to drain", || {
        writer.ops().len() > MAX_QUEUE
    });
    assert_eq!(pane.retry(&slot(1)), Retried::Queued);
    until(&desk, "the save", || pane.view(&slot(1)) == View::Saved);
    assert!(writer.ops().contains(&Op::Put(slot(1).terminal, lease(1))));
}

#[test]
fn a_newer_save_replaces_an_older_failure() {
    let (writer, keeper, desk) = setup();
    let pane = desk.handle();
    pane.register(&slot(1)).unwrap();
    writer.fail_next(1);
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    until(&desk, "the failure", || {
        pane.view(&slot(1)) == View::NotSaved
    });
    keeper.keep(slot(1), lease(2)).ok().unwrap();
    until(&desk, "the newer save", || {
        pane.view(&slot(1)) == View::Saved
    });
    assert_eq!(pane.retry(&slot(1)), Retried::Nothing);
    assert_eq!(writer.ops(), [Op::Put(slot(1).terminal, lease(2))]);
}

#[test]
fn a_retry_after_a_newer_request_is_superseded_and_writes_nothing() {
    let (writer, keeper, desk) = setup();
    let pane = desk.handle();
    pane.register(&slot(1)).unwrap();
    writer.fail_next(1);
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    until(&desk, "the failure", || {
        pane.view(&slot(1)) == View::NotSaved
    });
    let entered = writer.block();
    keeper.keep(slot(9), lease(9)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    keeper.keep(slot(1), lease(2)).ok().unwrap();
    // The old token keeps its revision: the newer request wins.
    assert_eq!(pane.retry(&slot(1)), Retried::Superseded);
    writer.release();
    until(&desk, "the newer save", || {
        pane.view(&slot(1)) == View::Saved
    });
    assert_eq!(
        writer.ops(),
        [
            Op::Put(slot(9).terminal, lease(9)),
            Op::Put(slot(1).terminal, lease(2))
        ]
    );
}

#[test]
fn a_failed_removal_is_retried_from_the_desk() {
    let (writer, keeper, desk) = setup();
    let pane = desk.handle();
    pane.register(&slot(1)).unwrap();
    writer.fail_next(1);
    keeper.forget_ended(slot(1), Ended::ExitReceived).unwrap();
    until(&desk, "the failure", || {
        pane.view(&slot(1)) == View::NotForgotten
    });
    assert_eq!(pane.unsettled().not_forgotten, 1);
    assert_eq!(pane.retry(&slot(1)), Retried::Queued);
    until(&desk, "the removal", || {
        pane.view(&slot(1)) == View::Forgotten
    });
    assert_eq!(writer.ops(), [Op::Remove(slot(1).terminal)]);
}

#[test]
fn a_proof_without_room_waits_here_and_is_recorded_later() {
    let (writer, keeper, desk) = setup();
    let pane = desk.handle();
    pane.register(&slot(1)).unwrap();
    fill(&writer, &keeper);
    assert!(pane.hand_proof(slot(1), Ended::ExitReceived).is_ok());
    assert_eq!(pane.view(&slot(1)), View::Ending);
    assert_eq!(pane.unsettled().ends_waiting, 1);
    // The same end again: one waiting proof per slot.
    assert!(pane.hand_proof(slot(1), Ended::CloseConfirmed).is_ok());
    assert_eq!(pane.unsettled().ends_waiting, 1);
    writer.release();
    until(&desk, "the removal", || {
        pane.view(&slot(1)) == View::Forgotten
    });
    assert!(writer.ops().contains(&Op::Remove(slot(1).terminal)));
    assert!(pane.unsettled().is_empty());
}

#[test]
fn a_proof_with_room_is_admitted_at_once() {
    let (writer, _keeper, desk) = setup();
    let pane = desk.handle();
    pane.register(&slot(1)).unwrap();
    assert!(pane.hand_proof(slot(1), Ended::CloseConfirmed).is_ok());
    assert_eq!(pane.unsettled().ends_waiting, 0, "nothing waits");
    until(&desk, "the removal", || {
        pane.view(&slot(1)) == View::Forgotten
    });
    assert_eq!(writer.ops(), [Op::Remove(slot(1).terminal)]);
}

#[test]
fn waiting_proofs_are_bounded_and_handed_back_whole() {
    let (writer, keeper, desk) = setup();
    let pane = desk.handle();
    fill(&writer, &keeper);
    for n in 1000..(1000 + MAX_SLOTS as u16) {
        assert!(pane.hand_proof(slot(n), Ended::ExitReceived).is_ok());
    }
    assert_eq!(pane.unsettled().ends_waiting, MAX_SLOTS);
    let Err(ProofRefused::Full(back, Ended::ExitReceived)) =
        pane.hand_proof(slot(5000), Ended::ExitReceived)
    else {
        panic!("expected the proof handed back");
    };
    assert_eq!(back, slot(5000));
    // A caller that cannot keep it counts it, for an explicit cleanup.
    pane.count_unrecorded_end();
    assert_eq!(pane.unsettled().unrecorded_ends, 1);
    writer.release();
    until(&desk, "every waiting proof", || {
        pane.unsettled().ends_waiting == 0
    });
    until(&desk, "every removal", || {
        writer
            .ops()
            .iter()
            .filter(|op| matches!(op, Op::Remove(_)))
            .count()
            == MAX_SLOTS
    });
}

#[test]
fn a_proof_that_proves_nothing_is_refused() {
    let (writer, _keeper, desk) = setup();
    let pane = desk.handle();
    let refused = pane.hand_proof(
        slot(1),
        Ended::EpochChanged {
            current: Epoch::from_bytes([1; 16]),
        },
    );
    assert!(matches!(refused, Err(ProofRefused::Invalid)));
    std::thread::sleep(Duration::from_millis(50));
    desk.frame();
    assert!(writer.ops().is_empty());
}

#[test]
fn a_settled_slot_no_pane_shows_is_let_go() {
    let (writer, keeper, desk) = setup();
    let pane = desk.handle();
    pane.register(&slot(1)).unwrap();
    keeper.keep(slot(1), lease(1)).ok().unwrap();
    until(&desk, "the save", || pane.view(&slot(1)) == View::Saved);
    pane.unregister(&slot(1));
    assert_eq!(pane.view(&slot(1)), View::Unknown);
    // Reports of slots no pane shows leave nothing behind either.
    keeper.keep(slot(2), lease(2)).ok().unwrap();
    until(&desk, "the other save", || writer.ops().len() == 2);
    std::thread::sleep(Duration::from_millis(50));
    desk.frame();
    assert_eq!(pane.view(&slot(2)), View::Unknown);
    assert!(pane.unsettled().is_empty());
}

#[test]
fn a_proof_with_room_goes_on_while_new_slots_find_none() {
    let (writer, keeper, desk) = setup();
    let pane = desk.handle();
    pane.register(&slot(1)).unwrap();
    // Every place of the keeper's slot table held by a failure token kept in the desk.
    writer.fail_next(MAX_SLOTS);
    for n in 1..=MAX_SLOTS as u16 {
        keeper.keep(slot(n), lease(1)).ok().unwrap();
        until(&desk, "a failure", || pane.view(&slot(n)) == View::NotSaved);
    }
    // A full queue of slots the keeper already follows; their writes fail too, so the table
    // stays full.
    let entered = writer.block();
    keeper.keep(slot(2), lease(2)).ok().unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    for n in 3..(3 + MAX_QUEUE as u16) {
        keeper.keep(slot(n), lease(2)).ok().unwrap();
    }
    // Waiting: proofs for new slots, which will find no place in the table, and one for slot
    // 1, which the keeper follows and which needs no new place.
    for n in 3000..3100 {
        assert!(pane.hand_proof(slot(n), Ended::ExitReceived).is_ok());
    }
    assert!(pane.hand_proof(slot(1), Ended::ExitReceived).is_ok());
    assert_eq!(pane.unsettled().ends_waiting, 101);
    writer.fail_next(1 + MAX_QUEUE);
    writer.release();
    until(&desk, "slot 1's end past the refused new slots", || {
        pane.view(&slot(1)) == View::Forgotten
    });
    assert!(writer.ops().contains(&Op::Remove(slot(1).terminal)));
}
