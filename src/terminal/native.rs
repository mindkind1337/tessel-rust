//! Native host terminal ownership behind the terminal facade (Windows owner store).
//! Opening, commands and persistence run on workers; a frame only adopts and drains.
use super::{
    host_emulator, host_events, host_lease_desk, host_lease_keeper, host_opening, host_worker,
};
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
    pub scroll: super::host_viewport::ScrollState,
    pub selection_invalidated: bool,
    pub control: Option<host_worker::Control>,
    pub saved: bool,
    desk: host_lease_desk::DeskHandle,
    openings: host_opening::Openings,
    registration: Option<Box<dyn Send>>,
    /// Informational only: reconcile with encrypted store before cleanup. A
    /// newer lease may supersede creation without consuming this intent.
    creation_record: Option<String>,
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
        runtime: &super::host_runtime::Runtime,
        repaint: eframe::egui::Context,
    ) -> Self {
        let keeper = runtime.keeper();
        let desk = runtime.desk();
        let openings = runtime.openings();
        let alias = match &target {
            host_opening::Target::New(spec) => spec.alias.clone(),
            host_opening::Target::Existing { alias, .. } => alias.clone(),
        };
        let wake = repaint.clone();
        let result = host_opening::open_managed(
            host.clone(),
            Arc::new(OwnerStore),
            target.clone(),
            host_opening::Management {
                keeper: keeper.clone(),
                registry: Arc::new(desk.clone()),
                openings: openings.clone(),
            },
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
            scroll: Default::default(),
            selection_invalidated: false,
            control: None,
            saved: true,
            desk,
            openings,
            registration: None,
            creation_record: None,
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
                host_opening::open_managed(
                    self.host.clone(),
                    Arc::new(OwnerStore),
                    self.target.clone(),
                    host_opening::Management {
                        keeper: self.keeper.clone(),
                        registry: Arc::new(self.desk.clone()),
                        openings: self.openings.clone(),
                    },
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
            can_retry_save_lease: self.slot.as_ref().is_some_and(|slot| {
                matches!(
                    self.desk.view(slot),
                    host_lease_desk::View::NotSaved | host_lease_desk::View::NotForgotten
                )
            }),
            ..Default::default()
        }
    }

    pub fn retry_save(&mut self) {
        if let Some(slot) = &self.slot {
            self.desk.retry(slot);
        }
    }

    pub fn lease_access(&self) -> crate::host_terminal_panel::LeaseSaveState {
        use crate::host_terminal_panel::LeaseSaveState as Access;
        let view = self.slot.as_ref().map(|slot| self.desk.view(slot));
        match view {
            Some(host_lease_desk::View::NotSaved) => Access::NotSaved,
            Some(host_lease_desk::View::Superseded) => Access::Superseded,
            Some(
                host_lease_desk::View::Saved
                | host_lease_desk::View::Forgotten
                | host_lease_desk::View::Ending,
            ) => Access::Ok,
            _ if !self.saved => Access::Saving,
            _ => Access::Ok,
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
                            detail: crate::i18n::tr("Opening cancelled").into(),
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
                        note: Some(crate::i18n::tr("Restoring remote screen").into()),
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
            return Err(crate::i18n::tr("Remote terminal is not writable").into());
        }
        self.active_until
            .set(Instant::now() + Duration::from_secs(1));
        self.repaint
            .request_repaint_after(Duration::from_millis(20));
        self.control
            .as_ref()
            .expect("writable control")
            .input(&bytes)
            .map_err(|_| crate::i18n::tr("Remote input was not queued").into())
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String> {
        if !self.writable() {
            return Err(crate::i18n::tr("Remote terminal is not writable").into());
        }
        if self.requested_size == Some((rows, cols)) {
            return Ok(());
        }
        self.control
            .as_ref()
            .expect("writable control")
            .resize(rows, cols)
            .map_err(|_| crate::i18n::tr("Remote resize was not queued").to_owned())?;
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
        self.selection_invalidated = false;
        if let Some(slot) = &self.slot {
            match self.desk.view(slot) {
                host_lease_desk::View::Saved | host_lease_desk::View::Forgotten => {
                    self.saved = true
                }
                host_lease_desk::View::NotSaved => self.saved = false,
                _ => {}
            }
        }
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
                && !opened.queued
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
                *status =
                    crate::i18n::tr("Waiting to queue remote terminal access for storage").into();
                self.awaiting = Some(opened);
            } else {
                self.registration = opened.registration.take();
                self.creation_record = opened.pending.take();
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
        let current_epoch = self
            .host
            .client
            .link()
            .ok()
            .and_then(|_| self.host.client.epoch());
        let mut changed = self.observe_epoch(current_epoch);
        if changed {
            query_tail.clear();
        }
        if let Some(control) = &self.control {
            while let Some(outcome) = control.outcome() {
                match outcome {
                    host_worker::Outcome::ControlTaken => {
                        self.saved = false;
                        if let host_opening::Target::Existing { resume, .. } = &mut self.target {
                            *resume = true;
                        }
                        if let Some(feed) = &mut self.feed {
                            feed.lease = true;
                        }
                    }
                    host_worker::Outcome::Terminated => {
                        self.terminating = true;
                        *status = crate::i18n::tr("Waiting for the remote program to exit").into();
                    }
                    host_worker::Outcome::InputUncertain => {
                        *status =
                            crate::i18n::tr("Remote input may have arrived; it was not sent again")
                                .into()
                    }
                    host_worker::Outcome::Busy => {
                        *status =
                            crate::i18n::tr("Terminal access storage is busy; retry the action")
                                .into()
                    }
                    host_worker::Outcome::Refused | host_worker::Outcome::Failed => {
                        *status = crate::i18n::tr("Remote terminal action was not confirmed").into()
                    }
                }
            }
        }
        if let Some(feed) = &mut self.feed {
            let anchor = self.scroll.before_poll(&feed.history, parser);
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
                                .ok_or_else(|| {
                                    crate::i18n::tr("Remote terminal detached").to_owned()
                                })?
                                .input(&reply)
                                .map_err(|_| {
                                    crate::i18n::tr("Remote cursor reply was not queued").to_owned()
                                })
                        })
                    {
                        *status = error;
                    }
                },
            );
            changed |= previous != feed.applied;
            self.selection_invalidated = self
                .scroll
                .after_poll(anchor, &feed.history, parser, previous != feed.applied)
                .clear_selection;
            if feed.has_pending_work() {
                self.repaint.request_repaint();
            }
            if feed.needs_detach() {
                self.control.take();
                if !self.end_observed {
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
                    self.end_observed = self.forget.is_some();
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
            match self.desk.hand_proof(slot.clone(), proof) {
                Ok(()) => {}
                Err(host_lease_desk::ProofRefused::Full(_, _)) => self.forget = Some(retry),
                Err(_) => {
                    *status =
                        crate::i18n::tr("Ended terminal access could not be queued for cleanup")
                            .into()
                }
            }
        }
        if self.pending.is_some()
            || self.cancelling.is_some()
            || self.awaiting.is_some()
            || self.control.is_some()
            || self.forget.is_some()
            || !self.end_observed
                && self
                    .feed
                    .as_ref()
                    .is_some_and(host_events::Feed::needs_detach)
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

    /// A rejected/closed stream no longer receives Lost events. The live host
    /// epoch must therefore also invalidate its old authority independently of
    /// that stream. Disconnection alone is not proof that its program ended.
    /// This pane retains one Client whose connector command is pinned to one
    /// daemon version. Installing another version creates another Client; an
    /// old slot refused during opening is never adopted here. Do not feed an
    /// epoch discovered through a different Connection into this method.
    fn observe_epoch(&mut self, current: Option<tessel_host_protocol::Epoch>) -> bool {
        let Some(current) = current else {
            return false;
        };
        let Some(slot) = &self.slot else {
            return false;
        };
        if current == slot.epoch {
            return false;
        }
        let Some(feed) = &mut self.feed else {
            return false;
        };
        if matches!(feed.state, host_events::State::Exited { .. }) {
            return false;
        }
        let changed = feed.state != host_events::State::Lost;
        feed.state = host_events::State::Lost;
        feed.lease = false;
        self.control.take();
        self.requested_size = None;
        if !self.end_observed {
            self.forget = Some(host_lease_keeper::Ended::EpochChanged { current });
            self.end_observed = true;
        }
        changed
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        if let (Some(slot), Some(proof)) = (&self.slot, self.forget.take())
            && matches!(
                self.desk.hand_proof(slot.clone(), proof),
                Err(host_lease_desk::ProofRefused::Full(_, _))
            )
        {
            self.desk.count_unrecorded_end();
        }
    }
}
