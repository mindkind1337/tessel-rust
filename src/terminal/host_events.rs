//! Ordered, bounded host event application, independent of egui and network I/O.
use super::host_apply::{FRAME_BYTES, Pending};
use super::host_history::History;
use std::sync::mpsc::{Receiver, TryRecvError};
use tessel_host_client::terminal::{StreamRejection, TerminalEvent};
use tessel_host_protocol::pty::AttachMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    InvalidSnapshot,
    OversizedSnapshot,
    InvalidData,
}
impl From<StreamRejection> for Rejection {
    fn from(reason: StreamRejection) -> Self {
        match reason {
            StreamRejection::InvalidSnapshot => Self::InvalidSnapshot,
            StreamRejection::OversizedSnapshot => Self::OversizedSnapshot,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Receiving,
    Disconnected,
    Lost,
    Rejected(Rejection),
    Closed,
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
}
pub struct Feed {
    receiver: Option<Receiver<TerminalEvent>>,
    pending: Option<Pending>,
    pub history: History,
    pub state: State,
    pub applied: Option<u64>,
    pub lease: bool,
    pub resynchronized: bool,
    initialized: bool,
    awaiting_snapshot: bool,
    budget_exhausted: bool,
    reset_live_parser: bool,
}
impl Feed {
    /// A newly created terminal replays from sequence zero. Its empty screen is
    /// already valid even when the program waits for input without printing.
    /// Existing attachments must use `new` and await their initial snapshot.
    pub fn new_created(receiver: Receiver<TerminalEvent>, lease: bool) -> Self {
        let mut feed = Self::new(receiver, lease);
        feed.initialized = true;
        feed.applied = Some(0);
        feed
    }
    pub fn new(receiver: Receiver<TerminalEvent>, lease: bool) -> Self {
        Self {
            receiver: Some(receiver),
            pending: None,
            history: History::default(),
            state: State::Receiving,
            applied: None,
            lease,
            resynchronized: false,
            initialized: false,
            awaiting_snapshot: false,
            budget_exhausted: false,
            reset_live_parser: true,
        }
    }
    pub fn screen_ready(&self) -> bool {
        self.initialized
            && self.state == State::Receiving
            && !self.awaiting_snapshot
            && !self.pending.as_ref().is_some_and(Pending::is_snapshot)
    }
    /// Request another frame when buffered work exceeded this frame's budget.
    pub fn has_pending_work(&self) -> bool {
        !self.needs_detach() && (self.pending.is_some() || self.budget_exhausted)
    }
    /// Caller must drop the command Control immediately when true, so a silent
    /// stream cannot retain authority while no further event detects receiver closure.
    pub fn needs_detach(&self) -> bool {
        matches!(
            self.state,
            State::Lost | State::Rejected(_) | State::Closed | State::Exited { .. }
        )
    }
    fn reject(&mut self, reason: Rejection) {
        self.pending = None;
        self.receiver = None;
        self.lease = false;
        self.state = State::Rejected(reason);
    }
    /// Live bytes use the caller's emulator path. Snapshot bytes never invoke it,
    /// so replayed DSR requests cannot send replies. `writer` must reflect current
    /// worker authority, not just the lease advertised in an older stream event.
    pub fn poll(
        &mut self,
        parser: &mut vt100::Parser,
        writer: bool,
        mut live: impl FnMut(&mut vt100::Parser, &[u8], bool, bool),
    ) {
        self.budget_exhausted = false;
        if matches!(
            self.state,
            State::Lost | State::Rejected(_) | State::Closed | State::Exited { .. }
        ) {
            return;
        }
        let mut budget = FRAME_BYTES;
        for iteration in 0..64 {
            self.budget_exhausted = iteration == 63;
            if let Some(pending) = self.pending.as_mut() {
                let count = budget.min(pending.remaining_bytes());
                let snapshot = pending.is_snapshot();
                if snapshot {
                    pending.advance_snapshot(parser, &mut self.history, count);
                } else {
                    pending.apply_output(count, |bytes| {
                        live(
                            parser,
                            bytes,
                            writer
                                && self.lease
                                && self.state == State::Receiving
                                && !self.awaiting_snapshot,
                            std::mem::take(&mut self.reset_live_parser),
                        )
                    });
                }
                budget -= count;
                if let Some(sequence) = pending.completed_sequence() {
                    self.applied = Some(sequence);
                    self.initialized = true;
                    if snapshot {
                        self.awaiting_snapshot = false;
                        self.reset_live_parser = true;
                    }
                    self.pending = None;
                } else {
                    break;
                }
                if budget == 0 {
                    self.budget_exhausted = true;
                    break;
                }
            }
            let event = match self.receiver.as_ref().expect("live receiver").try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => {
                    self.budget_exhausted = false;
                    break;
                }
                Err(TryRecvError::Disconnected) => {
                    self.state = State::Closed;
                    self.lease = false;
                    self.receiver = None;
                    break;
                }
            };
            match event {
                TerminalEvent::Output { seq, data } => {
                    if self.applied.is_some_and(|last| seq <= last) {
                        continue;
                    }
                    match Pending::output(seq, data) {
                        Ok(pending) => self.pending = Some(pending),
                        Err(_) => {
                            self.reject(Rejection::InvalidData);
                            break;
                        }
                    }
                }
                TerminalEvent::Snapshot {
                    at,
                    size,
                    resync,
                    data,
                    ..
                } => {
                    if self.applied.is_some_and(|last| at < last) {
                        self.reject(Rejection::InvalidData);
                        break;
                    }
                    match Pending::snapshot(at, size.rows, size.cols, data) {
                        Ok(pending) => {
                            self.pending = Some(if self.initialized {
                                pending
                            } else {
                                pending.without_previous_screen()
                            });
                            self.resynchronized |= resync;
                        }
                        Err(_) => {
                            self.reject(Rejection::InvalidData);
                            break;
                        }
                    }
                }
                TerminalEvent::Resized { seq, size } => {
                    if self.applied.is_some_and(|last| seq <= last) {
                        continue;
                    }
                    if !(1..=500).contains(&size.rows) || !(1..=1000).contains(&size.cols) {
                        self.reject(Rejection::InvalidData);
                        break;
                    }
                    parser.set_size(size.rows, size.cols);
                    self.applied = Some(seq);
                    self.initialized = true;
                }
                TerminalEvent::Exit(exit) => {
                    if self.applied.is_some_and(|last| exit.seq < last) {
                        self.reject(Rejection::InvalidData);
                        break;
                    }
                    self.applied = Some(exit.seq);
                    self.lease = false;
                    self.state = State::Exited {
                        code: exit.code,
                        signal: exit.signal,
                    };
                    self.receiver = None;
                    self.pending = None;
                    break;
                }
                TerminalEvent::LeaseLost => self.lease = false,
                TerminalEvent::Disconnected => self.state = State::Disconnected,
                TerminalEvent::Reattached { lease, mode } => {
                    self.lease = lease;
                    self.state = State::Receiving;
                    self.awaiting_snapshot = mode == AttachMode::Snapshot;
                    self.resynchronized |= self.awaiting_snapshot && self.initialized;
                }
                TerminalEvent::Lost => {
                    self.lease = false;
                    self.state = State::Lost;
                    self.receiver = None;
                    self.pending = None;
                    break;
                }
                TerminalEvent::StreamRejected(reason) => {
                    self.reject(reason.into());
                    break;
                }
            }
        }
    }
}
