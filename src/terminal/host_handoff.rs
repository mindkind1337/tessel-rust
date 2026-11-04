//! Ownership handoff for an opening worker. Only an explicit cancellation may
//! request termination of a newly created program; dropping the UI just detaches.
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unclaimed {
    Cancelled,
    Dropped,
    TimedOut,
}
#[derive(Clone, Copy)]
enum Phase {
    Waiting,
    Ready,
    Claimed,
    Stopped(Unclaimed),
}
struct Inner<T> {
    phase: Phase,
    value: Option<T>,
}
struct Shared<T> {
    state: Mutex<Inner<T>>,
    changed: Condvar,
}
pub struct Receiver<T>(Arc<Shared<T>>);
pub struct Sender<T>(Arc<Shared<T>>);

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared {
        state: Mutex::new(Inner {
            phase: Phase::Waiting,
            value: None,
        }),
        changed: Condvar::new(),
    });
    (Sender(shared.clone()), Receiver(shared))
}
impl<T> Receiver<T> {
    pub fn stopped(&self) -> Option<Unclaimed> {
        match self.0.state.lock().unwrap().phase {
            Phase::Stopped(reason) => Some(reason),
            _ => None,
        }
    }
    /// Leaves the pending value untouched for cleanup by its opening worker.
    pub fn cancel(&self) {
        self.stop(Unclaimed::Cancelled);
    }
    fn stop(&self, reason: Unclaimed) {
        let mut state = self.0.state.lock().unwrap();
        if matches!(state.phase, Phase::Waiting | Phase::Ready) {
            state.phase = Phase::Stopped(reason);
        }
        self.0.changed.notify_all();
    }
    /// Claims ownership atomically against cancellation and the worker timeout.
    pub fn take(&self) -> Option<T> {
        let mut state = self.0.state.lock().unwrap();
        if matches!(state.phase, Phase::Ready) {
            state.phase = Phase::Claimed;
            let value = state.value.take();
            self.0.changed.notify_all();
            value
        } else {
            None
        }
    }
}
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.stop(Unclaimed::Dropped);
    }
}
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap();
        if !matches!(state.phase, Phase::Claimed | Phase::Stopped(_)) {
            state.phase = Phase::Stopped(Unclaimed::Dropped);
        }
        let value = state.value.take();
        self.0.changed.notify_all();
        drop(state);
        // If ready notification unwinds, ownership still leaves on this worker,
        // never later when the interface drops its receiver.
        drop(value);
    }
}
impl<T> Sender<T> {
    /// Called only on the opening worker. A returned value must be detached, or
    /// explicitly cancelled if it is newly created and the reason is Cancelled.
    /// Ready notification must be nonblocking. No I/O runs under the mutex.
    pub fn offer(
        self,
        value: T,
        timeout: Duration,
        ready: impl FnOnce(),
    ) -> Result<(), (Unclaimed, T)> {
        let deadline = Instant::now() + timeout.min(Duration::from_secs(30));
        let mut state = self.0.state.lock().unwrap();
        if let Phase::Stopped(reason) = state.phase {
            return Err((reason, value));
        }
        state.value = Some(value);
        state.phase = Phase::Ready;
        drop(state);
        ready();
        let mut state = self.0.state.lock().unwrap();
        loop {
            match state.phase {
                Phase::Claimed => return Ok(()),
                Phase::Stopped(reason) => {
                    return Err((
                        reason,
                        state.value.take().expect("worker owns unclaimed value"),
                    ));
                }
                Phase::Waiting | Phase::Ready => {}
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                state.phase = Phase::Stopped(Unclaimed::TimedOut);
                continue;
            }
            state = self.0.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn only_explicit_cancellation_requests_cleanup_of_a_new_program() {
        for cancel in [false, true] {
            let (sender, receiver) = channel();
            let (notice, ready) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                sender.offer(42, Duration::from_secs(3), || {
                    notice.send(()).unwrap();
                })
            });
            ready.recv_timeout(Duration::from_secs(3)).unwrap();
            if cancel {
                receiver.cancel();
            }
            drop(receiver);
            let expected = if cancel {
                Unclaimed::Cancelled
            } else {
                Unclaimed::Dropped
            };
            assert_eq!(worker.join().unwrap(), Err((expected, 42)));
        }
    }
    #[test]
    fn claimed_value_survives_receiver_drop_and_timeout_does_not_cancel() {
        let (sender, receiver) = channel();
        let (notice, ready) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            sender.offer(42, Duration::from_secs(3), || {
                notice.send(()).unwrap();
            })
        });
        ready.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(receiver.take(), Some(42));
        receiver.cancel();
        drop(receiver);
        assert_eq!(worker.join().unwrap(), Ok(()));
        let (sender, receiver) = channel();
        assert_eq!(
            sender.offer(7, Duration::ZERO, || {}),
            Err((Unclaimed::TimedOut, 7))
        );
        assert_eq!(receiver.take(), None);
    }
    #[test]
    fn cancellation_before_ready_returns_ownership_to_worker() {
        let (sender, receiver) = channel();
        receiver.cancel();
        assert_eq!(
            sender.offer(9, Duration::from_secs(1), || panic!("already cancelled")),
            Err((Unclaimed::Cancelled, 9))
        );
    }
}
