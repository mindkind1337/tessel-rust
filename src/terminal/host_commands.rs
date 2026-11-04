//! Bounded, ordered commands for the remote-terminal worker.
//! No network calls or terminal contents are logged here. The worker must
//! revalidate the connection and lease immediately before executing a command.
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
};
use zeroize::Zeroizing;

pub const CAPACITY: usize = 32;
// Match the renderer's 64 KiB paste limit plus both bracketed-paste markers.
pub const MAX_INPUT_BYTES: usize = 64 * 1024 + 12;

// Deliberately no Debug: terminal input can include passwords.
pub enum Command {
    Input(Zeroizing<Vec<u8>>),
    Resize { rows: u16, cols: u16 },
    TakeControl,
    Terminate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Closed,
    Full,
    TooLarge,
    InvalidSize,
}

struct Shared {
    generation: AtomicU64,
    closed: AtomicBool,
}
struct Envelope {
    generation: u64,
    command: Command,
}
pub struct Sender {
    shared: Arc<Shared>,
    sender: SyncSender<Envelope>,
}
pub struct Worker {
    shared: Arc<Shared>,
    receiver: Receiver<Envelope>,
}

pub fn channel() -> (Sender, Worker) {
    let shared = Arc::new(Shared {
        generation: AtomicU64::new(0),
        closed: AtomicBool::new(false),
    });
    let (sender, receiver) = mpsc::sync_channel(CAPACITY);
    (
        Sender {
            shared: shared.clone(),
            sender,
        },
        Worker { shared, receiver },
    )
}

impl Sender {
    /// Reject an oversize paste as a whole: never enqueue only a prefix.
    pub fn input(&self, bytes: &[u8]) -> Result<(), Refused> {
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(Refused::TooLarge);
        }
        self.send(Command::Input(Zeroizing::new(bytes.to_vec())))
    }
    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), Refused> {
        if !(1..=150).contains(&rows) || !(1..=300).contains(&cols) {
            return Err(Refused::InvalidSize);
        }
        // Do not coalesce across input: resize and typing must stay ordered.
        self.send(Command::Resize { rows, cols })
    }
    pub fn take_control(&self) -> Result<(), Refused> {
        self.send(Command::TakeControl)
    }
    pub fn terminate(&self) -> Result<(), Refused> {
        self.send(Command::Terminate)
    }
    fn send(&self, command: Command) -> Result<(), Refused> {
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(Refused::Closed);
        }
        let envelope = Envelope {
            generation: self.shared.generation.load(Ordering::Acquire),
            command,
        };
        self.sender.try_send(envelope).map_err(|error| match error {
            TrySendError::Full(_) => Refused::Full,
            TrySendError::Disconnected(_) => Refused::Closed,
        })
    }
}
impl Drop for Sender {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
    }
}
impl Worker {
    /// Connection/authority changes invalidate unsent commands. A call already
    /// in flight cannot be recalled: report uncertain delivery, never resend it.
    pub fn invalidate(&self) {
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
    }
    pub fn closed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire)
    }
    pub fn next(&self) -> Result<Command, TryRecvError> {
        loop {
            if self.closed() {
                return Err(TryRecvError::Disconnected);
            }
            let envelope = self.receiver.try_recv()?;
            if envelope.generation == self.shared.generation.load(Ordering::Acquire) {
                return Ok(envelope.command);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_never_overtakes_typed_input() {
        let (sender, worker) = channel();
        sender.resize(30, 100).unwrap();
        sender.input(b"x").unwrap();
        sender.resize(40, 120).unwrap();
        assert!(matches!(
            worker.next(),
            Ok(Command::Resize {
                rows: 30,
                cols: 100
            })
        ));
        match worker.next().unwrap() {
            Command::Input(bytes) => assert_eq!(&**bytes, b"x"),
            _ => panic!("input order"),
        }
        assert!(matches!(
            worker.next(),
            Ok(Command::Resize {
                rows: 40,
                cols: 120
            })
        ));
    }
    #[test]
    fn full_queue_and_large_paste_do_not_partially_type() {
        let (sender, worker) = channel();
        assert_eq!(
            sender.input(&vec![0; MAX_INPUT_BYTES + 1]),
            Err(Refused::TooLarge)
        );
        assert!(matches!(worker.next(), Err(TryRecvError::Empty)));
        for _ in 0..CAPACITY {
            sender.input(b"x").unwrap();
        }
        assert_eq!(sender.input(b"must not be typed"), Err(Refused::Full));
        for _ in 0..CAPACITY {
            assert!(matches!(worker.next(), Ok(Command::Input(_))));
        }
        assert!(matches!(worker.next(), Err(TryRecvError::Empty)));
    }
    #[test]
    fn connection_or_lease_loss_discards_old_commands() {
        let (sender, worker) = channel();
        sender.input(b"old").unwrap();
        sender.terminate().unwrap();
        worker.invalidate();
        sender.take_control().unwrap();
        assert!(matches!(worker.next(), Ok(Command::TakeControl)));
        assert!(matches!(worker.next(), Err(TryRecvError::Empty)));
    }
    #[test]
    fn closing_pane_never_executes_queued_terminate() {
        let (sender, worker) = channel();
        sender.terminate().unwrap();
        drop(sender);
        assert!(worker.closed());
        assert!(matches!(worker.next(), Err(TryRecvError::Disconnected)));
    }
    #[test]
    fn invalid_sizes_and_dead_worker_are_refused() {
        let (sender, worker) = channel();
        assert_eq!(sender.resize(0, 80), Err(Refused::InvalidSize));
        assert_eq!(sender.resize(30, 301), Err(Refused::InvalidSize));
        drop(worker);
        assert_eq!(sender.input(b"x"), Err(Refused::Closed));
    }
}
