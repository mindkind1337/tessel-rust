//! Background connection ownership. No network operation runs on an egui frame.
use crate::native_host_install::{self, Bundle, HostInstall, InstallError};
use crate::native_host_transport::{SshTransport, connector};
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{self, Receiver, TryRecvError},
};
use tessel_host_client::{Client, ConnectionState, Opening, Options};

const MAX_HOSTS: usize = 8;

/// Owning handle: dropping even an undelivered result stops its supervisor.
pub struct Connection {
    client: Client,
    pub installation: HostInstall,
    close: Option<mpsc::Sender<()>>,
}
impl Connection {
    pub fn client(&self) -> &Client {
        &self.client
    }
    pub fn state(&self) -> ConnectionState {
        self.client.state()
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(close) = self.close.take() {
            let _ = close.send(());
        } else {
            // Only a not-yet-delivered result on the connection worker.
            self.client.close();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    InvalidHost,
    Capacity,
    Cancelled,
    Worker,
    Random,
    Install(InstallError),
    Connect,
}

struct Pending {
    opening: Arc<Opening>,
    cancel: Arc<AtomicBool>,
    reply: Receiver<Result<Connection, Failure>>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.opening.cancel();
    }
}
enum Entry {
    Pending(Pending),
    Ready(Connection),
    Failed(Failure),
}

/// One connection per exact validated OpenSSH alias. A cancelled job retains its
/// slot until it exits, so repeated clicks cannot create unbounded worker threads.
#[derive(Default)]
pub struct Manager {
    entries: HashMap<String, Entry>,
    workers: Arc<AtomicUsize>,
}

struct WorkerSlot(Arc<AtomicUsize>);
impl WorkerSlot {
    fn acquire(workers: &Arc<AtomicUsize>) -> Result<Self, Failure> {
        workers
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_HOSTS).then_some(count + 1)
            })
            .map_err(|_| Failure::Capacity)?;
        Ok(Self(workers.clone()))
    }
}
impl Drop for WorkerSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
impl Manager {
    pub fn aliases(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
    /// Explicit user action only: may install the bundled daemon on this host.
    pub fn start(&mut self, alias: &str) -> Result<(), Failure> {
        crate::workspace::validate_host(alias).map_err(|_| Failure::InvalidHost)?;
        self.poll();
        if self
            .entries
            .get(alias)
            .is_some_and(|entry| !matches!(entry, Entry::Failed(_)))
        {
            return Ok(());
        }
        if self.entries.len() >= MAX_HOSTS && !self.entries.contains_key(alias) {
            return Err(Failure::Capacity);
        }
        let slot = WorkerSlot::acquire(&self.workers)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let opening = Arc::new(Opening::default());
        let worker_opening = opening.clone();
        let host = alias.to_owned();
        let (sender, reply) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("host-connect".into())
            .spawn(move || {
                let _slot = slot;
                let result = connect(&host, worker_cancel, worker_opening);
                match result {
                    Ok(mut connection) => {
                        let client = connection.client.clone();
                        let (close, closing) = mpsc::channel();
                        connection.close = Some(close);
                        // SendError drops its payload, signalling this worker too.
                        let _ = sender.send(Ok(connection));
                        let _ = closing.recv();
                        client.close();
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error));
                    }
                }
            })
            .map_err(|_| Failure::Worker)?;
        self.entries.insert(
            alias.to_owned(),
            Entry::Pending(Pending {
                cancel,
                reply,
                opening,
            }),
        );
        Ok(())
    }
    pub fn poll(&mut self) {
        for entry in self.entries.values_mut() {
            let Entry::Pending(pending) = entry else {
                continue;
            };
            let result = match pending.reply.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => continue,
                Err(TryRecvError::Disconnected) => Err(Failure::Worker),
            };
            *entry = if pending.cancel.load(Ordering::Acquire) {
                drop(result);
                Entry::Failed(Failure::Cancelled)
            } else {
                match result {
                    Ok(connection) => Entry::Ready(connection),
                    Err(error) => Entry::Failed(error),
                }
            };
        }
    }
    pub fn connection(&self, alias: &str) -> Option<&Connection> {
        match self.entries.get(alias) {
            Some(Entry::Ready(connection)) => Some(connection),
            _ => None,
        }
    }
    pub fn failure(&self, alias: &str) -> Option<Failure> {
        match self.entries.get(alias) {
            Some(Entry::Failed(error)) => Some(*error),
            _ => None,
        }
    }
    pub fn pending(&self, alias: &str) -> bool {
        matches!(self.entries.get(alias), Some(Entry::Pending(_)))
    }
    pub fn cancelling(&self, alias: &str) -> bool {
        matches!(self.entries.get(alias), Some(Entry::Pending(pending)) if pending.cancel.load(Ordering::Acquire))
    }
    pub fn cancel(&mut self, alias: &str) {
        if let Some(Entry::Pending(pending)) = self.entries.get(alias) {
            pending.cancel.store(true, Ordering::Release);
            pending.opening.cancel();
        } else {
            self.entries.remove(alias);
        }
    }
}

fn connect(
    alias: &str,
    cancel: Arc<AtomicBool>,
    opening: Arc<Opening>,
) -> Result<Connection, Failure> {
    let cancelled = || {
        if cancel.load(Ordering::Acquire) {
            Err(Failure::Cancelled)
        } else {
            Ok(())
        }
    };
    cancelled()?;
    let bundle = Bundle::embedded().map_err(Failure::Install)?;
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).map_err(|_| Failure::Random)?;
    let mut transport =
        SshTransport::new(alias.to_owned(), cancel.clone()).map_err(|_| Failure::Connect)?;
    let installation = native_host_install::install(
        &mut transport,
        &bundle,
        None,
        &native_host_install::nonce_hex(random),
    )
    .map_err(Failure::Install)?;
    cancelled()?;
    let connector = connector(alias, &installation.connect).map_err(|_| Failure::Connect)?;
    let client = Client::connect_cancellable(connector, Options::default(), opening)
        .map_err(|_| Failure::Connect)?;
    let connection = Connection {
        client,
        installation,
        close: None,
    };
    cancelled()?;
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(
        manager: &mut Manager,
        alias: &str,
    ) -> (
        Arc<AtomicBool>,
        mpsc::SyncSender<Result<Connection, Failure>>,
    ) {
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, reply) = mpsc::sync_channel(1);
        manager.entries.insert(
            alias.into(),
            Entry::Pending(Pending {
                cancel: cancel.clone(),
                opening: Arc::new(Opening::default()),
                reply,
            }),
        );
        (cancel, sender)
    }

    #[test]
    fn cancellation_retains_slot_and_rejects_late_results() {
        let mut manager = Manager::default();
        let (cancel, sender) = pending(&mut manager, "fixture");
        manager.cancel("fixture");
        assert!(cancel.load(Ordering::Acquire));
        assert!(manager.pending("fixture"));
        assert_eq!(manager.start("fixture"), Ok(())); // no replacement worker
        assert!(sender.send(Err(Failure::Connect)).is_ok());
        manager.poll();
        assert_eq!(manager.failure("fixture"), Some(Failure::Cancelled));
        assert!(!manager.pending("fixture"));
        manager.cancel("fixture");
        assert!(manager.entries.is_empty());
    }

    #[test]
    fn abandoned_workers_and_owner_drop_are_explicit() {
        let mut manager = Manager::default();
        let (_, sender) = pending(&mut manager, "failed");
        drop(sender);
        manager.poll();
        assert_eq!(manager.failure("failed"), Some(Failure::Worker));
        let (cancel, sender) = pending(&mut manager, "running");
        drop(manager);
        assert!(cancel.load(Ordering::Acquire));
        assert!(sender.send(Err(Failure::Connect)).is_err());
    }

    #[test]
    fn capacity_and_invalid_alias_never_spawn_a_worker() {
        let mut manager = Manager::default();
        assert_eq!(
            manager.start("-oProxyCommand=bad"),
            Err(Failure::InvalidHost)
        );
        let mut senders = Vec::new();
        for n in 0..MAX_HOSTS {
            senders.push(pending(&mut manager, &format!("fixture{n}")).1);
        }
        assert_eq!(manager.start("extra"), Err(Failure::Capacity));
        assert_eq!(manager.entries.len(), MAX_HOSTS);
        manager.cancel("fixture0");
        assert_eq!(manager.start("extra"), Err(Failure::Capacity));
    }

    #[test]
    fn workers_closing_after_ui_removal_still_consume_capacity() {
        let mut manager = Manager::default();
        let mut slots: Vec<_> = (0..MAX_HOSTS)
            .map(|_| WorkerSlot::acquire(&manager.workers).unwrap())
            .collect();
        assert!(manager.entries.is_empty());
        assert_eq!(manager.start("fixture"), Err(Failure::Capacity));
        slots.pop();
        let slot = WorkerSlot::acquire(&manager.workers).unwrap();
        assert_eq!(manager.workers.load(Ordering::Acquire), MAX_HOSTS);
        drop(slot);
        drop(slots);
        assert_eq!(manager.workers.load(Ordering::Acquire), 0);
    }
}
