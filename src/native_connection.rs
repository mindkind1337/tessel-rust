//! Cancellable native-process startup. Validation and spawn stay off the UI thread.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};
// Filesystem validation and cold spawn have a separate budget from the
// provider protocol initialization (30 seconds after the child is delivered).
pub struct Pending<T> {
    receiver: mpsc::Receiver<Result<T, String>>,
    cancelled: Arc<AtomicBool>,
    started: Instant,
}
impl<T> Drop for Pending<T> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}
impl<T: Send + 'static> Pending<T> {
    pub fn start(
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        std::thread::Builder::new()
            .name("native-chat-connect".into())
            .spawn(move || {
                if flag.load(Ordering::Relaxed) {
                    return;
                }
                let result = work();
                if !flag.load(Ordering::Relaxed) {
                    let _ = sender.send(result);
                }
                // Dropping an undelivered result terminates its owned child process.
            })
            .map_err(|_| "Cannot start native chat connection worker.".to_owned())?;
        Ok(Self {
            receiver,
            cancelled,
            started: Instant::now(),
        })
    }
    pub fn poll(&self) -> Option<Result<T, String>> {
        match self.receiver.try_recv() {
            Ok(value) => Some(value),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("Native chat startup stopped unexpectedly.".into()))
            }
            Err(mpsc::TryRecvError::Empty) if self.started.elapsed() > Duration::from_secs(120) => {
                Some(Err("Native chat startup timed out.".into()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_drops_late_process_owner_without_delivering_it() {
        struct Owner(mpsc::Sender<()>);
        impl Drop for Owner {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (dropped_tx, dropped_rx) = mpsc::channel();
        let pending = Pending::start(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(Owner(dropped_tx))
        })
        .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(pending);
        release_tx.send(()).unwrap();
        dropped_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}
