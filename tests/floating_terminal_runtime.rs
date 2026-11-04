#[allow(dead_code)]
#[path = "../src/floating_terminal_runtime.rs"]
mod floating_terminal_runtime;

use floating_terminal_runtime::{
    Controller, MAX_ERROR_BYTES, MAX_WORKERS, Phase, Process, Refused, Work,
};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

const PATIENCE: Duration = Duration::from_secs(20);

type Make = Box<dyn FnOnce() -> Result<Fake, String> + Send>;

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !done() {
        assert!(Instant::now() < deadline, "never happened: {what}");
        thread::sleep(Duration::from_millis(1));
    }
}

fn poll_until(controller: &mut Controller<Fake>, phase: Phase) {
    until(&format!("{phase:?}"), || {
        controller.poll();
        controller.phase() == phase
    });
}

fn shown(controller: &mut Controller<Fake>) -> Option<usize> {
    controller.process().map(|process| process.id)
}

/// A door a worker waits at until the test opens it. A controller that waited at one on the UI
/// thread would fail the test by the patience's end.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
    reached: AtomicBool,
}

impl Gate {
    fn wait(&self) {
        self.reached.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + PATIENCE;
        let mut open = self.open.lock().unwrap();
        while !*open {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "the gate never opened");
            open = self.changed.wait_timeout(open, left).unwrap().0;
        }
    }

    fn wait_reached(&self) {
        until("gate reached", || self.reached.load(Ordering::SeqCst));
    }

    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

fn gate() -> Arc<Gate> {
    Arc::new(Gate::default())
}

/// What the programs did, in order, and on which thread.
#[derive(Default)]
struct Probe {
    events: Mutex<Vec<(String, ThreadId)>>,
}

impl Probe {
    fn note(&self, event: String) {
        self.events
            .lock()
            .unwrap()
            .push((event, thread::current().id()));
    }
}

#[derive(Default)]
struct Plan {
    make_gate: Option<Arc<Gate>>,
    stop_gate: Option<Arc<Gate>>,
    drop_gate: Option<Arc<Gate>>,
    stop_error: Option<&'static str>,
    stop_panics: bool,
}

struct Fake {
    id: usize,
    probe: Arc<Probe>,
    plan: Plan,
}

impl Process for Fake {
    fn stop(&mut self) -> Result<(), String> {
        self.probe.note(format!("stop {}", self.id));
        if let Some(gate) = &self.plan.stop_gate {
            gate.wait();
        }
        if self.plan.stop_panics {
            panic!("synthetic panic in stop");
        }
        match self.plan.stop_error.take() {
            Some(error) => Err(error.to_string()),
            None => Ok(()),
        }
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.probe.note(format!("drop {}", self.id));
        if let Some(gate) = &self.plan.drop_gate {
            gate.wait();
        }
    }
}

fn counting(wakes: Arc<AtomicUsize>) -> impl Fn() + Send + Sync + 'static {
    move || {
        wakes.fetch_add(1, Ordering::SeqCst);
    }
}

struct Rig {
    probe: Arc<Probe>,
    wakes: Arc<AtomicUsize>,
    pool: &'static AtomicUsize,
    ui: ThreadId,
}

impl Rig {
    fn new() -> Self {
        Self {
            probe: Arc::default(),
            wakes: Arc::default(),
            pool: Box::leak(Box::new(AtomicUsize::new(0))),
            ui: thread::current().id(),
        }
    }

    fn controller(&self) -> Controller<Fake> {
        Controller::bounded_by(counting(self.wakes.clone()), self.pool)
    }

    /// While `refuse` is set, no thread can start.
    fn controller_refusing(&self, refuse: &Arc<AtomicBool>) -> Controller<Fake> {
        let refuse = refuse.clone();
        Controller::with_spawn(
            counting(self.wakes.clone()),
            self.pool,
            move |work: Work| -> io::Result<()> {
                if refuse.load(Ordering::SeqCst) {
                    return Err(io::Error::other("no thread"));
                }
                drop(thread::spawn(work));
                Ok(())
            },
        )
    }

    fn make(&self, id: usize) -> Make {
        self.make_as(id, Plan::default())
    }

    fn make_as(&self, id: usize, plan: Plan) -> Make {
        let probe = self.probe.clone();
        Box::new(move || -> Result<Fake, String> {
            probe.note(format!("make {id}"));
            if let Some(gate) = &plan.make_gate {
                gate.wait();
            }
            Ok(Fake { id, probe, plan })
        })
    }

    fn failing(&self, id: usize, error: String) -> Make {
        let probe = self.probe.clone();
        Box::new(move || -> Result<Fake, String> {
            probe.note(format!("make {id}"));
            Err(error)
        })
    }

    fn log(&self) -> Vec<String> {
        let events = self.probe.events.lock().unwrap();
        events.iter().map(|(event, _)| event.clone()).collect()
    }

    fn saw(&self, event: &str) -> bool {
        self.log().iter().any(|seen| seen == event)
    }

    fn assert_nothing_on_the_ui(&self) {
        let events = self.probe.events.lock().unwrap();
        for (event, thread) in events.iter() {
            assert_ne!(*thread, self.ui, "{event} ran on the UI thread");
        }
    }

    fn wakes(&self) -> usize {
        self.wakes.load(Ordering::SeqCst)
    }

    fn workers(&self) -> usize {
        self.pool.load(Ordering::SeqCst)
    }

    fn workers_ended(&self) {
        until("every worker ended", || self.workers() == 0);
    }

    /// A controller with program 1 ready.
    fn ready(&self, plan: Plan) -> Controller<Fake> {
        let mut controller = self.controller();
        controller.open(self.make_as(1, plan)).unwrap();
        poll_until(&mut controller, Phase::Ready);
        // The wake comes after the publication: counted only once the worker ended.
        self.workers_ended();
        controller
    }
}

// Opening.

#[test]
fn open_makes_the_program_off_the_ui_and_gives_it_only_once_ready() {
    let rig = Rig::new();
    let mut controller = rig.controller();
    assert_eq!(controller.phase(), Phase::Closed);
    let door = gate();
    let plan = Plan {
        make_gate: Some(door.clone()),
        ..Plan::default()
    };
    controller.open(rig.make_as(1, plan)).unwrap();
    assert_eq!(controller.phase(), Phase::Opening);
    assert_eq!(shown(&mut controller), None);
    door.wait_reached();
    controller.poll();
    assert_eq!(controller.phase(), Phase::Opening);
    assert_eq!(rig.wakes(), 0);

    door.release();
    until("woken", || rig.wakes() == 1);
    assert_eq!(controller.phase(), Phase::Opening, "only poll changes it");
    assert_eq!(shown(&mut controller), None);
    controller.poll();
    assert_eq!(controller.phase(), Phase::Ready);
    assert_eq!(shown(&mut controller), Some(1));
    assert_eq!(controller.error(), None);
    rig.assert_nothing_on_the_ui();
    rig.workers_ended();
}

#[test]
fn a_failed_open_is_shown_and_never_retried() {
    let rig = Rig::new();
    let mut controller = rig.controller();
    controller
        .open(rig.failing(1, "no shell".to_string()))
        .unwrap();
    poll_until(&mut controller, Phase::Closed);
    assert_eq!(controller.error(), Some("no shell"));
    rig.workers_ended();
    assert_eq!(rig.wakes(), 1);
    controller.poll();
    assert_eq!(rig.log(), ["make 1"]);

    controller.open(rig.make(2)).unwrap();
    assert_eq!(controller.error(), None, "an accepted request clears it");
    poll_until(&mut controller, Phase::Ready);
    assert_eq!(shown(&mut controller), Some(2));
}

// Restarting.

#[test]
fn restart_stops_then_drops_then_makes_the_next_program() {
    let rig = Rig::new();
    let stop_door = gate();
    let drop_door = gate();
    let mut controller = rig.ready(Plan {
        stop_gate: Some(stop_door.clone()),
        drop_gate: Some(drop_door.clone()),
        ..Plan::default()
    });

    controller.restart(rig.make(2)).unwrap();
    assert_eq!(controller.phase(), Phase::Restarting);
    assert_eq!(shown(&mut controller), None, "out of the UI at once");
    stop_door.wait_reached();
    assert_eq!(rig.log(), ["make 1", "stop 1"]);

    stop_door.release();
    drop_door.wait_reached();
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1"]);
    controller.poll();
    assert_eq!(controller.phase(), Phase::Restarting);

    drop_door.release();
    poll_until(&mut controller, Phase::Ready);
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1", "make 2"]);
    assert_eq!(shown(&mut controller), Some(2));
    rig.assert_nothing_on_the_ui();
}

#[test]
fn a_restart_whose_stop_fails_gives_the_program_back_and_makes_nothing() {
    let rig = Rig::new();
    let mut controller = rig.ready(Plan {
        stop_error: Some("still running"),
        ..Plan::default()
    });
    controller.restart(rig.make(2)).unwrap();
    poll_until(&mut controller, Phase::Ready);
    assert_eq!(shown(&mut controller), Some(1));
    assert_eq!(controller.error(), Some("still running"));
    assert_eq!(rig.log(), ["make 1", "stop 1"]);

    controller.restart(rig.make(2)).unwrap();
    assert_eq!(controller.error(), None);
    poll_until(&mut controller, Phase::Ready);
    assert_eq!(shown(&mut controller), Some(2));
}

#[test]
fn a_restart_whose_next_program_fails_ends_closed_with_the_error_and_no_retry() {
    let rig = Rig::new();
    let mut controller = rig.ready(Plan::default());
    controller
        .restart(rig.failing(2, "no shell".to_string()))
        .unwrap();
    poll_until(&mut controller, Phase::Closed);
    assert_eq!(controller.error(), Some("no shell"));
    assert_eq!(shown(&mut controller), None);
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1", "make 2"]);
    rig.workers_ended();
    assert_eq!(controller.restart(rig.make(3)), Err(Refused::NotOpen));
    assert!(!rig.saw("make 3"));
}

// Refusals.

#[test]
fn requests_during_another_are_refused_and_start_nothing() {
    let rig = Rig::new();
    let mut controller = rig.controller();
    assert_eq!(controller.restart(rig.make(9)), Err(Refused::NotOpen));

    let open_door = gate();
    let plan = Plan {
        make_gate: Some(open_door.clone()),
        ..Plan::default()
    };
    controller.open(rig.make_as(1, plan)).unwrap();
    assert_eq!(controller.open(rig.make(9)), Err(Refused::Busy));
    assert_eq!(controller.restart(rig.make(9)), Err(Refused::Busy));
    open_door.release();
    poll_until(&mut controller, Phase::Ready);
    assert_eq!(controller.open(rig.make(9)), Err(Refused::AlreadyOpen));

    let restart_door = gate();
    let stop_door = gate();
    let plan = Plan {
        make_gate: Some(restart_door.clone()),
        stop_gate: Some(stop_door.clone()),
        ..Plan::default()
    };
    controller.restart(rig.make_as(2, plan)).unwrap();
    assert_eq!(controller.open(rig.make(9)), Err(Refused::Busy));
    assert_eq!(controller.restart(rig.make(9)), Err(Refused::Busy));
    restart_door.release();
    poll_until(&mut controller, Phase::Ready);

    controller.close().unwrap();
    assert_eq!(controller.open(rig.make(9)), Err(Refused::Busy));
    assert_eq!(controller.restart(rig.make(9)), Err(Refused::Busy));
    controller.close().unwrap();
    assert_eq!(controller.phase(), Phase::Closing);
    stop_door.release();
    poll_until(&mut controller, Phase::Closed);
    assert!(!rig.saw("make 9"));
    rig.workers_ended();
}

#[test]
fn a_refused_request_keeps_the_error() {
    let rig = Rig::new();
    let mut controller = rig.ready(Plan {
        stop_error: Some("still running"),
        ..Plan::default()
    });
    controller.close().unwrap();
    poll_until(&mut controller, Phase::Ready);
    assert_eq!(controller.open(rig.make(9)), Err(Refused::AlreadyOpen));
    assert_eq!(controller.error(), Some("still running"));
}

// Closing.

#[test]
fn close_takes_the_program_out_of_the_ui_and_ends_it_on_the_worker() {
    let rig = Rig::new();
    let stop_door = gate();
    let drop_door = gate();
    let mut controller = rig.ready(Plan {
        stop_gate: Some(stop_door.clone()),
        drop_gate: Some(drop_door.clone()),
        ..Plan::default()
    });
    let wakes = rig.wakes();

    controller.close().unwrap();
    assert_eq!(controller.phase(), Phase::Closing);
    assert_eq!(shown(&mut controller), None);
    stop_door.wait_reached();
    stop_door.release();
    drop_door.wait_reached();
    controller.poll();
    assert_eq!(
        controller.phase(),
        Phase::Closing,
        "not closed until it is dropped"
    );

    drop_door.release();
    poll_until(&mut controller, Phase::Closed);
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1"]);
    rig.workers_ended();
    assert_eq!(rig.wakes(), wakes + 1);
    rig.assert_nothing_on_the_ui();
}

#[test]
fn a_close_whose_stop_fails_gives_the_program_back() {
    let rig = Rig::new();
    let mut controller = rig.ready(Plan {
        stop_error: Some("still running"),
        ..Plan::default()
    });
    controller.close().unwrap();
    poll_until(&mut controller, Phase::Ready);
    assert_eq!(shown(&mut controller), Some(1));
    assert_eq!(controller.error(), Some("still running"));
    assert_eq!(rig.log(), ["make 1", "stop 1"]);

    controller.close().unwrap();
    poll_until(&mut controller, Phase::Closed);
    assert_eq!(controller.error(), None);
    assert_eq!(rig.log(), ["make 1", "stop 1", "stop 1", "drop 1"]);
}

#[test]
fn close_during_an_opening_leaves_no_program_behind() {
    let rig = Rig::new();
    let mut controller = rig.controller();
    let door = gate();
    let plan = Plan {
        make_gate: Some(door.clone()),
        ..Plan::default()
    };
    controller.open(rig.make_as(1, plan)).unwrap();
    door.wait_reached();

    controller.close().unwrap();
    assert_eq!(controller.phase(), Phase::Closing);
    assert_eq!(rig.workers(), 1, "joined the worker, started no other");

    door.release();
    poll_until(&mut controller, Phase::Closed);
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1"]);
    assert_eq!(shown(&mut controller), None);
    assert_eq!(controller.error(), None);
    rig.assert_nothing_on_the_ui();
    rig.workers_ended();

    controller.open(rig.make(2)).unwrap();
    poll_until(&mut controller, Phase::Ready);
    assert_eq!(
        shown(&mut controller),
        Some(2),
        "a later open is not cancelled"
    );
}

#[test]
fn close_during_an_opening_that_then_fails_is_just_closed() {
    let rig = Rig::new();
    let mut controller = rig.controller();
    let door = gate();
    let waiting = door.clone();
    let probe = rig.probe.clone();
    let make: Make = Box::new(move || -> Result<Fake, String> {
        probe.note("make 1".to_string());
        waiting.wait();
        Err("no shell".to_string())
    });
    controller.open(make).unwrap();
    door.wait_reached();
    controller.close().unwrap();
    door.release();
    poll_until(&mut controller, Phase::Closed);
    assert_eq!(controller.error(), None);
}

#[test]
fn close_during_an_opening_whose_program_will_not_stop_gives_it_back() {
    let rig = Rig::new();
    let mut controller = rig.controller();
    let door = gate();
    let plan = Plan {
        make_gate: Some(door.clone()),
        stop_error: Some("still running"),
        ..Plan::default()
    };
    controller.open(rig.make_as(1, plan)).unwrap();
    door.wait_reached();
    controller.close().unwrap();
    door.release();
    poll_until(&mut controller, Phase::Ready);
    assert_eq!(shown(&mut controller), Some(1));
    assert_eq!(controller.error(), Some("still running"));
    assert!(!rig.saw("drop 1"));
}

#[test]
fn close_while_a_restart_stops_never_makes_the_next_program() {
    let rig = Rig::new();
    let door = gate();
    let mut controller = rig.ready(Plan {
        stop_gate: Some(door.clone()),
        ..Plan::default()
    });
    controller.restart(rig.make(2)).unwrap();
    door.wait_reached();
    controller.close().unwrap();
    assert_eq!(controller.phase(), Phase::Closing);
    door.release();
    poll_until(&mut controller, Phase::Closed);
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1"]);
    rig.workers_ended();
}

#[test]
fn close_while_a_restart_cannot_stop_gives_the_program_back() {
    let rig = Rig::new();
    let door = gate();
    let mut controller = rig.ready(Plan {
        stop_gate: Some(door.clone()),
        stop_error: Some("still running"),
        ..Plan::default()
    });
    controller.restart(rig.make(2)).unwrap();
    door.wait_reached();
    controller.close().unwrap();
    door.release();
    poll_until(&mut controller, Phase::Ready);
    assert_eq!(shown(&mut controller), Some(1));
    assert_eq!(controller.error(), Some("still running"));
    assert!(!rig.saw("make 2"));
}

#[test]
fn close_while_a_restart_makes_ends_the_new_program_too() {
    let rig = Rig::new();
    let mut controller = rig.ready(Plan::default());
    let door = gate();
    let plan = Plan {
        make_gate: Some(door.clone()),
        ..Plan::default()
    };
    controller.restart(rig.make_as(2, plan)).unwrap();
    door.wait_reached();
    controller.close().unwrap();
    door.release();
    poll_until(&mut controller, Phase::Closed);
    assert_eq!(
        rig.log(),
        ["make 1", "stop 1", "drop 1", "make 2", "stop 2", "drop 2"]
    );
    assert_eq!(shown(&mut controller), None);
    rig.assert_nothing_on_the_ui();
}

#[test]
fn close_with_an_unread_program_ends_that_program() {
    let rig = Rig::new();
    let mut controller = rig.controller();
    controller.open(rig.make(1)).unwrap();
    until("woken", || rig.wakes() == 1);
    assert_eq!(controller.phase(), Phase::Opening);

    controller.close().unwrap();
    assert_eq!(controller.phase(), Phase::Closing);
    assert_eq!(shown(&mut controller), None);
    poll_until(&mut controller, Phase::Closed);
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1"]);
}

#[test]
fn close_with_nothing_open_clears_the_error_and_starts_nothing() {
    let rig = Rig::new();
    let mut controller = rig.controller();
    controller
        .open(rig.failing(1, "no shell".to_string()))
        .unwrap();
    poll_until(&mut controller, Phase::Closed);
    rig.workers_ended();

    controller.close().unwrap();
    assert_eq!(controller.phase(), Phase::Closed);
    assert_eq!(controller.error(), None);
    assert_eq!(rig.workers(), 0);
}

// Dropping the controller. Each gate below is still closed when the drop runs: a drop that
// waited for the program would fail on the gate's patience, on the UI thread.

#[test]
fn dropping_a_ready_controller_ends_the_program_off_the_ui() {
    let rig = Rig::new();
    let stop_door = gate();
    let drop_door = gate();
    let controller = rig.ready(Plan {
        stop_gate: Some(stop_door.clone()),
        drop_gate: Some(drop_door.clone()),
        ..Plan::default()
    });
    let wakes = rig.wakes();

    drop(controller);
    stop_door.wait_reached();
    stop_door.release();
    drop_door.wait_reached();
    drop_door.release();
    rig.workers_ended();
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1"]);
    assert_eq!(rig.wakes(), wakes, "nobody left to wake");
    rig.assert_nothing_on_the_ui();
}

#[test]
fn dropping_a_controller_whose_program_will_not_stop_still_drops_it_off_the_ui() {
    let rig = Rig::new();
    let controller = rig.ready(Plan {
        stop_error: Some("still running"),
        ..Plan::default()
    });
    drop(controller);
    rig.workers_ended();
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1"]);
    rig.assert_nothing_on_the_ui();
}

#[test]
fn dropping_a_controller_during_an_opening_leaves_no_program_behind() {
    let rig = Rig::new();
    let mut controller = rig.controller();
    let door = gate();
    let plan = Plan {
        make_gate: Some(door.clone()),
        ..Plan::default()
    };
    controller.open(rig.make_as(1, plan)).unwrap();
    door.wait_reached();

    drop(controller);
    door.release();
    rig.workers_ended();
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1"]);
    assert_eq!(rig.wakes(), 0);
    rig.assert_nothing_on_the_ui();
}

#[test]
fn dropping_a_controller_with_an_unread_program_ends_it_off_the_ui() {
    let rig = Rig::new();
    let mut controller = rig.controller();
    let door = gate();
    let plan = Plan {
        stop_gate: Some(door.clone()),
        ..Plan::default()
    };
    controller.open(rig.make_as(1, plan)).unwrap();
    until("woken", || rig.wakes() == 1);

    drop(controller);
    door.wait_reached();
    door.release();
    rig.workers_ended();
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1"]);
    assert_eq!(rig.wakes(), 1);
    rig.assert_nothing_on_the_ui();
}

#[test]
fn dropping_a_controller_during_a_restart_makes_nothing_and_tells_nobody() {
    let rig = Rig::new();
    let door = gate();
    let mut controller = rig.ready(Plan {
        stop_gate: Some(door.clone()),
        ..Plan::default()
    });
    let wakes = rig.wakes();
    controller.restart(rig.make(2)).unwrap();
    door.wait_reached();

    drop(controller);
    door.release();
    rig.workers_ended();
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1"]);
    assert_eq!(rig.wakes(), wakes);
    rig.assert_nothing_on_the_ui();
}

#[test]
fn dropping_a_controller_during_a_failing_close_drops_the_program_on_the_worker() {
    let rig = Rig::new();
    let door = gate();
    let mut controller = rig.ready(Plan {
        stop_gate: Some(door.clone()),
        stop_error: Some("still running"),
        ..Plan::default()
    });
    let wakes = rig.wakes();
    controller.close().unwrap();
    door.wait_reached();

    drop(controller);
    door.release();
    rig.workers_ended();
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1"]);
    assert_eq!(rig.wakes(), wakes);
    rig.assert_nothing_on_the_ui();
}

#[test]
fn dropping_a_controller_without_a_thread_never_runs_the_blocking_drop_on_the_ui() {
    let rig = Rig::new();
    let refuse = Arc::new(AtomicBool::new(false));
    let mut controller = rig.controller_refusing(&refuse);
    controller.open(rig.make(1)).unwrap();
    poll_until(&mut controller, Phase::Ready);
    rig.workers_ended();

    refuse.store(true, Ordering::SeqCst);
    drop(controller);
    assert_eq!(rig.log(), ["make 1"]);
    assert_eq!(rig.workers(), 0, "the cleanup's permit comes back");
}

// A thread that cannot start.

#[test]
fn an_open_without_a_thread_changes_nothing() {
    let rig = Rig::new();
    let refuse = Arc::new(AtomicBool::new(true));
    let mut controller = rig.controller_refusing(&refuse);
    assert_eq!(controller.open(rig.make(1)), Err(Refused::NoThread));
    assert_eq!(controller.phase(), Phase::Closed);
    assert_eq!(rig.workers(), 0);
    controller.poll();
    assert_eq!(controller.error(), None, "no worker to report on");
    assert_eq!(rig.wakes(), 0);
    assert!(rig.log().is_empty());

    refuse.store(false, Ordering::SeqCst);
    controller.open(rig.make(1)).unwrap();
    poll_until(&mut controller, Phase::Ready);
    assert_eq!(shown(&mut controller), Some(1));
}

#[test]
fn a_restart_or_close_without_a_thread_keeps_the_program_intact() {
    let rig = Rig::new();
    let refuse = Arc::new(AtomicBool::new(false));
    let mut controller = rig.controller_refusing(&refuse);
    controller.open(rig.make(1)).unwrap();
    poll_until(&mut controller, Phase::Ready);
    rig.workers_ended();
    let wakes = rig.wakes();

    refuse.store(true, Ordering::SeqCst);
    assert_eq!(controller.restart(rig.make(2)), Err(Refused::NoThread));
    assert_eq!(controller.close(), Err(Refused::NoThread));
    assert_eq!(controller.phase(), Phase::Ready);
    assert_eq!(shown(&mut controller), Some(1));
    assert_eq!(rig.workers(), 0);
    controller.poll();
    assert_eq!(controller.phase(), Phase::Ready, "no worker to report on");
    assert_eq!(controller.error(), None);
    assert_eq!(rig.wakes(), wakes);
    assert_eq!(rig.log(), ["make 1"]);

    refuse.store(false, Ordering::SeqCst);
    controller.close().unwrap();
    poll_until(&mut controller, Phase::Closed);
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1"]);
    rig.assert_nothing_on_the_ui();
}

// The bound.

#[test]
fn without_a_free_worker_nothing_changes() {
    let rig = Rig::new();
    let mut controller = rig.controller();
    rig.pool.store(MAX_WORKERS, Ordering::SeqCst);
    assert_eq!(controller.open(rig.make(1)), Err(Refused::Capacity));
    assert_eq!(controller.phase(), Phase::Closed);
    rig.pool.store(0, Ordering::SeqCst);

    controller.open(rig.make(1)).unwrap();
    poll_until(&mut controller, Phase::Ready);
    rig.workers_ended();
    rig.pool.store(MAX_WORKERS, Ordering::SeqCst);
    assert_eq!(controller.restart(rig.make(2)), Err(Refused::Capacity));
    assert_eq!(controller.close(), Err(Refused::Capacity));
    assert_eq!(controller.phase(), Phase::Ready);
    assert_eq!(shown(&mut controller), Some(1));
    assert_eq!(rig.pool.load(Ordering::SeqCst), MAX_WORKERS);
    assert_eq!(rig.log(), ["make 1"]);
    rig.pool.store(0, Ordering::SeqCst);
}

#[test]
fn workers_are_bounded_across_controllers_until_their_threads_end() {
    let rig = Rig::new();
    let doors: Vec<_> = (0..MAX_WORKERS).map(|_| gate()).collect();
    let mut controllers = Vec::new();
    for (id, door) in doors.iter().enumerate() {
        let mut controller = rig.controller();
        let plan = Plan {
            make_gate: Some(door.clone()),
            ..Plan::default()
        };
        controller.open(rig.make_as(id, plan)).unwrap();
        controllers.push(controller);
    }
    assert_eq!(rig.workers(), MAX_WORKERS);

    let mut extra = rig.controller();
    assert_eq!(extra.open(rig.make(99)), Err(Refused::Capacity));
    doors[0].release();
    until("a permit freed", || rig.workers() == MAX_WORKERS - 1);
    extra.open(rig.make(99)).unwrap();
    for door in &doors {
        door.release();
    }
    poll_until(&mut extra, Phase::Ready);
    assert_eq!(shown(&mut extra), Some(99));
}

// Panics and long errors.

#[test]
fn a_panic_while_making_ends_closed_with_an_error() {
    let rig = Rig::new();
    let mut controller = rig.controller();
    let make: Make = Box::new(|| -> Result<Fake, String> { panic!("synthetic panic in make") });
    controller.open(make).unwrap();
    poll_until(&mut controller, Phase::Closed);
    assert!(controller.error().is_some());
    rig.workers_ended();
    assert_eq!(rig.wakes(), 1);

    controller.open(rig.make(1)).unwrap();
    poll_until(&mut controller, Phase::Ready);
    assert_eq!(shown(&mut controller), Some(1));
}

#[test]
fn a_panic_while_stopping_ends_closed_and_drops_the_program_on_the_worker() {
    let rig = Rig::new();
    let mut controller = rig.ready(Plan {
        stop_panics: true,
        ..Plan::default()
    });
    controller.close().unwrap();
    poll_until(&mut controller, Phase::Closed);
    assert!(controller.error().is_some());
    assert_eq!(rig.log(), ["make 1", "stop 1", "drop 1"]);
    rig.assert_nothing_on_the_ui();
    rig.workers_ended();
}

#[test]
fn a_long_error_is_cut_on_a_character() {
    let rig = Rig::new();
    let mut controller = rig.controller();
    // With this first byte, the bound falls inside a two-byte character.
    let error = format!("x{}", "é".repeat(MAX_ERROR_BYTES));
    controller.open(rig.failing(1, error)).unwrap();
    poll_until(&mut controller, Phase::Closed);
    let kept = controller.error().unwrap();
    assert_eq!(kept.len(), MAX_ERROR_BYTES - 1);
    assert!(kept.ends_with('é'));
}
