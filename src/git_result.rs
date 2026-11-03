//! Keeps asynchronous Git output attached to the project that requested it.
use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub project: u64,
    pub root: PathBuf,
}

#[derive(Default)]
pub struct GitResult {
    pub output: String,
    displayed: Option<Owner>,
    pending: Option<(Owner, Receiver<Result<String, String>>)>,
}

impl GitResult {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn pending_owner(&self) -> Option<&Owner> {
        self.pending.as_ref().map(|(owner, _)| owner)
    }

    pub fn select(&mut self, owner: Option<Owner>) -> bool {
        if self.displayed == owner {
            return false;
        }
        self.displayed = owner;
        self.output.clear();
        true
    }

    pub fn start(&mut self, owner: Owner, receiver: Receiver<Result<String, String>>) -> bool {
        if self.busy() {
            return false;
        }
        self.select(Some(owner.clone()));
        self.pending = Some((owner, receiver));
        true
    }

    pub fn poll(&mut self) {
        let Some((owner, receiver)) = self.pending.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("Git worker stopped without a result.".into()),
        };
        if self.displayed.as_ref() == Some(owner) {
            self.output = result.unwrap_or_else(|error| format!("Error: {error}"));
        }
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    fn owner(project: u64, root: &str) -> Owner {
        Owner {
            project,
            root: root.into(),
        }
    }
    #[test]
    fn late_results_never_appear_under_a_different_project_or_path() {
        let mut state = GitResult::default();
        let (sender, receiver) = mpsc::channel();
        assert!(state.start(owner(1, "first"), receiver));
        state.select(Some(owner(2, "second")));
        assert_eq!(state.pending_owner(), Some(&owner(1, "first")));
        state.poll();
        assert!(state.busy());
        sender.send(Ok("first output".into())).unwrap();
        state.poll();
        assert!(!state.busy());
        assert!(state.output.is_empty());
        let (sender, receiver) = mpsc::channel();
        assert!(state.start(owner(2, "second"), receiver));
        state.select(Some(owner(2, "replacement")));
        sender.send(Ok("old path output".into())).unwrap();
        state.poll();
        assert!(state.output.is_empty());
    }
    #[test]
    fn returning_to_owner_accepts_result_and_disconnected_worker_releases_busy() {
        let mut state = GitResult::default();
        let (sender, receiver) = mpsc::channel();
        state.start(owner(1, "first"), receiver);
        state.select(None);
        state.select(Some(owner(1, "first")));
        sender.send(Ok("correct output".into())).unwrap();
        state.poll();
        assert_eq!(state.output, "correct output");
        let (sender, receiver) = mpsc::channel();
        state.start(owner(1, "first"), receiver);
        drop(sender);
        state.poll();
        assert!(!state.busy());
        assert!(state.output.contains("stopped"));
    }
}
