//! Bounded, ordered commands for the remote-terminal worker.
//! The mutex protects only this small queue, never any network operation.
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{RecvTimeoutError, TryRecvError},
    },
    time::Duration,
};
use zeroize::Zeroizing;

pub const CAPACITY: usize = 32;
// One protocol input, including any bracketed-paste markers. Never split a paste.
pub const MAX_INPUT_BYTES: usize = 64 * 1024;

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
#[derive(Default)]
struct Queue {
    commands: VecDeque<Command>,
    closed: bool,
}
#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
}
pub struct Sender {
    shared: Arc<Shared>,
}
pub struct Worker {
    shared: Arc<Shared>,
}
pub fn channel() -> (Sender, Worker) {
    let shared = Arc::new(Shared::default());
    (
        Sender {
            shared: shared.clone(),
        },
        Worker { shared },
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
        self.send(Command::Resize { rows, cols })
    }
    pub fn take_control(&self) -> Result<(), Refused> {
        self.send(Command::TakeControl)
    }
    pub fn terminate(&self) -> Result<(), Refused> {
        self.send(Command::Terminate)
    }
    fn send(&self, command: Command) -> Result<(), Refused> {
        let mut queue = self.shared.queue.lock().unwrap();
        if queue.closed {
            return Err(Refused::Closed);
        }
        // Replace only an immediately adjacent resize; never overtake input.
        if matches!(command, Command::Resize { .. })
            && let Some(last @ Command::Resize { .. }) = queue.commands.back_mut()
        {
            *last = command;
            return Ok(());
        }
        if queue.commands.len() == CAPACITY {
            return Err(Refused::Full);
        }
        queue.commands.push_back(command);
        self.shared.wake.notify_one();
        Ok(())
    }
}
impl Shared {
    fn close(&self) {
        let mut queue = self.queue.lock().unwrap();
        queue.closed = true;
        queue.commands.clear();
        self.wake.notify_one();
    }
}
impl Drop for Sender {
    fn drop(&mut self) {
        self.shared.close();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.close();
    }
}
impl Worker {
    /// Atomically discard unsent work on an observed connection/lease change.
    /// An RPC already in flight cannot be recalled or resubmitted with a new ID.
    pub fn invalidate(&self) {
        self.shared.queue.lock().unwrap().commands.clear();
    }
    pub fn closed(&self) -> bool {
        self.shared.queue.lock().unwrap().closed
    }
    pub fn next(&self) -> Result<Command, TryRecvError> {
        let mut queue = self.shared.queue.lock().unwrap();
        if queue.closed {
            return Err(TryRecvError::Disconnected);
        }
        queue.commands.pop_front().ok_or(TryRecvError::Empty)
    }
    /// Wakes immediately on input or close; timeout only refreshes authority.
    pub fn wait(&self, timeout: Duration) -> Result<Command, RecvTimeoutError> {
        let queue = self.shared.queue.lock().unwrap();
        let (mut queue, _) = self
            .shared
            .wake
            .wait_timeout_while(queue, timeout, |queue| {
                !queue.closed && queue.commands.is_empty()
            })
            .unwrap();
        if queue.closed {
            return Err(RecvTimeoutError::Disconnected);
        }
        queue.commands.pop_front().ok_or(RecvTimeoutError::Timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_resizes_coalesce_without_crossing_input() {
        let (sender, worker) = channel();
        for cols in 20..100 {
            sender.resize(30, cols).unwrap();
        }
        sender.input(b"x").unwrap();
        for cols in 100..200 {
            sender.resize(40, cols).unwrap();
        }
        assert!(matches!(
            worker.next(),
            Ok(Command::Resize { rows: 30, cols: 99 })
        ));
        assert!(matches!(worker.next(), Ok(Command::Input(_))));
        assert!(matches!(
            worker.next(),
            Ok(Command::Resize {
                rows: 40,
                cols: 199
            })
        ));
        assert!(matches!(
            worker.wait(Duration::ZERO),
            Err(RecvTimeoutError::Timeout)
        ));
        drop(sender);
        assert!(matches!(
            worker.wait(Duration::from_secs(30)),
            Err(RecvTimeoutError::Disconnected)
        ));
    }

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
