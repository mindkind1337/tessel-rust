//! Application-owned persistence and opening lifetime, independent of panes.
//! Construction starts a worker but does not read the owner store. Frames never
//! perform filesystem or network I/O.
use super::{host_lease_desk, host_lease_keeper, host_opening};
use std::sync::Arc;

pub struct Runtime {
    keeper: Arc<host_lease_keeper::Keeper>,
    desk: host_lease_desk::LeaseDesk,
    openings: host_opening::Openings,
    closing: bool,
}

#[derive(Debug)]
pub struct Drain {
    pub openings: usize,
    pub writing: bool,
    pub unsettled: host_lease_desk::Unsettled,
    pub ready: bool,
}

impl Runtime {
    pub fn start<W: host_lease_keeper::LeaseWriter>(
        writer: Arc<W>,
    ) -> Result<Self, host_lease_keeper::KeeperError> {
        let keeper = Arc::new(host_lease_keeper::Keeper::start(writer)?);
        Ok(Self {
            desk: host_lease_desk::LeaseDesk::new(keeper.clone()),
            keeper,
            openings: Default::default(),
            closing: false,
        })
    }

    pub fn keeper(&self) -> Arc<host_lease_keeper::Keeper> {
        self.keeper.clone()
    }
    pub fn desk(&self) -> host_lease_desk::DeskHandle {
        self.desk.handle()
    }
    pub fn openings(&self) -> host_opening::Openings {
        self.openings.clone()
    }
    pub fn closing(&self) -> bool {
        self.closing
    }

    /// Call only after the user confirms closing, before dropping pane handles.
    /// A mere close request must leave "Keep working" non-destructive.
    pub fn begin_close(&mut self) {
        self.closing = true;
        self.keeper.close_admissions();
    }

    pub fn frame(&self) -> Drain {
        self.desk.frame();
        let openings = self.openings.count();
        let mut writing = !self.keeper.is_idle();
        // Idle is published after the final report. Read reports again after
        // observing it, or a failed last write could look like a clean exit.
        if openings == 0 && !writing {
            // Draining can itself admit a waiting proof. Even if its write is
            // fast, wait another frame to collect that new write's result.
            writing = self.desk.frame();
        }
        let unsettled = self.desk.handle().unsettled();
        Drain {
            openings,
            writing,
            ready: self.closing && openings == 0 && !writing && unsettled.is_empty(),
            unsettled,
        }
    }
}
