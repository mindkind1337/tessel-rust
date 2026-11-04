//! Blocking terminal operations belong to this worker, never to an egui frame.
//!
//! A takeover's lease and a confirmed close go to the application's lease keeper: a place is
//! held there before the request is sent (none, nothing is sent), and the answer fills it as
//! soon as the request returns, even if the pane was closed meanwhile.
use super::host_commands::{self, Command, Refused};
use super::host_lease_keeper::{Ended, Keeper};
use super::host_opening::Slot;
use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicUsize, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
};
use std::time::Duration;
use tessel_host_client::{Client, ConnectionState, terminal::Terminal};
use tessel_host_protocol::{Epoch, LeaseKey, pty::Size};

const ONLINE: u8 = 1;
const WRITER: u8 = 2;
const MAX_WORKERS: usize = 16;
static WORKERS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    ControlTaken,
    Terminated,
    Refused,
    /// The lease keeper had no place for what this takeover or close would answer (too many
    /// saves waiting, another takeover of this terminal still in flight, or shutting down):
    /// nothing was sent. Try again.
    Busy,
    /// Some or all input may have reached the program. Never resubmit it.
    InputUncertain,
    Failed,
}

pub struct Control {
    commands: host_commands::Sender,
    state: Arc<AtomicU8>,
    outcomes: Receiver<Outcome>,
    connection: Option<(Client, Epoch)>,
}
impl Control {
    pub fn online(&self) -> bool {
        self.state.load(Ordering::Acquire) & ONLINE != 0
            && self
                .connection
                .as_ref()
                .is_none_or(|(client, epoch)| connected(client, epoch))
    }
    pub fn writable(&self) -> bool {
        self.online() && self.state.load(Ordering::Acquire) & WRITER != 0
    }
    pub fn input(&self, bytes: &[u8]) -> Result<(), Refused> {
        if !self.writable() {
            return Err(Refused::Closed);
        }
        self.commands.input(bytes)
    }
    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), Refused> {
        if !self.writable() {
            return Err(Refused::Closed);
        }
        self.commands.resize(rows, cols)
    }
    pub fn take_control(&self) -> Result<(), Refused> {
        if !self.online() {
            return Err(Refused::Closed);
        }
        self.commands.take_control()
    }
    pub fn terminate(&self) -> Result<(), Refused> {
        if !self.writable() {
            return Err(Refused::Closed);
        }
        self.commands.terminate()
    }
    /// Drain on every UI frame; outcomes are bounded and preserve error ordering.
    pub fn outcome(&self) -> Option<Outcome> {
        self.outcomes.try_recv().ok()
    }
}

/// Reserve before any remote create/attach RPC. Moving this permit into the
/// command worker keeps opening and running terminals under the same limit.
/// Dropping a cancelled or failed opening releases capacity automatically.
#[must_use]
pub struct Permit {
    count: &'static AtomicUsize,
}
impl Permit {
    pub fn acquire() -> Option<Self> {
        Self::reserve(&WORKERS)
    }
    fn reserve(count: &'static AtomicUsize) -> Option<Self> {
        count
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_WORKERS).then_some(count + 1)
            })
            .ok()
            .map(|_| Self { count })
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::AcqRel);
    }
}

/// One method per command, so a lease can only come from a takeover.
trait Driver: Send + 'static {
    fn authority(&self) -> u8;
    fn input(&mut self, bytes: &[u8]) -> Result<(), ()>;
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), ()>;
    fn take_control(&mut self) -> Result<LeaseKey, ()>;
    fn terminate(&mut self) -> Result<(), ()>;
}

struct HostDriver {
    client: Client,
    terminal: Terminal,
    epoch: Epoch,
}
fn connected(client: &Client, expected: &Epoch) -> bool {
    match client.state() {
        ConnectionState::Connected { epoch } => &epoch == expected,
        ConnectionState::Restarted { current, .. } => &current == expected,
        _ => false,
    }
}
impl Driver for HostDriver {
    fn authority(&self) -> u8 {
        let online = connected(&self.client, &self.epoch);
        if !online {
            0
        } else if self.terminal.lease().is_some() {
            ONLINE | WRITER
        } else {
            ONLINE
        }
    }
    fn input(&mut self, bytes: &[u8]) -> Result<(), ()> {
        self.terminal.input(bytes).map_err(|_| ())
    }
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), ()> {
        self.terminal.resize(Size { rows, cols }).map_err(|_| ())
    }
    fn take_control(&mut self) -> Result<LeaseKey, ()> {
        self.terminal.take_lease().map_err(|_| ())
    }
    fn terminate(&mut self) -> Result<(), ()> {
        self.terminal.close().map_err(|_| ())
    }
}

/// The terminal's place in the application's lease keeper (the one keeper of the app).
struct Saving {
    keeper: Arc<Keeper>,
    slot: Slot,
}

/// Call on the opening worker after creating or attaching a terminal. Its event
/// receiver remains with the UI. Dropping Control detaches, never terminates.
/// `slot` names this terminal (its alias, epoch and id); the epoch is the one expected live.
pub fn start(
    permit: Permit,
    client: Client,
    terminal: Terminal,
    slot: Slot,
    keeper: Arc<Keeper>,
) -> Result<Control, &'static str> {
    if terminal.id() != &slot.terminal {
        return Err("Remote terminal and its saved place differ");
    }
    let epoch = slot.epoch.clone();
    let connection = Some((client.clone(), epoch.clone()));
    spawn(
        permit,
        HostDriver {
            client,
            terminal,
            epoch,
        },
        connection,
        Saving { keeper, slot },
    )
}

/// Runs one command. `Ok(None)`: done, nothing to tell. `Err`: what to tell, after which the
/// queued commands are dropped.
fn execute(
    driver: &mut impl Driver,
    saving: &Saving,
    command: &Command,
) -> Result<Option<Outcome>, Outcome> {
    match command {
        Command::Input(bytes) => driver
            .input(bytes)
            .map(|()| None)
            .map_err(|()| Outcome::InputUncertain),
        Command::Resize { rows, cols } => driver
            .resize(*rows, *cols)
            .map(|()| None)
            .map_err(|()| Outcome::Failed),
        Command::TakeControl => {
            let place = saving
                .keeper
                .reserve(saving.slot.clone())
                .map_err(|_| Outcome::Busy)?;
            // A failure drops the place: nothing to save.
            let lease = driver.take_control().map_err(|()| Outcome::Failed)?;
            place.keep(lease);
            Ok(Some(Outcome::ControlTaken))
        }
        Command::Terminate => {
            let place = saving
                .keeper
                .reserve(saving.slot.clone())
                .map_err(|_| Outcome::Busy)?;
            driver.terminate().map_err(|()| Outcome::Failed)?;
            // Only an `EpochChanged` proof can be refused; a confirmed close never is.
            let forgotten = place.forget(Ended::CloseConfirmed);
            debug_assert!(forgotten.is_ok());
            Ok(Some(Outcome::Terminated))
        }
    }
}

fn spawn(
    permit: Permit,
    mut driver: impl Driver,
    connection: Option<(Client, Epoch)>,
    saving: Saving,
) -> Result<Control, &'static str> {
    let state = Arc::new(AtomicU8::new(driver.authority()));
    let worker_state = state.clone();
    let (commands, worker) = host_commands::channel();
    let (sender, outcomes) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("host-terminal".into())
        .spawn(move || {
            let _permit = permit;
            let mut previous = driver.authority();
            while !worker.closed() {
                let authority = driver.authority();
                if authority != previous {
                    worker.invalidate();
                    previous = authority;
                }
                worker_state.store(authority, Ordering::Release);
                let command = match worker.wait(Duration::from_millis(20)) {
                    Ok(command) => command,
                    Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => continue,
                };
                // Recheck immediately before the blocking call, not just when the UI enqueued it.
                let authority = driver.authority();
                let required = if matches!(command, Command::TakeControl) {
                    ONLINE
                } else {
                    ONLINE | WRITER
                };
                if worker.closed() {
                    break;
                }
                let outcome = if authority & required != required {
                    worker.invalidate();
                    Some(Outcome::Refused)
                } else {
                    match execute(&mut driver, &saving, &command) {
                        Ok(outcome) => outcome,
                        Err(outcome) => {
                            worker.invalidate();
                            Some(outcome)
                        }
                    }
                };
                if let Some(outcome) = outcome
                    && !send_outcome(&sender, &worker, outcome)
                {
                    break;
                }
                if outcome == Some(Outcome::Terminated) {
                    // close() acknowledges the request before the final output/Exit.
                    // Keep the stream alive briefly, without accepting more commands.
                    worker_state.store(0, Ordering::Release);
                    worker.invalidate();
                    let until = std::time::Instant::now() + Duration::from_secs(5);
                    while !worker.closed() && std::time::Instant::now() < until {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    break;
                }
            }
            worker_state.store(0, Ordering::Release);
            // Host Terminal::drop cancels its follower and detaches on another thread.
            // Any RPC already in flight may finish; no queued command is started after close.
        })
        .map_err(|_| "Cannot start remote terminal worker")?;
    Ok(Control {
        commands,
        state,
        outcomes,
        connection,
    })
}

fn send_outcome(
    sender: &SyncSender<Outcome>,
    worker: &host_commands::Worker,
    outcome: Outcome,
) -> bool {
    while !worker.closed() {
        match sender.try_send(outcome) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(_)) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::super::host_lease_keeper::{LeaseWriter, MAX_QUEUE};
    use super::super::host_opening::LocalError;
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;
    use tessel_host_protocol::TerminalId;

    struct Fake {
        authority: Arc<AtomicU8>,
        calls: mpsc::Sender<&'static str>,
        dropped: mpsc::Sender<()>,
        block_input: Option<Receiver<()>>,
        block_control: Option<Receiver<()>>,
        fail_input: Arc<AtomicBool>,
        fail_control: bool,
        fail_terminate: bool,
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = self.dropped.send(());
        }
    }
    impl Driver for Fake {
        fn authority(&self) -> u8 {
            self.authority.load(Ordering::Acquire)
        }
        fn input(&mut self, _bytes: &[u8]) -> Result<(), ()> {
            self.calls.send("input").unwrap();
            if let Some(block) = self.block_input.take() {
                block.recv_timeout(Duration::from_secs(3)).unwrap();
            }
            if self.fail_input.swap(false, Ordering::AcqRel) {
                return Err(());
            }
            Ok(())
        }
        fn resize(&mut self, _rows: u16, _cols: u16) -> Result<(), ()> {
            self.calls.send("resize").unwrap();
            Ok(())
        }
        fn take_control(&mut self) -> Result<LeaseKey, ()> {
            self.calls.send("control").unwrap();
            if let Some(block) = self.block_control.take() {
                block.recv_timeout(Duration::from_secs(3)).unwrap();
            }
            if self.fail_control {
                return Err(());
            }
            self.authority.store(ONLINE | WRITER, Ordering::Release);
            Ok(LeaseKey::from_bytes([7; 16]))
        }
        fn terminate(&mut self) -> Result<(), ()> {
            self.calls.send("terminate").unwrap();
            if self.fail_terminate {
                return Err(());
            }
            Ok(())
        }
    }
    fn fake(authority: u8) -> (Fake, Receiver<&'static str>, Receiver<()>) {
        let (calls, received) = mpsc::channel();
        let (dropped, ended) = mpsc::channel();
        (
            Fake {
                authority: Arc::new(AtomicU8::new(authority)),
                calls,
                dropped,
                block_input: None,
                block_control: None,
                fail_input: Arc::new(AtomicBool::new(false)),
                fail_control: false,
                fail_terminate: false,
            },
            received,
            ended,
        )
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Op {
        Put(LeaseKey),
        Remove,
    }
    #[derive(Default)]
    struct Written(Mutex<Vec<Op>>);
    impl Written {
        fn ops(&self) -> Vec<Op> {
            self.0.lock().unwrap().clone()
        }
    }
    impl LeaseWriter for Written {
        fn put(&self, _slot: &Slot, lease: &LeaseKey) -> Result<(), LocalError> {
            self.0.lock().unwrap().push(Op::Put(lease.clone()));
            Ok(())
        }
        fn remove(&self, _slot: &Slot) -> Result<(), LocalError> {
            self.0.lock().unwrap().push(Op::Remove);
            Ok(())
        }
    }
    fn slot(n: u8) -> Slot {
        Slot {
            alias: "fixture".into(),
            epoch: Epoch::from_bytes([1; 16]),
            terminal: TerminalId::from_bytes([n; 16]),
        }
    }
    fn saving() -> (Arc<Written>, Arc<Keeper>, Saving) {
        let written = Arc::new(Written::default());
        let keeper = Arc::new(Keeper::start(written.clone()).unwrap());
        let saving = Saving {
            keeper: keeper.clone(),
            slot: slot(1),
        };
        (written, keeper, saving)
    }
    fn wait(mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !ready() {
            assert!(Instant::now() < deadline, "worker timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn openings_reserve_capacity_before_network_and_release_on_cancellation() {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let mut pending: Vec<_> = (0..MAX_WORKERS)
            .map(|_| Permit::reserve(&COUNT).unwrap())
            .collect();
        assert!(Permit::reserve(&COUNT).is_none());
        drop(pending.pop());
        let replacement = Permit::reserve(&COUNT).unwrap();
        assert!(Permit::reserve(&COUNT).is_none());
        drop(pending);
        drop(replacement);
        assert_eq!(COUNT.load(Ordering::Acquire), 0);
    }

    #[test]
    fn commands_execute_in_order_and_termination_stops_the_worker() {
        let (driver, calls, ended) = fake(ONLINE | WRITER);
        let (written, _keeper, saving) = saving();
        let control = spawn(Permit::acquire().unwrap(), driver, None, saving).unwrap();
        control.input(b"fixture").unwrap();
        control.resize(30, 80).unwrap();
        control.terminate().unwrap();
        for expected in ["input", "resize", "terminate"] {
            assert_eq!(
                calls.recv_timeout(Duration::from_secs(3)).unwrap(),
                expected
            );
        }
        wait(|| control.outcome() == Some(Outcome::Terminated));
        wait(|| !control.writable());
        // The confirmed close forgets the saved lease.
        wait(|| written.ops() == [Op::Remove]);
        drop(control);
        ended.recv_timeout(Duration::from_secs(3)).unwrap();
    }
    #[test]
    fn observer_can_request_control_but_cannot_type_resize_or_terminate() {
        let (driver, calls, ended) = fake(ONLINE);
        let (written, _keeper, saving) = saving();
        let control = spawn(Permit::acquire().unwrap(), driver, None, saving).unwrap();
        assert_eq!(control.input(b"fixture"), Err(Refused::Closed));
        assert_eq!(control.resize(30, 80), Err(Refused::Closed));
        assert_eq!(control.terminate(), Err(Refused::Closed));
        control.take_control().unwrap();
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(3)).unwrap(),
            "control"
        );
        wait(|| control.outcome() == Some(Outcome::ControlTaken));
        wait(|| control.writable());
        // The lease the takeover returned is saved.
        wait(|| written.ops() == [Op::Put(LeaseKey::from_bytes([7; 16]))]);
        drop(control);
        ended.recv_timeout(Duration::from_secs(3)).unwrap();
    }
    #[test]
    fn closing_during_a_blocked_rpc_discards_queued_termination() {
        let (mut driver, calls, ended) = fake(ONLINE | WRITER);
        let (release, block) = mpsc::channel();
        driver.block_input = Some(block);
        let (written, _keeper, saving) = saving();
        let control = spawn(Permit::acquire().unwrap(), driver, None, saving).unwrap();
        control.input(b"fixture").unwrap();
        assert_eq!(calls.recv_timeout(Duration::from_secs(3)).unwrap(), "input");
        control.terminate().unwrap();
        drop(control); // must not wait for the RPC
        release.send(()).unwrap();
        ended.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(calls.try_recv().is_err());
        assert!(written.ops().is_empty());
    }
    #[test]
    fn uncertain_input_is_not_retried_and_queued_input_is_discarded() {
        let (mut driver, calls, ended) = fake(ONLINE | WRITER);
        let (release, block) = mpsc::channel();
        driver.block_input = Some(block);
        driver.fail_input.store(true, Ordering::Release);
        let (_written, _keeper, saving) = saving();
        let control = spawn(Permit::acquire().unwrap(), driver, None, saving).unwrap();
        control.input(b"first").unwrap();
        assert_eq!(calls.recv_timeout(Duration::from_secs(3)).unwrap(), "input");
        control.input(b"queued").unwrap();
        release.send(()).unwrap();
        wait(|| control.outcome() == Some(Outcome::InputUncertain));
        assert!(calls.try_recv().is_err());
        control.input(b"new explicit input").unwrap();
        assert_eq!(calls.recv_timeout(Duration::from_secs(3)).unwrap(), "input");
        drop(control);
        ended.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(calls.try_recv().is_err());
    }
    #[test]
    fn without_a_place_in_the_keeper_nothing_is_sent() {
        let (driver, calls, ended) = fake(ONLINE | WRITER);
        let (written, keeper, saving) = saving();
        // Every place held elsewhere.
        let held: Vec<_> = (10..10 + MAX_QUEUE as u8)
            .map(|n| keeper.reserve(slot(n)).ok().unwrap())
            .collect();
        let control = spawn(Permit::acquire().unwrap(), driver, None, saving).unwrap();
        control.take_control().unwrap();
        wait(|| control.outcome() == Some(Outcome::Busy));
        control.terminate().unwrap();
        wait(|| control.outcome() == Some(Outcome::Busy));
        assert!(calls.try_recv().is_err(), "nothing sent without a place");
        drop(held);
        control.take_control().unwrap();
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(3)).unwrap(),
            "control"
        );
        wait(|| control.outcome() == Some(Outcome::ControlTaken));
        wait(|| written.ops() == [Op::Put(LeaseKey::from_bytes([7; 16]))]);
        drop(control);
        ended.recv_timeout(Duration::from_secs(3)).unwrap();
    }
    #[test]
    fn a_takeover_in_flight_holds_back_another_one_for_the_same_terminal() {
        let (driver, calls, ended) = fake(ONLINE);
        let (_written, keeper, saving) = saving();
        let other = keeper.reserve(slot(1)).ok().unwrap();
        let control = spawn(Permit::acquire().unwrap(), driver, None, saving).unwrap();
        control.take_control().unwrap();
        wait(|| control.outcome() == Some(Outcome::Busy));
        assert!(calls.try_recv().is_err());
        drop(other);
        drop(control);
        ended.recv_timeout(Duration::from_secs(3)).unwrap();
    }
    #[test]
    fn a_failed_takeover_or_close_saves_nothing_and_gives_the_place_back() {
        let (mut driver, calls, ended) = fake(ONLINE | WRITER);
        driver.fail_control = true;
        driver.fail_terminate = true;
        let (written, keeper, saving) = saving();
        let control = spawn(Permit::acquire().unwrap(), driver, None, saving).unwrap();
        control.take_control().unwrap();
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(3)).unwrap(),
            "control"
        );
        wait(|| control.outcome() == Some(Outcome::Failed));
        control.terminate().unwrap();
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(3)).unwrap(),
            "terminate"
        );
        wait(|| control.outcome() == Some(Outcome::Failed));
        // Both places were given back: the slot can be reserved again.
        assert!(keeper.reserve(slot(1)).is_ok());
        std::thread::sleep(Duration::from_millis(50));
        assert!(written.ops().is_empty());
        drop(control);
        ended.recv_timeout(Duration::from_secs(3)).unwrap();
    }
    #[test]
    fn a_lease_returned_after_the_pane_closed_is_still_saved() {
        let (mut driver, calls, ended) = fake(ONLINE);
        let (release, block) = mpsc::channel();
        driver.block_control = Some(block);
        let (written, _keeper, saving) = saving();
        let control = spawn(Permit::acquire().unwrap(), driver, None, saving).unwrap();
        control.take_control().unwrap();
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(3)).unwrap(),
            "control"
        );
        drop(control); // the pane closes while the takeover is in flight
        release.send(()).unwrap();
        ended.recv_timeout(Duration::from_secs(3)).unwrap();
        wait(|| written.ops() == [Op::Put(LeaseKey::from_bytes([7; 16]))]);
    }
}
