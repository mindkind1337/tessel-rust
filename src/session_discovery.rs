//! Single-flight discovery; repeated refreshes replace pending work, never spawn a fan-out.
use crate::side_panels_model::{
    SessionDiscoveryResult, SessionHistoryEntry, scan_past_sessions_cancellable,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

struct Request {
    home: Option<PathBuf>,
    live: Vec<SessionHistoryEntry>,
    limit: usize,
}
struct Job {
    generation: u64,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<SessionDiscoveryResult>,
}

#[derive(Default)]
pub(crate) struct Discovery {
    generation: u64,
    requested: bool,
    pending: Option<Request>,
    job: Option<Job>,
    pub limited: bool,
}
impl Drop for Discovery {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl Discovery {
    pub(crate) fn requested(&self) -> bool {
        self.requested
    }
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some() || self.job.is_some()
    }
    pub(crate) fn request(
        &mut self,
        home: Option<PathBuf>,
        live: Vec<SessionHistoryEntry>,
        limit: usize,
    ) {
        self.requested = true;
        self.generation = self.generation.wrapping_add(1);
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.pending = Some(Request { home, live, limit });
    }
    pub(crate) fn poll(&mut self) -> Option<Vec<SessionHistoryEntry>> {
        let mut completed = None;
        if let Some(job) = &self.job {
            match job.rx.try_recv() {
                Ok(result) => {
                    if job.generation == self.generation {
                        self.limited = result.limited;
                        completed = Some(result.entries);
                    }
                    self.job = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    if job.generation == self.generation {
                        self.limited = true;
                    }
                    self.job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.job.is_none()
            && let Some(request) = self.pending.take()
        {
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let (tx, rx) = mpsc::sync_channel(1);
            let spawn = std::thread::Builder::new()
                .name("session-discovery".into())
                .spawn(move || {
                    let result = scan_past_sessions_cancellable(
                        request.home.as_deref(),
                        &request.live,
                        request.limit,
                        &flag,
                    );
                    let _ = tx.send(result);
                });
            if spawn.is_ok() {
                self.job = Some(Job {
                    generation: self.generation,
                    cancel,
                    rx,
                });
            } else {
                self.limited = true;
            }
        }
        completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_refresh_is_single_flight_and_rejects_old_completion() {
        let mut discovery = Discovery::default();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(1);
        discovery.job = Some(Job {
            generation: 0,
            cancel: cancel.clone(),
            rx,
        });
        discovery.request(None, vec![], 50);
        assert!(cancel.load(Ordering::Relaxed));
        assert!(discovery.poll().is_none());
        assert!(discovery.pending.is_some());
        assert!(discovery.busy());
        tx.send(SessionDiscoveryResult {
            entries: vec![],
            limited: true,
        })
        .unwrap();
        assert!(discovery.poll().is_none());
        assert!(!discovery.limited);
        assert!(discovery.pending.is_none());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while discovery.busy() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
            discovery.poll();
        }
        assert!(discovery.requested()); // Empty discovery is a completed attempt, not a retry loop.
        assert!(!discovery.limited);
    }
}
