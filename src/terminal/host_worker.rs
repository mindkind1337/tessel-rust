//! Blocking terminal operations belong to this worker, never to an egui frame.
use super::host_commands::{self, Command, Refused};
use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicUsize, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
};
use std::time::Duration;
use tessel_host_client::{Client, ConnectionState, terminal::Terminal};
use tessel_host_protocol::{Epoch, pty::Size};

const ONLINE: u8 = 1;
const WRITER: u8 = 2;
const MAX_WORKERS: usize = 16;
static WORKERS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    ControlTaken,
    Terminated,
    Refused,
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

trait Driver: Send + 'static {
    fn authority(&self) -> u8;
    fn execute(&mut self, command: &Command) -> Result<(), ()>;
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
    fn execute(&mut self, command: &Command) -> Result<(), ()> {
        match command {
            Command::Input(bytes) => self.terminal.input(bytes),
            Command::Resize { rows, cols } => self.terminal.resize(Size {
                rows: *rows,
                cols: *cols,
            }),
            Command::TakeControl => self.terminal.take_lease().map(|_| ()),
            Command::Terminate => self.terminal.close(),
        }
        .map_err(|_| ())
    }
}

/// Call on the opening worker after creating or attaching a terminal. Its event
/// receiver remains with the UI. Dropping Control detaches, never terminates.
pub fn start(
    permit: Permit,
    client: Client,
    terminal: Terminal,
    epoch: Epoch,
) -> Result<Control, &'static str> {
    let connection = Some((client.clone(), epoch.clone()));
    spawn(
        permit,
        HostDriver {
            client,
            terminal,
            epoch,
        },
        connection,
    )
}

fn spawn(
    permit: Permit,
    mut driver: impl Driver,
    connection: Option<(Client, Epoch)>,
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
                    match driver.execute(&command) {
                        Ok(()) => match command {
                            Command::TakeControl => Some(Outcome::ControlTaken),
                            Command::Terminate => Some(Outcome::Terminated),
                            _ => None,
                        },
                        Err(()) => {
                            worker.invalidate();
                            Some(if matches!(command, Command::Input(_)) {
                                Outcome::InputUncertain
                            } else {
                                Outcome::Failed
                            })
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
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    struct Fake {
        authority: Arc<AtomicU8>,
        calls: mpsc::Sender<&'static str>,
        dropped: mpsc::Sender<()>,
        block_input: Option<Receiver<()>>,
        fail_input: Arc<AtomicBool>,
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
        fn execute(&mut self, command: &Command) -> Result<(), ()> {
            let name = match command {
                Command::Input(_) => "input",
                Command::Resize { .. } => "resize",
                Command::TakeControl => "control",
                Command::Terminate => "terminate",
            };
            self.calls.send(name).unwrap();
            if matches!(command, Command::TakeControl) {
                self.authority.store(ONLINE | WRITER, Ordering::Release);
            }
            if matches!(command, Command::Input(_)) {
                if let Some(block) = self.block_input.take() {
                    block.recv_timeout(Duration::from_secs(3)).unwrap();
                }
                if self.fail_input.swap(false, Ordering::AcqRel) {
                    return Err(());
                }
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
                fail_input: Arc::new(AtomicBool::new(false)),
            },
            received,
            ended,
        )
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
        let control = spawn(Permit::acquire().unwrap(), driver, None).unwrap();
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
        drop(control);
        ended.recv_timeout(Duration::from_secs(3)).unwrap();
    }
    #[test]
    fn observer_can_request_control_but_cannot_type_resize_or_terminate() {
        let (driver, calls, ended) = fake(ONLINE);
        let control = spawn(Permit::acquire().unwrap(), driver, None).unwrap();
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
        drop(control);
        ended.recv_timeout(Duration::from_secs(3)).unwrap();
    }
    #[test]
    fn closing_during_a_blocked_rpc_discards_queued_termination() {
        let (mut driver, calls, ended) = fake(ONLINE | WRITER);
        let (release, block) = mpsc::channel();
        driver.block_input = Some(block);
        let control = spawn(Permit::acquire().unwrap(), driver, None).unwrap();
        control.input(b"fixture").unwrap();
        assert_eq!(calls.recv_timeout(Duration::from_secs(3)).unwrap(), "input");
        control.terminate().unwrap();
        drop(control); // must not wait for the RPC
        release.send(()).unwrap();
        ended.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(calls.try_recv().is_err());
    }
    #[test]
    fn uncertain_input_is_not_retried_and_queued_input_is_discarded() {
        let (mut driver, calls, ended) = fake(ONLINE | WRITER);
        let (release, block) = mpsc::channel();
        driver.block_input = Some(block);
        driver.fail_input.store(true, Ordering::Release);
        let control = spawn(Permit::acquire().unwrap(), driver, None).unwrap();
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
}
