//! One background discovery at a time; failed refreshes retain the last good snapshot.
use std::sync::mpsc::{self, Receiver, TryRecvError};

type Snapshot = Vec<(String, bool)>;
type Outcome = Result<Snapshot, String>;

#[derive(Default)]
pub(crate) struct Detection {
    entries: Snapshot,
    loaded: bool,
    error: Option<String>,
    pending: Option<Receiver<Outcome>>,
}

impl Detection {
    pub fn entries(&self) -> &[(String, bool)] {
        &self.entries
    }
    pub fn loaded(&self) -> bool {
        self.loaded
    }
    pub fn detecting(&self) -> bool {
        self.pending.is_some()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Does not join or replace an in-flight scan, including a slow filesystem lookup.
    pub fn start(&mut self, scan: impl FnOnce() -> Outcome + Send + 'static) {
        if self.detecting() {
            return;
        }
        self.error = None;
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("agent-detection".into())
            .spawn(move || {
                let _ = sender.send(scan());
            }) {
            Ok(_) => self.pending = Some(receiver),
            Err(_) => self.error = Some("Could not start agent detection.".into()),
        }
    }

    pub fn poll(&mut self) {
        let Some(receiver) = &self.pending else {
            return;
        };
        let outcome = match receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("Agent detection stopped unexpectedly.".into()),
        };
        self.pending = None;
        match outcome {
            Ok(entries) => {
                self.entries = entries;
                self.loaded = true;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(detection: &mut Detection) -> mpsc::Sender<Outcome> {
        let (tx, rx) = mpsc::channel();
        detection.pending = Some(rx);
        tx
    }

    #[test]
    fn refresh_preserves_snapshot_until_success_and_never_blocks_poll() {
        let mut state = Detection::default();
        assert!(!state.loaded());
        let tx = pending(&mut state);
        state.poll();
        assert!(state.detecting());
        assert!(!state.loaded());
        tx.send(Ok(vec![("codex".into(), true)])).unwrap();
        state.poll();
        assert!(state.loaded());
        assert!(!state.detecting());
        let tx = pending(&mut state);
        state.poll();
        assert_eq!(state.entries(), &[("codex".into(), true)]);
        tx.send(Ok(vec![("codex".into(), false)])).unwrap();
        state.poll();
        assert_eq!(state.entries(), &[("codex".into(), false)]);
    }

    #[test]
    fn failure_or_disconnect_never_claims_an_empty_successful_scan() {
        let mut state = Detection::default();
        drop(pending(&mut state));
        state.poll();
        assert!(!state.loaded());
        assert!(state.error().is_some());
        let tx = pending(&mut state);
        tx.send(Ok(vec![("claude".into(), true)])).unwrap();
        state.poll();
        assert!(state.error().is_none());
        let tx = pending(&mut state);
        tx.send(Err("Synthetic failure".into())).unwrap();
        state.poll();
        assert!(state.loaded());
        assert_eq!(state.error(), Some("Synthetic failure"));
        assert_eq!(state.entries(), &[("claude".into(), true)]);
        drop(pending(&mut state));
        state.poll();
        assert_eq!(state.entries(), &[("claude".into(), true)]);
    }

    #[test]
    fn refresh_during_scan_does_not_run_another_worker() {
        let mut state = Detection::default();
        let tx = pending(&mut state);
        state.start(|| panic!("Duplicate scan must not execute"));
        tx.send(Ok(vec![])).unwrap();
        state.poll();
        assert!(state.loaded());
        assert!(state.entries().is_empty());
        assert!(state.error().is_none());
    }

    #[test]
    fn scan_runs_on_a_worker_and_publishes_its_result() {
        let mut state = Detection::default();
        let caller = std::thread::current().id();
        state.start(move || {
            assert_ne!(caller, std::thread::current().id());
            Ok(vec![("codex".into(), true)])
        });
        // Wait only in this test, never in UI code. Requeue the result for normal polling.
        let outcome = state
            .pending
            .take()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        pending(&mut state).send(outcome).unwrap();
        state.poll();
        assert_eq!(state.entries(), &[("codex".into(), true)]);
    }
}
