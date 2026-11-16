use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::oneshot;
fn identity(id: &str) -> LinearIdentity {
    LinearIdentity {
        viewer: LinearPerson {
            id: id.into(),
            name: "Fixture".into(),
            display_name: Some("Fixture".into()),
            email: Some("fixture@example.test".into()),
        },
        organization: LinearOrganization {
            id: "org-1".into(),
            name: "Test".into(),
            url_key: "test".into(),
        },
    }
}
fn credentials(id: &str) -> Credentials {
    Credentials {
        key: Zeroizing::new(format!("synthetic-key-{id}")),
        identity: identity(id),
    }
}
fn copy(value: &Credentials) -> Credentials {
    Credentials {
        key: Zeroizing::new(value.key.to_string()),
        identity: value.identity.clone(),
    }
}
struct MemoryStore {
    value: Mutex<Option<Credentials>>,
    fail: AtomicBool,
    unavailable: AtomicBool,
    saves: AtomicUsize,
}
impl MemoryStore {
    fn new() -> Self {
        Self {
            value: Mutex::new(Some(credentials("old"))),
            fail: AtomicBool::new(false),
            unavailable: AtomicBool::new(false),
            saves: AtomicUsize::new(0),
        }
    }
}
impl Store for MemoryStore {
    fn encryption(&self) -> Result<(), String> {
        if self.unavailable.load(Ordering::SeqCst) {
            Err("synthetic unavailable encryption".into())
        } else {
            Ok(())
        }
    }
    fn load(&self) -> Result<Option<Credentials>, String> {
        Ok(self.value.lock().unwrap().as_ref().map(copy))
    }
    fn save(&self, value: &Credentials) -> Result<(), String> {
        if self.fail.load(Ordering::SeqCst) {
            return Err("synthetic save failure".into());
        }
        self.saves.fetch_add(1, Ordering::SeqCst);
        *self.value.lock().unwrap() = Some(copy(value));
        Ok(())
    }
    fn remove(&self) -> Result<(), String> {
        if self.fail.load(Ordering::SeqCst) {
            return Err("synthetic remove failure".into());
        }
        *self.value.lock().unwrap() = None;
        Ok(())
    }
}
struct Step {
    reply: Reply,
    started: Option<oneshot::Sender<()>>,
    wait: Option<oneshot::Receiver<()>>,
}
impl Step {
    fn ready(reply: Reply) -> Self {
        Self {
            reply,
            started: None,
            wait: None,
        }
    }
}
struct Fake {
    steps: Mutex<VecDeque<Step>>,
    calls: AtomicUsize,
}
impl Fake {
    fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: Mutex::new(steps.into()),
            calls: AtomicUsize::new(0),
        }
    }
}
impl Transport for Fake {
    fn request<'a>(&'a self, _: &'a str, _: Remote) -> FutureResult<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let step = self
                .steps
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected network request");
            if let Some(started) = step.started {
                let _ = started.send(());
            }
            if let Some(wait) = step.wait {
                wait.await.map_err(|_| "synthetic cancelled")?;
            }
            Ok(step.reply)
        })
    }
}
fn pending(reply: Reply) -> (Step, oneshot::Receiver<()>, oneshot::Sender<()>) {
    let (started, entered) = oneshot::channel();
    let (release, wait) = oneshot::channel();
    (
        Step {
            reply,
            started: Some(started),
            wait: Some(wait),
        },
        entered,
        release,
    )
}
fn teams() -> Reply {
    Reply::Teams(Page {
        items: vec![],
        has_next_page: false,
        end_cursor: None,
    })
}
fn issues() -> Reply {
    Reply::Issues(Page {
        items: vec![],
        has_next_page: false,
        end_cursor: None,
    })
}
fn update() -> Reply {
    Reply::Updated(Box::new(LinearIssue {
        id: "issue-1".into(),
        ..Default::default()
    }))
}
fn read(query: Query) -> Action {
    Action::Read {
        query,
        refresh: false,
    }
}
fn delivered(delivery: Delivery) -> Result<Reply, String> {
    delivery.apply(|result| result).and_then(|result| result)
}
#[tokio::test]
async fn durable_failures_keep_previous_connection_and_success_revokes_old_delivery() {
    let owner = request::Generation::default();
    let store = Arc::new(MemoryStore::new());
    let transport = Arc::new(Fake::new(vec![
        Step::ready(Reply::Identity(identity("new"))),
        Step::ready(Reply::Identity(identity("new"))),
    ]));
    let service = Service::with_backends(owner.scope(), store.clone(), transport);
    assert_eq!(
        delivered(service.execute(owner.scope(), Action::Status).await).unwrap(),
        Reply::Status(Some(identity("old")))
    );
    store.fail.store(true, Ordering::SeqCst);
    assert!(
        delivered(
            service
                .execute(
                    owner.scope(),
                    Action::Connect(Zeroizing::new("synthetic-new-key".into()))
                )
                .await
        )
        .is_err()
    );
    assert_eq!(
        delivered(service.execute(owner.scope(), Action::Status).await).unwrap(),
        Reply::Status(Some(identity("old")))
    );
    assert!(delivered(service.execute(owner.scope(), Action::Disconnect).await).is_err());
    assert_eq!(
        store
            .value
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .identity
            .viewer
            .id,
        "old"
    );
    let old = service.execute(owner.scope(), Action::Status).await;
    store.fail.store(false, Ordering::SeqCst);
    assert_eq!(
        delivered(
            service
                .execute(
                    owner.scope(),
                    Action::Connect(Zeroizing::new("synthetic-new-key".into()))
                )
                .await
        )
        .unwrap(),
        Reply::Status(Some(identity("new")))
    );
    assert!(old.apply(|_| panic!("stale delivery")).is_err());
    assert_eq!(store.saves.load(Ordering::SeqCst), 1);
    assert_eq!(
        delivered(service.execute(owner.scope(), Action::Disconnect).await).unwrap(),
        Reply::Status(None)
    );
    assert!(store.value.lock().unwrap().is_none());
}
#[tokio::test]
async fn concurrent_reads_share_flight_and_cache_expires_at_reference_ttl() {
    let owner = request::Generation::default();
    let store = Arc::new(MemoryStore::new());
    let (step, entered, release) = pending(teams());
    let transport = Arc::new(Fake::new(vec![
        step,
        Step::ready(teams()),
        Step::ready(teams()),
    ]));
    let service = Service::with_backends(owner.scope(), store, transport.clone());
    let first = tokio::spawn({
        let service = service.clone();
        let scope = owner.scope();
        async move { service.execute(scope, read(Query::Teams)).await }
    });
    entered.await.unwrap();
    let second = tokio::spawn({
        let service = service.clone();
        let scope = owner.scope();
        async move {
            service
                .execute(
                    scope,
                    Action::Read {
                        query: Query::Teams,
                        refresh: true,
                    },
                )
                .await
        }
    });
    tokio::task::yield_now().await;
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    release.send(()).unwrap();
    assert_eq!(delivered(first.await.unwrap()).unwrap(), teams());
    assert_eq!(delivered(second.await.unwrap()).unwrap(), teams());
    assert_eq!(
        delivered(service.execute(owner.scope(), read(Query::Teams)).await).unwrap(),
        teams()
    );
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    service.state.lock().unwrap().cache.front_mut().unwrap().at =
        Instant::now() - Duration::from_secs(601);
    delivered(service.execute(owner.scope(), read(Query::Teams)).await).unwrap();
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
    delivered(
        service
            .execute(
                owner.scope(),
                Action::Read {
                    query: Query::Teams,
                    refresh: true,
                },
            )
            .await,
    )
    .unwrap();
    assert_eq!(transport.calls.load(Ordering::SeqCst), 3);
}
#[tokio::test]
async fn mutations_reject_inflight_and_queued_issue_lists() {
    let owner = request::Generation::default();
    let (step, entered, release) = pending(issues());
    let transport = Arc::new(Fake::new(vec![
        step,
        Step::ready(update()),
        Step::ready(issues()),
        Step::ready(update()),
    ]));
    let service = Service::with_backends(owner.scope(), Arc::new(MemoryStore::new()), transport);
    let running = tokio::spawn({
        let service = service.clone();
        let scope = owner.scope();
        async move {
            service
                .execute(scope, read(Query::Issues(IssueSelection::default())))
                .await
        }
    });
    entered.await.unwrap();
    delivered(
        service
            .execute(
                owner.scope(),
                Action::SetState {
                    issue: "issue-1".into(),
                    state: "state-1".into(),
                },
            )
            .await,
    )
    .unwrap();
    release.send(()).unwrap();
    assert!(delivered(running.await.unwrap()).is_err());
    let queued = service
        .execute(
            owner.scope(),
            read(Query::Issues(IssueSelection::default())),
        )
        .await;
    delivered(
        service
            .execute(
                owner.scope(),
                Action::SetState {
                    issue: "issue-1".into(),
                    state: "state-2".into(),
                },
            )
            .await,
    )
    .unwrap();
    assert!(queued.apply(|_| panic!("obsolete issue list")).is_err());
}
#[tokio::test]
async fn dropping_leader_removes_flight_so_next_read_can_retry() {
    let owner = request::Generation::default();
    let (step, entered, _release) = pending(teams());
    let transport = Arc::new(Fake::new(vec![step, Step::ready(teams())]));
    let service = Service::with_backends(
        owner.scope(),
        Arc::new(MemoryStore::new()),
        transport.clone(),
    );
    let running = tokio::spawn({
        let service = service.clone();
        let scope = owner.scope();
        async move { service.execute(scope, read(Query::Teams)).await }
    });
    entered.await.unwrap();
    running.abort();
    assert!(matches!(running.await, Err(error) if error.is_cancelled()));
    assert_eq!(
        delivered(service.execute(owner.scope(), read(Query::Teams)).await).unwrap(),
        teams()
    );
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn newer_connect_cancels_old_validation_and_foreign_owner_cannot_use_service() {
    let owner = request::Generation::default();
    let store = Arc::new(MemoryStore::new());
    let (step, entered, _release) = pending(Reply::Identity(identity("superseded")));
    let transport = Arc::new(Fake::new(vec![
        step,
        Step::ready(Reply::Identity(identity("latest"))),
    ]));
    let service = Service::with_backends(owner.scope(), store.clone(), transport.clone());
    let old = tokio::spawn({
        let service = service.clone();
        let scope = owner.scope();
        async move {
            service
                .execute(
                    scope,
                    Action::Connect(Zeroizing::new("synthetic-old-key".into())),
                )
                .await
        }
    });
    entered.await.unwrap();
    delivered(
        service
            .execute(
                owner.scope(),
                Action::Connect(Zeroizing::new("synthetic-new-key".into())),
            )
            .await,
    )
    .unwrap();
    assert!(delivered(old.await.unwrap()).is_err());
    assert_eq!(store.saves.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .value
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .identity
            .viewer
            .id,
        "latest"
    );
    let foreign = request::Generation::default();
    assert!(delivered(service.execute(foreign.scope(), Action::Disconnect).await).is_err());
    assert!(store.value.lock().unwrap().is_some());
}
#[tokio::test]
async fn cache_evicts_oldest_entry_at_128() {
    let owner = request::Generation::default();
    let page = Reply::States(Page {
        items: vec![],
        has_next_page: false,
        end_cursor: None,
    });
    let transport = Arc::new(Fake::new(
        (0..130).map(|_| Step::ready(page.clone())).collect(),
    ));
    let service = Service::with_backends(
        owner.scope(),
        Arc::new(MemoryStore::new()),
        transport.clone(),
    );
    for index in 0..129 {
        delivered(
            service
                .execute(owner.scope(), read(Query::States(format!("team-{index}"))))
                .await,
        )
        .unwrap();
    }
    delivered(
        service
            .execute(owner.scope(), read(Query::States("team-128".into())))
            .await,
    )
    .unwrap();
    assert_eq!(transport.calls.load(Ordering::SeqCst), 129);
    delivered(
        service
            .execute(owner.scope(), read(Query::States("team-0".into())))
            .await,
    )
    .unwrap();
    assert_eq!(transport.calls.load(Ordering::SeqCst), 130);
    assert_eq!(service.state.lock().unwrap().cache.len(), 128);
}
#[cfg(windows)]
#[tokio::test]
async fn encrypted_record_reopens_identity_without_network_and_never_exposes_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("linear.dat");
    let store = Arc::new(super::super::record::DiskStore { path: path.clone() });
    store.save(&credentials("saved")).unwrap();
    let bytes = std::fs::read(path).unwrap();
    assert!(!bytes.windows(13).any(|window| window == b"synthetic-key"));
    let owner = request::Generation::default();
    let transport = Arc::new(Fake::new(vec![]));
    let service = Service::with_backends(owner.scope(), store, transport.clone());
    let reply = delivered(service.execute(owner.scope(), Action::Status).await).unwrap();
    assert_eq!(reply, Reply::Status(Some(identity("saved"))));
    assert!(!format!("{reply:?}").contains("synthetic-key"));
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unavailable_encryption_never_sends_key_and_empty_filters_share_cache() {
    let owner = request::Generation::default();
    let store = Arc::new(MemoryStore::new());
    let transport = Arc::new(Fake::new(vec![Step::ready(issues())]));
    let service = Service::with_backends(owner.scope(), store.clone(), transport.clone());
    store.unavailable.store(true, Ordering::SeqCst);
    let prior = owner.scope();
    assert!(
        delivered(
            service
                .execute(
                    owner.scope(),
                    Action::Connect(Zeroizing::new("synthetic-valid-key".into()))
                )
                .await
        )
        .is_err()
    );
    assert!(prior.is_current());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    delivered(
        service
            .execute(
                owner.scope(),
                read(Query::Issues(IssueSelection::default())),
            )
            .await,
    )
    .unwrap();
    let selection = IssueSelection {
        team_id: Some(String::new()),
        state_id: Some(String::new()),
        ..Default::default()
    };
    delivered(
        service
            .execute(owner.scope(), read(Query::Issues(selection)))
            .await,
    )
    .unwrap();
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
}
