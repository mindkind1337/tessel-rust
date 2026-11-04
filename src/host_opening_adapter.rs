//! Production bindings for the opening state machine. Methods run on its worker.
use crate::host_lease_store::{CreateToken, LeaseSlot, LeaseStore};
use crate::terminal::host_opening::{
    Events, Host, HostError, LocalError, Slot, Spec, Store, Token,
};
use std::time::{Duration, Instant};
use tessel_host_client::{
    Client, ClientError,
    terminal::{Terminal, close_terminal},
};
use tessel_host_protocol::{DedupKey, Epoch, LeaseKey, TerminalId};

#[derive(Clone)]
pub struct HostConnection {
    pub client: Client,
    timeout: Duration,
    info: crate::native_host_info::Cache,
}
impl HostConnection {
    /// Pass the request timeout actually used to construct this client.
    pub fn new(client: Client, timeout: Duration) -> Self {
        Self {
            client,
            timeout,
            info: Default::default(),
        }
    }
    pub fn with_info(mut self, info: crate::native_host_info::Cache) -> Self {
        self.info = info;
        self
    }
    pub fn persistence(&self) -> tessel_host_protocol::Persistence {
        let epoch = self.client.link().ok().and_then(|_| self.client.epoch());
        self.info.read(epoch.as_ref())
    }
}
fn host_error(error: ClientError) -> HostError {
    match error {
        ClientError::EpochChanged => HostError::EpochChanged,
        ClientError::Refused(_) => HostError::Refused,
        _ => HostError::Ambiguous,
    }
}
impl Host for HostConnection {
    type Terminal = Terminal;
    fn epoch(&self) -> Option<Epoch> {
        self.client.epoch()
    }
    fn request_timeout(&self) -> Duration {
        self.timeout
    }
    fn create_keyed(
        &self,
        key: DedupKey,
        epoch: &Epoch,
        spec: &Spec,
    ) -> Result<(Terminal, Events, LeaseKey), HostError> {
        let created = Terminal::create_keyed(
            &self.client,
            key,
            epoch,
            spec.argv.clone(),
            spec.cwd.clone(),
            spec.env.clone(),
            spec.size,
        )
        .map_err(host_error)?;
        Ok((created.terminal, created.events, created.lease))
    }
    fn attach_fresh(
        &self,
        terminal: &TerminalId,
        epoch: &Epoch,
        lease: Option<LeaseKey>,
    ) -> Result<(Terminal, Events), HostError> {
        Terminal::attach_fresh(&self.client, terminal.clone(), epoch.clone(), lease)
            .map_err(host_error)
    }
    fn close_known(
        &self,
        epoch: &Epoch,
        terminal: &TerminalId,
        lease: &LeaseKey,
    ) -> Result<(), HostError> {
        close_terminal(&self.client, epoch, terminal, lease).map_err(host_error)
    }
    fn terminal_id(terminal: &Terminal) -> TerminalId {
        terminal.id().clone()
    }
    fn lease(terminal: &Terminal) -> Option<LeaseKey> {
        terminal.lease()
    }
}

/// No filesystem operation in construction. The opening worker resolves and
/// opens the owner's encrypted store for every transaction, independent of a
/// project or imported workspace. No configurable path can redirect authority.
pub struct OwnerStore;
impl crate::terminal::host_lease_keeper::LeaseWriter for OwnerStore {
    fn complete(&self, pending: &str, value: &Slot, lease: &LeaseKey) -> Result<(), LocalError> {
        Store::complete(self, pending, value, lease)
    }
    fn put(&self, value: &Slot, lease: &LeaseKey) -> Result<(), LocalError> {
        Store::put(self, value, lease)
    }
    fn remove(&self, value: &Slot) -> Result<(), LocalError> {
        Store::remove(self, value)
    }
}
fn slot(value: &Slot) -> LeaseSlot {
    LeaseSlot {
        alias: value.alias.clone(),
        epoch: value.epoch.clone(),
        terminal: value.terminal.clone(),
    }
}
impl Store for OwnerStore {
    type Token = CreateToken;
    fn reserve(&self, alias: &str, epoch: &Epoch) -> Result<CreateToken, LocalError> {
        LeaseStore::open_default()
            .and_then(|s| s.reserve_pending(alias, epoch))
            .map_err(|_| LocalError)
    }
    fn complete(&self, pending: &str, value: &Slot, lease: &LeaseKey) -> Result<(), LocalError> {
        LeaseStore::open_default()
            .and_then(|s| s.complete_pending(pending, &slot(value), lease))
            .map_err(|_| LocalError)
    }
    fn forget_pending(&self, pending: &str) -> Result<(), LocalError> {
        LeaseStore::open_default()
            .and_then(|s| s.remove_pending_confirmed(pending))
            .map_err(|_| LocalError)
    }
    fn remove(&self, value: &Slot) -> Result<(), LocalError> {
        LeaseStore::open_default()
            .and_then(|s| s.remove(&slot(value)))
            .map_err(|_| LocalError)
    }
    fn get(&self, value: &Slot) -> Result<Option<LeaseKey>, LocalError> {
        LeaseStore::open_default()
            .and_then(|s| s.get(&slot(value)))
            .map_err(|_| LocalError)
    }
    fn put(&self, value: &Slot, lease: &LeaseKey) -> Result<(), LocalError> {
        LeaseStore::open_default()
            .and_then(|s| s.put(&slot(value), lease))
            .map_err(|_| LocalError)
    }
}
impl Token for CreateToken {
    fn id(&self) -> &str {
        &self.info().id
    }
    fn key(
        &mut self,
        epoch: &Epoch,
        now: Instant,
        timeout: Duration,
    ) -> Result<DedupKey, LocalError> {
        self.key_for_attempt(epoch, now, timeout)
            .map_err(|_| LocalError)
    }
}

impl crate::terminal::host_terminal_list::Lister for HostConnection {
    fn epoch(&self) -> Option<Epoch> {
        self.client.epoch()
    }
    fn list(
        &self,
    ) -> Result<tessel_host_protocol::pty::Listed, crate::terminal::host_terminal_list::ListError>
    {
        use crate::terminal::host_terminal_list::ListError;
        self.client
            .ask(
                tessel_host_protocol::pty::kind::LIST,
                &serde_json::Value::Null,
            )
            .map_err(|error| match error {
                ClientError::ConnectionLost => ListError::NotConnected,
                ClientError::Timeout => ListError::TimedOut,
                ClientError::Refused(ref refusal)
                    if refusal.code == tessel_host_protocol::ErrorCode::UnsupportedKind =>
                {
                    ListError::Unsupported
                }
                _ => ListError::RequestFailed,
            })
    }
}

impl crate::terminal::host_terminal_list::SavedIndex for OwnerStore {
    fn index(
        &self,
        alias: &str,
        epoch: &Epoch,
    ) -> Result<
        crate::terminal::host_terminal_list::Index,
        crate::terminal::host_terminal_list::IndexError,
    > {
        use crate::host_lease_store::LeaseStoreError;
        use crate::terminal::host_terminal_list::{Index, IndexError, PendingMeta};
        LeaseStore::open_default()
            .and_then(|store| store.index(alias, epoch))
            .map(|metadata| Index {
                saved: metadata.saved.into_iter().collect(),
                pending: metadata
                    .pending
                    .into_iter()
                    .map(|entry| PendingMeta {
                        id: entry.id,
                        epoch: entry.epoch,
                    })
                    .collect(),
            })
            .map_err(|error| match error {
                LeaseStoreError::Locked => IndexError::Locked,
                LeaseStoreError::Corrupt => IndexError::Corrupt,
                _ => IndexError::Unavailable,
            })
    }
}
