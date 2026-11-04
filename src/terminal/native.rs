//! Native host terminal ownership behind the terminal facade (Windows owner store).
//! Opening, commands and persistence run on workers; a frame only adopts and drains.
use super::{host_emulator, host_events, host_lease_keeper, host_opening, host_worker};
use crate::host_opening_adapter::{HostConnection, OwnerStore};
use std::sync::Arc;
use std::{
    cell::Cell,
    time::{Duration, Instant},
};

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;

type Opening = host_opening::Pending<HostConnection, OwnerStore>;
pub(super) type Failure = host_opening::Failure<HostConnection, OwnerStore>;

pub(super) struct Native {
    pub alias: String,
    pub host: HostConnection,
    pub pending: Option<Opening>,
    pub failure: Option<Failure>,
    pub slot: Option<host_opening::Slot>,
    pub feed: Option<host_events::Feed>,
    pub control: Option<host_worker::Control>,
    pub saved: bool,
    pub retry_save: Option<host_lease_keeper::RetrySave>,
    awaiting: Option<host_opening::Opened<HostConnection>>,
    cancelling: Option<std::sync::mpsc::Receiver<Failure>>,
    admission_paused: bool,
    target: host_opening::Target,
    requested_size: Option<(u16, u16)>,
    pub allow_replies: bool,
    keeper: Arc<host_lease_keeper::Keeper>,
    forget: Option<host_lease_keeper::Ended>,
    end_observed: bool,
    active_until: Cell<Instant>,
    terminating: bool,
    repaint: eframe::egui::Context,
}

impl Native {
    pub fn open(
        host: HostConnection,
        target: host_opening::Target,
        keeper: Arc<host_lease_keeper::Keeper>,
        repaint: eframe::egui::Context,
    ) -> Self {
        let alias = match &target {
            host_opening::Target::New(spec) => spec.alias.clone(),
            host_opening::Target::Existing { alias, .. } => alias.clone(),
        };
        let wake = repaint.clone();
        let result = host_opening::open(
            host.clone(),
            Arc::new(OwnerStore),
            target.clone(),
            move || wake.request_repaint(),
        );
        let (pending, failure) = match result {
            Ok(pending) => (Some(pending), None),
            Err(failure) => (None, Some(failure)),
        };
        Self {
            alias,
            host,
            pending,
            failure,
            slot: None,
            feed: None,
            control: None,
            saved: true,
            retry_save: None,
            awaiting: None,
            cancelling: None,
            admission_paused: false,
            target,
            requested_size: None,
            allow_replies: false,
            keeper,
            forget: None,
            end_observed: false,
            active_until: Cell::new(Instant::now() + Duration::from_secs(1)),
            terminating: false,
            repaint,
        }
    }

    pub fn writable(&self) -> bool {
        self.feed
            .as_ref()
            .is_some_and(|feed| feed.screen_ready() && feed.lease)
            && self
                .control
                .as_ref()
                .is_some_and(host_worker::Control::writable)
    }

    /// Cancellation is explicit and only applies before command-worker adoption.
    /// Once adopted, closing the pane must detach without ending the program.
    pub fn cancel_opening(&mut self, status: &mut String) -> bool {
        if self.cancelling.is_some() {
            return true;
        }
        if let Some(pending) = &self.pending {
            pending.cancel();
            return true;
        }
        if let Some(opened) = self.awaiting.take() {
            let wake = self.repaint.clone();
            match host_opening::cancel_adopted(opened, Arc::new(OwnerStore), move || {
                wake.request_repaint()
            }) {
                Ok(receiver) => {
                    self.admission_paused = false;
                    self.cancelling = Some(receiver);
                }
                Err(opened) => {
                    self.awaiting = Some(*opened);
                    *status =
                        crate::i18n::tr("Could not start terminal cancellation; try again.").into();
                }
            }
            return true;
        }
        false
    }

    pub fn retry_opening(&mut self) -> bool {
        let Some(failure) = self.failure.take() else {
            return false;
        };
        let wake = self.repaint.clone();
        let result = match failure {
            Failure::Ambiguous(retry) | Failure::Deferred(retry) => {
                retry.open(move || wake.request_repaint())
            }
            Failure::Capacity | Failure::NotConnected | Failure::StoreUnavailable => {
                host_opening::open(
                    self.host.clone(),
                    Arc::new(OwnerStore),
                    self.target.clone(),
                    move || wake.request_repaint(),
                )
            }
            other => {
                self.failure = Some(other);
                return false;
            }
        };
        match result {
            Ok(pending) => self.pending = Some(pending),
            Err(failure) => self.failure = Some(failure),
        }
        true
    }

    pub fn can_retry_opening(&self) -> bool {
        matches!(
            self.failure,
            Some(
                Failure::Ambiguous(_)
                    | Failure::Deferred(_)
                    | Failure::Capacity
                    | Failure::NotConnected
                    | Failure::StoreUnavailable
            )
        )
    }

    pub fn capabilities(&self) -> crate::host_terminal_panel::HostTerminalCapabilities {
        crate::host_terminal_panel::HostTerminalCapabilities {
            can_terminate: self.writable(),
            can_detach: true,
            can_take_control: self
                .control
                .as_ref()
                .is_some_and(host_worker::Control::online),
            can_retry_opening: self.can_retry_opening(),
            can_retry_save_lease: self.retry_save.is_some(),
            ..Default::default()
        }
    }

    pub fn retry_save(&mut self) {
        let Some(retry) = self.retry_save.take() else {
            return;
        };
        match self.keeper.retry_save(retry) {
            Ok(()) | Err(host_lease_keeper::RetryRefused::Superseded) => {}
            Err(
                host_lease_keeper::RetryRefused::Busy(retry)
                | host_lease_keeper::RetryRefused::Closed(retry),
            ) => {
                self.retry_save = Some(retry);
            }
        }
    }

    pub fn phase(&self) -> crate::host_terminal_panel::HostTerminalPhase {
        use crate::host_terminal_panel::{
            HostTerminalPhase as Phase, LostReason, OpeningFailureReason as Failed, ReadOnlyReason,
            StreamRejectReason,
        };
        if self.admission_paused {
            return Phase::Opening { note: Some(crate::i18n::tr("Terminal access could not be queued for storage. Cancel this opening to continue.").into()) };
        }
        if let Some(failure) = &self.failure {
            let reason = match failure {
                Failure::Capacity => Failed::Capacity,
                Failure::Deferred(_) => Failed::DeferredRetry,
                Failure::NotConnected => Failed::NotConnected,
                Failure::StoreUnavailable => Failed::StoreUnavailable,
                Failure::Ambiguous(_) => Failed::Ambiguous,
                Failure::Expired => Failed::RetryExpired,
                Failure::Detached(_) => Failed::Detached,
                Failure::CancelNotConfirmed(_) => Failed::CancelNotConfirmed,
                Failure::Stopped => Failed::Stopped,
                Failure::EpochChanged => {
                    return Phase::Lost {
                        reason: LostReason::EpochChanged,
                    };
                }
                Failure::Gone => {
                    return Phase::Lost {
                        reason: LostReason::SessionGone,
                    };
                }
                Failure::Cancelled => {
                    return Phase::Lost {
                        reason: LostReason::StartFailed {
                            detail: "Opening cancelled".into(),
                        },
                    };
                }
            };
            return Phase::OpeningFailed { reason };
        }
        let Some(feed) = &self.feed else {
            return Phase::Opening { note: None };
        };
        match feed.state {
            host_events::State::Exited { code, signal } => Phase::Exited {
                exit_code: code,
                signal,
            },
            host_events::State::Lost => {
                Phase::Lost {
                    reason: if self.host.client.epoch().is_some_and(|epoch| {
                        self.slot.as_ref().is_some_and(|slot| slot.epoch != epoch)
                    }) {
                        LostReason::EpochChanged
                    } else {
                        LostReason::Unavailable
                    },
                }
            }
            host_events::State::Closed => Phase::StreamRejected {
                reason: StreamRejectReason::Closed,
            },
            host_events::State::Rejected(reason) => Phase::StreamRejected {
                reason: match reason {
                    host_events::Rejection::InvalidSnapshot => StreamRejectReason::InvalidSnapshot,
                    host_events::Rejection::OversizedSnapshot => {
                        StreamRejectReason::OversizedSnapshot
                    }
                    host_events::Rejection::InvalidData => StreamRejectReason::InvalidData,
                },
            },
            host_events::State::Disconnected => Phase::Reconnecting {
                attempt: 0,
                max_attempts: None,
            },
            host_events::State::Receiving => {
                if self.terminating {
                    return Phase::Opening {
                        note: Some(
                            crate::i18n::tr("Waiting for the remote program to exit").into(),
                        ),
                    };
                }
                if !self
                    .control
                    .as_ref()
                    .is_some_and(host_worker::Control::online)
                {
                    Phase::Reconnecting {
                        attempt: 0,
                        max_attempts: None,
                    }
                } else if !feed.screen_ready() {
                    Phase::Opening {
                        note: Some("Restoring remote screen".into()),
                    }
                } else if self.writable() {
                    Phase::Live
                } else {
                    Phase::ReadOnly {
                        reason: if matches!(
                            self.target,
                            host_opening::Target::Existing { resume: false, .. }
                        ) {
                            ReadOnlyReason::ObserverAttach
                        } else {
                            ReadOnlyReason::ControlledElsewhere
                        },
                    }
                }
            }
        }
    }

    pub fn send(&self, bytes: Vec<u8>) -> Result<(), String> {
        let bytes = zeroize::Zeroizing::new(bytes);
        if !self.writable() {
            return Err("Remote terminal is not writable".into());
        }
        self.active_until
            .set(Instant::now() + Duration::from_secs(1));
        self.repaint
            .request_repaint_after(Duration::from_millis(20));
        self.control
            .as_ref()
            .expect("writable control")
            .input(&bytes)
            .map_err(|_| "Remote input was not queued".into())
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String> {
        if !self.writable() {
            return Err("Remote terminal is not writable".into());
        }
        if self.requested_size == Some((rows, cols)) {
            return Ok(());
        }
        self.control
            .as_ref()
            .expect("writable control")
            .resize(rows, cols)
            .map_err(|_| "Remote resize was not queued".to_owned())?;
        self.requested_size = Some((rows, cols));
        Ok(())
    }

    /// Returns whether the emulator changed. It never changes its size before a
    /// remote Resized event, and never sends replies while restoring a snapshot.
    pub fn poll(
        &mut self,
        parser: &mut vt100::Parser,
        query_tail: &mut Vec<u8>,
        status: &mut String,
    ) -> bool {
        if let Some(receiver) = &self.cancelling {
            match receiver.try_recv() {
                Ok(failure) => {
                    self.failure = Some(failure);
                    self.cancelling = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.failure = Some(Failure::Stopped);
                    self.cancelling = None;
                }
            }
        }
        if let Some(pending) = &self.pending {
            match pending.poll() {
                host_opening::Poll::Waiting => {}
                host_opening::Poll::Failed(failure) => {
                    *status = failure.code().into();
                    self.failure = Some(failure);
                    self.pending = None;
                }
                host_opening::Poll::Ready(opened) => {
                    self.pending = None;
                    self.awaiting = Some(opened);
                }
            }
        }
        if !self.admission_paused
            && let Some(mut opened) = self.awaiting.take()
        {
            let slot = host_opening::Slot {
                alias: opened.alias.clone(),
                epoch: opened.epoch.clone(),
                terminal: opened.id.clone(),
            };
            self.slot = Some(slot.clone());
            self.saved = opened.saved;
            let mut admitted = true;
            if !opened.saved
                && let Some(lease) = opened.recovery_lease.take()
            {
                if let Some(pending) = opened.pending.take() {
                    if let Err(refused) = self.keeper.promote(slot.clone(), pending, lease) {
                        let (reason, _, pending, lease) = *refused;
                        opened.pending = Some(pending);
                        opened.recovery_lease = Some(lease);
                        self.admission_paused = reason != host_lease_keeper::Refusal::Busy;
                        admitted = false;
                    }
                } else if let Err((reason, _, lease)) = self.keeper.keep(slot.clone(), lease) {
                    opened.recovery_lease = Some(lease);
                    self.admission_paused = reason != host_lease_keeper::Refusal::Busy;
                    admitted = false;
                }
            }
            if !admitted {
                *status = "Waiting to queue remote terminal access for storage".into();
                self.awaiting = Some(opened);
            } else {
                // A new attachment starts with a fresh emulator, even when the
                // server chooses byte replay rather than a screen snapshot.
                let (rows, cols) = match &self.target {
                    host_opening::Target::New(spec) => (spec.size.rows, spec.size.cols),
                    host_opening::Target::Existing { .. } => (30, 100),
                };
                *parser = vt100::Parser::new(rows, cols, 1000);
                query_tail.clear();
                let mut feed = if opened.created {
                    host_events::Feed::new_created(opened.events, false)
                } else {
                    host_events::Feed::new(opened.events, false)
                };
                match host_worker::start(
                    opened.permit,
                    self.host.client.clone(),
                    opened.terminal,
                    slot,
                    self.keeper.clone(),
                ) {
                    Ok(control) => {
                        feed.lease = control.writable();
                        self.control = Some(control);
                        self.feed = Some(feed);
                        status.clear();
                    }
                    Err(error) => {
                        *status = error.into();
                        self.failure = Some(Failure::Stopped);
                    }
                }
            }
        }
        let mut changed = false;
        if let Some(control) = &self.control {
            while let Some(outcome) = control.outcome() {
                match outcome {
                    host_worker::Outcome::ControlTaken => {
                        if let Some(feed) = &mut self.feed {
                            feed.lease = true;
                        }
                    }
                    host_worker::Outcome::Terminated => {
                        self.terminating = true;
                        *status = crate::i18n::tr("Waiting for the remote program to exit").into();
                    }
                    host_worker::Outcome::InputUncertain => {
                        *status = "Remote input may have arrived; it was not sent again".into()
                    }
                    host_worker::Outcome::Busy => {
                        *status = "Terminal access storage is busy; retry the action".into()
                    }
                    host_worker::Outcome::Refused | host_worker::Outcome::Failed => {
                        *status = "Remote terminal action was not confirmed".into()
                    }
                }
            }
        }
        if let Some(feed) = &mut self.feed {
            let previous = feed.applied;
            let control = self.control.as_ref();
            feed.poll(
                parser,
                self.allow_replies && control.is_some_and(host_worker::Control::writable),
                |parser, bytes, can_reply, reset| {
                    if reset {
                        query_tail.clear();
                    }
                    if let Some(error) =
                        host_emulator::apply(parser, query_tail, bytes, can_reply, |reply| {
                            control
                                .ok_or_else(|| "Remote terminal detached".to_owned())?
                                .input(&reply)
                                .map_err(|_| "Remote cursor reply was not queued".to_owned())
                        })
                    {
                        *status = error;
                    }
                },
            );
            changed = previous != feed.applied;
            if feed.has_pending_work() {
                self.repaint.request_repaint();
            }
            if feed.needs_detach() {
                self.control.take();
                if !self.end_observed {
                    self.end_observed = true;
                    self.forget = match feed.state {
                        host_events::State::Exited { .. } => {
                            Some(host_lease_keeper::Ended::ExitReceived)
                        }
                        host_events::State::Lost => self
                            .host
                            .client
                            .epoch()
                            .filter(|epoch| self.slot.as_ref().is_some_and(|s| &s.epoch != epoch))
                            .map(|current| host_lease_keeper::Ended::EpochChanged { current }),
                        _ => None,
                    };
                }
            }
        }
        if let (Some(slot), Some(proof)) = (&self.slot, self.forget.take()) {
            // A refused admission retains proof for another frame; no filesystem work here.
            let retry = match &proof {
                host_lease_keeper::Ended::ExitReceived => host_lease_keeper::Ended::ExitReceived,
                host_lease_keeper::Ended::CloseConfirmed => {
                    host_lease_keeper::Ended::CloseConfirmed
                }
                host_lease_keeper::Ended::EpochChanged { current } => {
                    host_lease_keeper::Ended::EpochChanged {
                        current: current.clone(),
                    }
                }
            };
            match self.keeper.forget_ended(slot.clone(), proof) {
                Ok(()) => self.retry_save = None,
                Err(host_lease_keeper::Refusal::Busy) => self.forget = Some(retry),
                Err(_) => *status = "Ended terminal access could not be queued for cleanup".into(),
            }
        }
        if self.pending.is_some()
            || self.cancelling.is_some()
            || self.awaiting.is_some()
            || self.control.is_some()
            || self.forget.is_some()
        {
            if changed {
                self.active_until
                    .set(Instant::now() + Duration::from_secs(1));
            }
            self.repaint
                .request_repaint_after(std::time::Duration::from_millis(
                    if Instant::now() < self.active_until.get() {
                        20
                    } else {
                        250
                    },
                ));
        }
        if !self.writable() || self.requested_size == Some(parser.screen().size()) {
            self.requested_size = None;
        }
        changed
    }
}
