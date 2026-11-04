//! Epoch-bound, non-secret host persistence metadata. `Probe::poll` belongs on
//! the existing connection worker; only `Cache::read` is used on UI frames.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tessel_host_protocol::{Epoch, Persistence};

pub trait Source {
    fn epoch(&self) -> Option<Epoch>;
    /// A bounded request on the connection worker, never on the UI thread.
    fn persistence(&self) -> Option<Persistence>;
}

impl Source for tessel_host_client::Client {
    fn epoch(&self) -> Option<Epoch> {
        self.link().ok()?;
        self.epoch()
    }

    fn persistence(&self) -> Option<Persistence> {
        let report: tessel_host_protocol::HostInfo =
            self.ask("host.info", &serde_json::Value::Null).ok()?;
        report.validate().ok()?;
        Some(report.persistence)
    }
}

#[derive(Clone, Default)]
pub struct Cache(Arc<Shared>);

#[derive(Default)]
struct Shared {
    report: Mutex<Option<(Epoch, Persistence)>>,
    closed: AtomicBool,
}

impl Cache {
    /// A disconnected client or mismatched epoch never inherits an old report.
    pub fn read(&self, epoch: Option<&Epoch>) -> Persistence {
        let cached = self.0.report.lock().unwrap();
        if self.0.closed.load(Ordering::Acquire) {
            return Persistence::UNKNOWN;
        }
        match (&*cached, epoch) {
            (Some((known, report)), Some(current)) if known == current => *report,
            _ => Persistence::UNKNOWN,
        }
    }

    /// Owner connection removal invalidates all pane readers immediately, even
    /// while a metadata request is still completing on its worker.
    pub fn close(&self) {
        self.0.closed.store(true, Ordering::Release);
        *self.0.report.lock().unwrap() = None;
    }
}

#[derive(Default)]
pub struct Probe {
    cache: Cache,
    attempted: Option<Epoch>,
}

impl Probe {
    #[cfg(test)]
    pub fn cache(&self) -> Cache {
        self.cache.clone()
    }

    /// At most one request per observed epoch, unless a disconnection intervenes.
    /// Failed reads remain unknown; polling frames cannot cause request floods.
    pub fn poll(&mut self, source: &impl Source) {
        if self.cache.0.closed.load(Ordering::Acquire) {
            return;
        }
        let Some(epoch) = source.epoch() else {
            self.attempted = None;
            *self.cache.0.report.lock().unwrap() = None;
            return;
        };
        if self.attempted.as_ref() == Some(&epoch) {
            return;
        }
        self.attempted = Some(epoch.clone());
        *self.cache.0.report.lock().unwrap() = None;
        let report = source.persistence().unwrap_or(Persistence::UNKNOWN);
        if source.epoch().as_ref() == Some(&epoch) {
            let mut cached = self.cache.0.report.lock().unwrap();
            if !self.cache.0.closed.load(Ordering::Acquire) {
                *cached = Some((epoch, report));
            }
        }
    }
}

// Matches the connection manager's maximum live owners. The permit is retained
// until the metadata thread actually exits, including after owner cancellation.
const MAX_WORKERS: usize = 8;
static WORKERS: AtomicUsize = AtomicUsize::new(0);
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Owned and joined only by the existing background connection worker. UI owners
/// receive a stop sender and a cache; they never join a thread or close a client.
pub struct Worker {
    stop: Sender<()>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn start(client: tessel_host_client::Client, cache: Cache) -> Option<Self> {
        WORKERS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_WORKERS).then_some(n + 1)
            })
            .ok()?;
        let permit = Permit;
        let (stop, received) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("host-info".into())
            .spawn(move || {
                let _permit = permit;
                run(client, cache, received);
            })
            .ok()?;
        Some(Self {
            stop,
            thread: Some(thread),
        })
    }
    pub fn stop_sender(&self) -> Sender<()> {
        self.stop.clone()
    }
    /// First close the client on the connection worker to interrupt an in-flight
    /// RPC, then call this. Never call from a UI frame or an owner Drop.
    pub fn finish(mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

fn run(client: tessel_host_client::Client, cache: Cache, stop: Receiver<()>) {
    let mut probe = Probe {
        cache: cache.clone(),
        attempted: None,
    };
    loop {
        if cache.0.closed.load(Ordering::Acquire)
            || matches!(client.state(), tessel_host_client::ConnectionState::Closed)
            || !matches!(stop.try_recv(), Err(mpsc::TryRecvError::Empty))
        {
            break;
        }
        probe.poll(&client);
        if !matches!(
            stop.recv_timeout(Duration::from_secs(1)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            break;
        }
    }
    cache.close();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use tessel_host_protocol::{PersistenceMethod, Survival};

    struct Fixture {
        epoch: RefCell<Option<Epoch>>,
        after_request: RefCell<Option<Option<Epoch>>>,
        result: Cell<Option<Persistence>>,
        requests: Cell<usize>,
        close_during_request: RefCell<Option<Cache>>,
    }
    impl Source for Fixture {
        fn epoch(&self) -> Option<Epoch> {
            self.epoch.borrow().clone()
        }
        fn persistence(&self) -> Option<Persistence> {
            self.requests.set(self.requests.get() + 1);
            if let Some(cache) = self.close_during_request.borrow_mut().take() {
                cache.close();
            }
            if let Some(epoch) = self.after_request.borrow_mut().take() {
                *self.epoch.borrow_mut() = epoch;
            }
            self.result.get()
        }
    }
    fn fixture() -> Fixture {
        Fixture {
            epoch: RefCell::new(Some(Epoch::from_bytes([1; 16]))),
            after_request: RefCell::new(None),
            result: Cell::new(Some(Persistence {
                method: PersistenceMethod::UserScope,
                linger: Some(true),
                kill_user_processes: Some(false),
                survives_logout: Survival::Yes,
            })),
            requests: Cell::new(0),
            close_during_request: RefCell::new(None),
        }
    }

    #[test]
    fn readers_share_one_report_without_triggering_requests() {
        let source = fixture();
        let mut probe = Probe::default();
        let first = probe.cache();
        let second = first.clone();
        assert_eq!(
            first.read(source.epoch().as_ref()).survives_logout,
            Survival::Unknown
        );
        for _ in 0..100 {
            probe.poll(&source);
        }
        assert_eq!(source.requests.get(), 1);
        assert_eq!(
            first.read(source.epoch().as_ref()),
            second.read(source.epoch().as_ref())
        );
        assert_eq!(
            first.read(source.epoch().as_ref()).survives_logout,
            Survival::Yes
        );
        assert_eq!(first.read(None).survives_logout, Survival::Unknown);
        assert_eq!(
            first
                .read(Some(&Epoch::from_bytes([2; 16])))
                .survives_logout,
            Survival::Unknown
        );
    }

    #[test]
    fn an_epoch_change_during_the_request_cannot_publish_the_old_report() {
        let source = fixture();
        let mut probe = Probe::default();
        *source.after_request.borrow_mut() = Some(Some(Epoch::from_bytes([2; 16])));
        probe.poll(&source);
        assert_eq!(
            probe.cache().read(source.epoch().as_ref()).survives_logout,
            Survival::Unknown
        );
        source.result.set(None);
        probe.poll(&source);
        assert_eq!(source.requests.get(), 2);
        assert_eq!(
            probe.cache().read(source.epoch().as_ref()).survives_logout,
            Survival::Unknown
        );
    }

    #[test]
    fn failure_does_not_spin_and_disconnect_invalidates_even_same_epoch_reconnect() {
        let source = fixture();
        source.result.set(None);
        let mut probe = Probe::default();
        for _ in 0..100 {
            probe.poll(&source);
        }
        assert_eq!(source.requests.get(), 1);
        let epoch = source.epoch.borrow_mut().take();
        probe.poll(&source);
        assert_eq!(
            probe.cache().read(epoch.as_ref()).survives_logout,
            Survival::Unknown
        );
        *source.epoch.borrow_mut() = epoch;
        probe.poll(&source);
        assert_eq!(source.requests.get(), 2);
    }

    #[test]
    fn disconnection_during_request_does_not_publish_a_survival_guarantee() {
        let source = fixture();
        let mut probe = Probe::default();
        *source.after_request.borrow_mut() = Some(None);
        probe.poll(&source);
        assert_eq!(
            probe
                .cache()
                .read(Some(&Epoch::from_bytes([1; 16])))
                .survives_logout,
            Survival::Unknown
        );
    }

    #[test]
    fn owner_drop_invalidates_readers_and_late_response_cannot_restore_them() {
        let source = fixture();
        let mut probe = Probe::default();
        let cache = probe.cache();
        *source.close_during_request.borrow_mut() = Some(cache.clone());
        probe.poll(&source);
        for _ in 0..100 {
            probe.poll(&source);
        }
        assert_eq!(source.requests.get(), 1);
        assert_eq!(
            cache.read(source.epoch().as_ref()).survives_logout,
            Survival::Unknown
        );
    }
}
