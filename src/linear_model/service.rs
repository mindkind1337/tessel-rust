//! Private asynchronous account service. Never serialize this service or its actions.
//! Account changes advance the generation before validation and after durable success. All I/O runs
//! on the eventual dedicated worker, not in a renderer callback.
use super::{request::Scope, *};
use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::watch;
type FutureResult<'a> = Pin<Box<dyn Future<Output = Result<Reply, String>> + Send + 'a>>;
const LIMIT: usize = 128;
const CANCELLED: &str = "Linear connection changed";
pub(super) struct Credentials {
    pub key: Zeroizing<String>,
    pub identity: LinearIdentity,
}
pub(super) trait Store: Send + Sync {
    fn encryption(&self) -> Result<(), String>;
    fn load(&self) -> Result<Option<Credentials>, String>;
    fn save(&self, value: &Credentials) -> Result<(), String>;
    fn remove(&self) -> Result<(), String>;
}
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Capability(bool),
    Status(Option<LinearIdentity>),
    Identity(LinearIdentity),
    Teams(Page<LinearTeam>),
    States(Page<LinearState>),
    Issues(Page<LinearIssue>),
    Updated(Box<LinearIssue>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    Teams,
    States(String),
    Issues(IssueSelection),
}
#[derive(Clone)]
pub(super) enum Remote {
    Identity,
    Read { query: Query, viewer: String },
    SetState { issue: String, state: String },
}
pub(super) trait Transport: Send + Sync {
    fn request<'a>(&'a self, key: &'a str, request: Remote) -> FutureResult<'a>;
}
struct Http;
impl Transport for Http {
    fn request<'a>(&'a self, key: &'a str, request: Remote) -> FutureResult<'a> {
        Box::pin(async move {
            match request {
                Remote::Identity => validate_linear_key(key).await.map(Reply::Identity),
                Remote::Read {
                    query: Query::Teams,
                    ..
                } => fetch_linear_teams(key).await.map(Reply::Teams),
                Remote::Read {
                    query: Query::States(id),
                    ..
                } => fetch_linear_states(key, &id).await.map(Reply::States),
                Remote::Read {
                    query: Query::Issues(selection),
                    viewer,
                } => fetch_linear_issues(key, &viewer, &selection)
                    .await
                    .map(Reply::Issues),
                Remote::SetState { issue, state } => update_linear_issue_state(key, &issue, &state)
                    .await
                    .map(|issue| Reply::Updated(Box::new(issue))),
            }
        })
    }
}
pub enum Action {
    Capability,
    Status,
    Connect(Zeroizing<String>),
    Disconnect,
    Read { query: Query, refresh: bool },
    SetState { issue: String, state: String },
}
struct CacheEntry {
    scope: Scope,
    query: Query,
    revision: u64,
    at: Instant,
    reply: Reply,
}
struct Flight {
    id: u64,
    scope: Scope,
    query: Query,
    revision: u64,
    receiver: watch::Receiver<Option<Result<Reply, String>>>,
}
#[derive(Default)]
struct State {
    loaded: bool,
    account: Option<Arc<Credentials>>,
    revision: u64,
    sequence: u64,
    cache: VecDeque<CacheEntry>,
    flights: Vec<Flight>,
}

struct FlightCleanup {
    state: Arc<Mutex<State>>,
    id: u64,
}
impl Drop for FlightCleanup {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.flights.retain(|flight| flight.id != self.id);
        }
    }
}

/// Result payload is public data; private guards are never serialized or logged.
/// Applying it rechecks both account and issue revision.
pub struct Delivery {
    scope: Scope,
    state: Arc<Mutex<State>>,
    revision: Option<u64>,
    result: Result<Reply, String>,
}
/// Captured from a delivered issue page, never from a renderer snapshot.
/// This is transient account/revision authority, not serialized data.
#[derive(Clone)]
pub struct IssuePermit {
    scope: Scope,
    state: Arc<Mutex<State>>,
    revision: u64,
    issues: Arc<Vec<String>>,
}
impl IssuePermit {
    #[cfg(test)]
    pub(crate) fn fixture(issue: &str) -> (super::request::Generation, Self) {
        let owner = super::request::Generation::default();
        let state = State {
            loaded: true,
            account: Some(Arc::new(Credentials {
                key: Zeroizing::new("synthetic-only".into()),
                identity: LinearIdentity {
                    viewer: Default::default(),
                    organization: Default::default(),
                },
            })),
            ..Default::default()
        };
        let delivery = Delivery {
            scope: owner.scope(),
            state: Arc::new(Mutex::new(state)),
            revision: Some(0),
            result: Ok(Reply::Issues(Page {
                items: vec![LinearIssue {
                    id: issue.into(),
                    ..Default::default()
                }],
                has_next_page: false,
                end_cursor: None,
            })),
        };
        let permit = delivery
            .apply_with_permit(|_, permit| permit.unwrap())
            .unwrap();
        (owner, permit)
    }
    pub(super) fn captured_scope(&self) -> Scope {
        self.scope.clone()
    }
    /// Synchronous local action only. Do not re-enter Linear from the callback.
    pub fn commit<T>(
        &self,
        issue: &str,
        apply: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        if !self.issues.iter().any(|id| id == issue) {
            return Err("Issue is not in the captured Linear page".into());
        }
        self.scope.commit(|| {
            let state = self.state.lock().map_err(|_| CANCELLED)?;
            if state.revision != self.revision || state.account.is_none() {
                return Err("Linear issues changed; refresh".into());
            }
            apply()
        })
    }
}
impl Delivery {
    /// Callback is synchronous and must not re-enter this service or its authority.
    pub fn apply<T>(self, apply: impl FnOnce(Result<Reply, String>) -> T) -> Result<T, String> {
        self.apply_with_permit(|reply, _| apply(reply))
    }
    /// Store the permit during delivery; use it only after this callback returns.
    pub fn apply_with_permit<T>(
        self,
        apply: impl FnOnce(Result<Reply, String>, Option<IssuePermit>) -> T,
    ) -> Result<T, String> {
        self.scope.commit(|| {
            let state = self.state.lock().map_err(|_| CANCELLED)?;
            if self
                .revision
                .is_some_and(|revision| revision != state.revision)
            {
                return Err("Linear issues changed; refresh".into());
            }
            let permit = match (&self.result, self.revision) {
                (Ok(Reply::Issues(page)), Some(revision)) if state.account.is_some() => {
                    Some(IssuePermit {
                        scope: self.scope.clone(),
                        state: self.state.clone(),
                        revision,
                        issues: Arc::new(page.items.iter().map(|issue| issue.id.clone()).collect()),
                    })
                }
                _ => None,
            };
            Ok(apply(self.result, permit))
        })
    }
}
#[derive(Clone)]
pub struct Service {
    owner: Scope,
    state: Arc<Mutex<State>>,
    store: Arc<dyn Store>,
    transport: Arc<dyn Transport>,
}
impl Service {
    pub fn new(owner: Scope) -> Result<Self, String> {
        Ok(Self::new_at(
            owner,
            linear_storage_path().ok_or("No private data directory")?,
        ))
    }
    pub fn new_at(owner: Scope, path: PathBuf) -> Self {
        Self::with_backends(
            owner,
            Arc::new(super::record::DiskStore { path }),
            Arc::new(Http),
        )
    }
    pub(super) fn with_backends(
        owner: Scope,
        store: Arc<dyn Store>,
        transport: Arc<dyn Transport>,
    ) -> Self {
        Self {
            owner,
            state: Arc::new(Mutex::new(State::default())),
            store,
            transport,
        }
    }
    fn account(&self, scope: &Scope) -> Result<Option<Arc<Credentials>>, String> {
        scope.commit(|| {
            let mut state = self.state.lock().map_err(|_| CANCELLED)?;
            if !state.loaded {
                state.account = self.store.load()?.map(Arc::new);
                state.loaded = true;
            }
            Ok(state.account.clone())
        })
    }
    pub(super) fn current_scope(&self) -> Scope {
        self.owner.latest()
    }
    /// Claim an update against the account/revision that supplied the issue.
    /// Advancing revision here prevents replay even while network work is pending.
    pub(super) fn prepare_issue_update(
        &self,
        permit: &IssuePermit,
        issue: &str,
        target: &str,
    ) -> Result<Scope, String> {
        checked_id(issue)?;
        checked_id(target)?;
        if !permit.issues.iter().any(|id| id == issue) {
            return Err("Issue is not in the captured Linear page".into());
        }
        if !self.accepts_owner(&permit.scope) || !Arc::ptr_eq(&self.state, &permit.state) {
            return Err("Wrong Linear account owner".into());
        }
        permit.scope.commit(|| {
            let mut state = self.state.lock().map_err(|_| CANCELLED)?;
            if state.revision != permit.revision || state.account.is_none() {
                return Err("Linear issues changed; refresh".into());
            }
            state.revision = state
                .revision
                .checked_add(1)
                .ok_or("Linear issue revision exhausted")?;
            state
                .cache
                .retain(|entry| !matches!(entry.query, Query::Issues(_)));
            Ok(permit.scope.clone())
        })
    }
    pub(super) fn accepts_owner(&self, scope: &Scope) -> bool {
        self.owner.same_owner(scope)
    }
    pub(super) fn prepare(&self, scope: &Scope, action: &Action) -> Result<Scope, String> {
        if !self.owner.same_owner(scope) {
            return Err("Wrong Linear account owner".into());
        }
        if !scope.is_current() {
            return Err(CANCELLED.into());
        }
        if let Action::Connect(key) = action {
            valid_key(key)?;
            self.store.encryption()?;
        }
        if matches!(action, Action::Connect(_) | Action::Disconnect) {
            scope.commit_and_advance(|| Ok(())).map(|(_, scope)| scope)
        } else {
            Ok(scope.clone())
        }
    }
    pub(super) fn failure(&self, scope: Scope, error: String) -> Delivery {
        Delivery {
            scope,
            state: self.state.clone(),
            revision: None,
            result: Err(error),
        }
    }
    pub async fn execute(&self, scope: Scope, action: Action) -> Delivery {
        match self.prepare(&scope, &action) {
            Ok(scope) => self.execute_prepared(scope, action).await,
            Err(error) => self.failure(scope, error),
        }
    }
    pub(super) async fn execute_prepared(&self, scope: Scope, action: Action) -> Delivery {
        let mut delivery_scope = scope.clone();
        let mut revision = None;
        let result = match action {
            Action::Capability => Ok(Reply::Capability(self.store.encryption().is_ok())),
            Action::Status => self
                .account(&scope)
                .map(|account| Reply::Status(account.map(|a| a.identity.clone()))),
            Action::Connect(key) => match self.connect(&scope, key).await {
                Ok((reply, current)) => {
                    delivery_scope = current;
                    Ok(reply)
                }
                Err(error) => Err(error),
            },
            Action::Disconnect => match self.disconnect(&scope) {
                Ok((reply, current)) => {
                    delivery_scope = current;
                    Ok(reply)
                }
                Err(error) => Err(error),
            },
            Action::Read { query, refresh } => {
                let result = self.read(&scope, query, refresh).await;
                match result {
                    Ok((reply, rev)) => {
                        revision = rev;
                        Ok(reply)
                    }
                    Err(error) => Err(error),
                }
            }
            Action::SetState { issue, state } => self.set_state(&scope, issue, state).await,
        };
        Delivery {
            scope: delivery_scope,
            state: self.state.clone(),
            revision,
            result,
        }
    }
    async fn connect(
        &self,
        scope: &Scope,
        key: Zeroizing<String>,
    ) -> Result<(Reply, Scope), String> {
        let key = Zeroizing::new(valid_key(&key)?.to_owned());
        let mut request = scope.clone();
        let Reply::Identity(identity) = request
            .run(self.transport.request(&key, Remote::Identity))
            .await?
        else {
            return Err("Invalid Linear identity response".into());
        };
        let identity = super::record::validated_identity(&identity)?;
        let account = Arc::new(Credentials { key, identity });
        scope.commit_and_advance(|| {
            let mut state = self.state.lock().map_err(|_| CANCELLED)?;
            self.store.save(&account)?;
            state.account = Some(account.clone());
            state.loaded = true;
            state.cache.clear();
            state.flights.clear();
            Ok(Reply::Status(Some(account.identity.clone())))
        })
    }
    fn disconnect(&self, scope: &Scope) -> Result<(Reply, Scope), String> {
        scope.commit_and_advance(|| {
            let mut state = self.state.lock().map_err(|_| CANCELLED)?;
            self.store.remove()?;
            state.account = None;
            state.loaded = true;
            state.cache.clear();
            state.flights.clear();
            Ok(Reply::Status(None))
        })
    }
    async fn read(
        &self,
        scope: &Scope,
        query: Query,
        refresh: bool,
    ) -> Result<(Reply, Option<u64>), String> {
        let account = self.account(scope)?.ok_or("Connect Linear first")?;
        let query = match query {
            Query::Issues(mut selection) => {
                selection.team_id = selection.team_id.filter(|id| !id.is_empty());
                selection.state_id = selection.state_id.filter(|id| !id.is_empty());
                Query::Issues(selection)
            }
            other => other,
        };
        match &query {
            Query::Teams => (),
            Query::States(id) => checked_id(id)?,
            Query::Issues(selection) => {
                issue_filter(&account.identity.viewer.id, selection)?;
            }
        }
        enum Plan {
            Hit(Box<Reply>),
            Wait(watch::Receiver<Option<Result<Reply, String>>>),
            Start(u64, watch::Sender<Option<Result<Reply, String>>>),
        }
        let (plan, revision) = scope.commit(|| {
            let mut state = self.state.lock().map_err(|_| CANCELLED)?;
            let revision = if matches!(query, Query::Issues(_)) {
                state.revision
            } else {
                0
            };
            state.cache.retain(|entry| entry.scope.is_current());
            state.flights.retain(|entry| entry.scope.is_current());
            let ttl = Duration::from_secs(if matches!(query, Query::Issues(_)) {
                60
            } else {
                600
            });
            if !refresh
                && let Some(entry) = state.cache.iter().find(|entry| {
                    entry.scope.same_generation(scope)
                        && entry.query == query
                        && entry.revision == revision
                        && entry.at.elapsed() < ttl
                })
            {
                return Ok((Plan::Hit(Box::new(entry.reply.clone())), revision));
            }
            if let Some(flight) = state.flights.iter().find(|flight| {
                flight.scope.same_generation(scope)
                    && flight.query == query
                    && flight.revision == revision
            }) {
                return Ok((Plan::Wait(flight.receiver.clone()), revision));
            }
            if state.flights.len() >= LIMIT {
                return Err("Too many pending Linear requests".into());
            }
            state.sequence = state
                .sequence
                .checked_add(1)
                .ok_or("Linear request identity exhausted")?;
            let id = state.sequence;
            let (sender, receiver) = watch::channel(None);
            state.flights.push(Flight {
                id,
                scope: scope.clone(),
                query: query.clone(),
                revision,
                receiver,
            });
            Ok((Plan::Start(id, sender), revision))
        })?;
        let issue_revision = matches!(query, Query::Issues(_)).then_some(revision);
        let reply = match plan {
            Plan::Hit(reply) => *reply,
            Plan::Wait(mut receiver) => {
                let mut request = scope.clone();
                request
                    .run(async {
                        loop {
                            if let Some(result) = receiver.borrow_and_update().clone() {
                                return result;
                            }
                            receiver
                                .changed()
                                .await
                                .map_err(|_| "Linear request cancelled")?;
                        }
                    })
                    .await?
            }
            Plan::Start(id, sender) => {
                let _cleanup = FlightCleanup {
                    state: self.state.clone(),
                    id,
                };
                let mut request = scope.clone();
                let result = request
                    .run(self.transport.request(
                        &account.key,
                        Remote::Read {
                            query: query.clone(),
                            viewer: account.identity.viewer.id.clone(),
                        },
                    ))
                    .await;
                let result = scope.commit(|| {
                    let mut state = self.state.lock().map_err(|_| CANCELLED)?;
                    if issue_revision.is_some_and(|revision| revision != state.revision) {
                        return Err("Linear issues changed; refresh".into());
                    }
                    let reply = result?;
                    state.cache.retain(|entry| {
                        !(entry.scope.same_generation(scope)
                            && entry.query == query
                            && entry.revision == revision)
                    });
                    state.cache.push_back(CacheEntry {
                        scope: scope.clone(),
                        query,
                        revision,
                        at: Instant::now(),
                        reply: reply.clone(),
                    });
                    while state.cache.len() > LIMIT {
                        state.cache.pop_front();
                    }
                    Ok(reply)
                });
                if let Ok(mut state) = self.state.lock() {
                    state.flights.retain(|flight| flight.id != id);
                }
                sender.send_replace(Some(result.clone()));
                result?
            }
        };
        Ok((reply, issue_revision))
    }
    async fn set_state(
        &self,
        scope: &Scope,
        issue: String,
        state: String,
    ) -> Result<Reply, String> {
        checked_id(&issue)?;
        checked_id(&state)?;
        let account = self.account(scope)?.ok_or("Connect Linear first")?;
        self.invalidate_issues(scope)?;
        let mut request = scope.clone();
        let reply = request
            .run(
                self.transport
                    .request(&account.key, Remote::SetState { issue, state }),
            )
            .await?;
        if !matches!(reply, Reply::Updated(_)) {
            return Err("Invalid Linear update response".into());
        }
        self.invalidate_issues(scope)?;
        Ok(reply)
    }
    fn invalidate_issues(&self, scope: &Scope) -> Result<(), String> {
        scope.commit(|| {
            let mut state = self.state.lock().map_err(|_| CANCELLED)?;
            state.revision = state
                .revision
                .checked_add(1)
                .ok_or("Linear issue revision exhausted")?;
            state
                .cache
                .retain(|entry| !matches!(entry.query, Query::Issues(_)));
            Ok(())
        })
    }
}
#[cfg(test)]
mod tests;
