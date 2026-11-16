//! Owner-local dialog controller. Neither its key draft nor worker is snapshot/Debug data.
use super::{
    service::{Action, IssuePermit, Query, Reply},
    worker::Worker,
    *,
};

#[derive(Clone, Debug)]
pub struct AgentChoice {
    pub id: String,
    pub label: String,
    pub available: bool,
}
struct Launch {
    key: u64,
    issue: String,
    permit: IssuePermit,
    next_state: Option<String>,
    accepted: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub open: bool,
    pub epoch: u64,
    pub secure: Option<bool>,
    pub identity: Option<LinearIdentity>,
    pub loading_account: bool,
    pub busy: bool,
    pub loading_issues: bool,
    pub confirm_disconnect: bool,
    pub error: String,
    pub selection: IssueSelection,
    pub teams: Vec<LinearTeam>,
    pub states: Vec<LinearState>,
    pub issues: Vec<LinearIssue>,
    pub has_more: bool,
    pub selected: Option<LinearIssue>,
    pub agents: Vec<AgentChoice>,
    pub agent: String,
    pub isolated: bool,
    pub project_name: String,
    pub start_states: Vec<LinearState>,
    pub loading_start_states: bool,
    pub next_state: Option<String>,
}
/// No secret-bearing command variant: key submission has its own private method.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Close,
    Retry,
    Refresh,
    ConfirmDisconnect(bool),
    Disconnect,
    Filter(LinearFilter),
    Team(Option<String>),
    State(Option<String>),
    Select(String),
    Back,
    Agent(String),
    Isolated(bool),
    NextState(Option<String>),
    Start,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Capability,
    Account,
    Teams,
    States,
    Issues,
    StartStates,
    Update,
}
#[derive(Default)]
pub struct Panel {
    pub view: Snapshot,
    worker: Option<Worker>,
    path: Option<PathBuf>,
    pending: Vec<(Slot, u64)>,
    issue_permit: Option<IssuePermit>,
    pub(crate) project: Option<(u64, PathBuf)>,
    pub(crate) requested_start: Option<u64>,
    sequence: u64,
    launch: Option<Launch>,
    warning: Option<String>,
    // Compatibility input only. Never copied to a public snapshot or serialized.
    pub(crate) key: Zeroizing<String>,
    pub(crate) rejected_key: bool,
}
impl Panel {
    pub fn open(&mut self, path: Option<PathBuf>) {
        if self.view.open {
            return;
        }
        let Some(epoch) = self.view.epoch.checked_add(1) else {
            return;
        };
        self.view = Snapshot {
            open: true,
            epoch,
            isolated: true,
            ..Default::default()
        };
        self.key = Zeroizing::new(String::new());
        self.rejected_key = false;
        self.path = path;
        self.pending.clear();
        self.project = None;
        self.requested_start = None;
        self.issue_permit = None;
        self.status();
    }
    fn send(&mut self, slot: Slot, action: Action) {
        self.pending.retain(|(s, _)| *s != slot);
        let result = (|| {
            if self.worker.is_none() {
                self.worker = Some(Worker::new_at(
                    self.path.clone().ok_or("No private data directory")?,
                )?);
            }
            self.worker
                .as_mut()
                .ok_or("Linear worker unavailable")?
                .submit(action)
        })();
        match result {
            Ok(id) => self.pending.push((slot, id)),
            Err(error) => {
                self.view.error = error;
                self.set_idle(slot);
            }
        }
    }
    fn status(&mut self) {
        self.clear_start_states();
        self.issue_permit = None;
        self.pending.clear();
        self.view.loading_issues = false;
        self.view.loading_account = true;
        self.view.secure = None;
        self.send(Slot::Capability, Action::Capability);
        self.send(Slot::Account, Action::Status);
    }
    pub fn connect(&mut self, epoch: u64, key: Zeroizing<String>) {
        if !self.accepts(epoch)
            || self.rejected_key
            || self.view.secure != Some(true)
            || self.view.identity.is_some()
        {
            return;
        }
        self.view.error.clear();
        self.view.busy = true;
        self.issue_permit = None;
        self.pending.clear();
        self.view.loading_issues = false;
        self.send(Slot::Account, Action::Connect(key));
    }
    pub fn accepts(&self, epoch: u64) -> bool {
        self.view.open && self.view.epoch == epoch && !self.view.busy
    }
    pub fn command(&mut self, epoch: u64, command: Command) {
        if !self.accepts(epoch) {
            return;
        }
        match command {
            Command::Close => {
                self.issue_permit = None;
                self.view.open = false;
                self.pending.clear();
                self.key = Zeroizing::new(String::new());
                self.rejected_key = false;
                self.view.selected = None;
                self.view.issues.clear();
            }
            Command::Retry => {
                self.view.error.clear();
                self.status();
            }
            Command::ConfirmDisconnect(value) => self.view.confirm_disconnect = value,
            Command::Disconnect if self.view.confirm_disconnect => {
                self.issue_permit = None;
                self.view.busy = true;
                self.view.error.clear();
                self.pending.clear();
                self.view.loading_issues = false;
                self.send(Slot::Account, Action::Disconnect);
            }
            Command::Refresh if self.view.identity.is_some() => {
                self.view.error.clear();
                self.read(Slot::Teams, Query::Teams, true);
                self.issues(true);
            }
            Command::Filter(filter) if self.view.identity.is_some() => {
                self.view.selection.filter = filter;
                self.issues(false);
            }
            Command::Team(team) if self.view.identity.is_some() => {
                if team
                    .as_ref()
                    .is_some_and(|id| !self.view.teams.iter().any(|t| &t.id == id))
                {
                    return;
                }
                self.view.selection.team_id = team.clone();
                self.view.selection.state_id = None;
                self.view.states.clear();
                self.pending.retain(|(s, _)| *s != Slot::States);
                if let Some(id) = team {
                    self.read(Slot::States, Query::States(id), false);
                }
                self.issues(false);
            }
            Command::State(state) if self.view.identity.is_some() => {
                if state
                    .as_ref()
                    .is_some_and(|id| !self.view.states.iter().any(|s| &s.id == id))
                {
                    return;
                }
                self.view.selection.state_id = state;
                self.issues(false);
            }
            Command::Select(id) if !self.view.loading_issues => {
                self.clear_start_states();
                self.view.selected = self
                    .view
                    .issues
                    .iter()
                    .find(|issue| issue.id == id)
                    .cloned();
                if let Some(team) = self
                    .view
                    .selected
                    .as_ref()
                    .map(|issue| issue.team.id.clone())
                    .filter(|id| !id.is_empty())
                {
                    self.view.loading_start_states = true;
                    self.read(Slot::StartStates, Query::States(team), false);
                }
            }
            Command::Back => {
                self.view.selected = None;
                self.clear_start_states();
            }
            Command::Agent(id) if self.view.agents.iter().any(|a| a.id == id && a.available) => {
                self.view.agent = id
            }
            Command::Isolated(value) => self.view.isolated = value,
            Command::NextState(state)
                if state
                    .as_ref()
                    .is_none_or(|id| self.view.start_states.iter().any(|s| &s.id == id)) =>
            {
                self.view.next_state = state
            }
            Command::Start => self.requested_start = Some(epoch),
            _ => {}
        }
    }
    fn clear_start_states(&mut self) {
        self.view.start_states.clear();
        self.view.next_state = None;
        self.view.loading_start_states = false;
        self.pending.retain(|(s, _)| *s != Slot::StartStates);
    }
    fn read(&mut self, slot: Slot, query: Query, refresh: bool) {
        self.send(slot, Action::Read { query, refresh });
    }
    fn issues(&mut self, refresh: bool) {
        self.clear_start_states();
        self.issue_permit = None;
        self.view.selected = None;
        self.view.loading_issues = true;
        self.read(
            Slot::Issues,
            Query::Issues(self.view.selection.clone()),
            refresh,
        );
    }
    fn set_idle(&mut self, slot: Slot) {
        match slot {
            Slot::Account => {
                self.view.loading_account = false;
                self.view.busy = false;
            }
            Slot::Issues => self.view.loading_issues = false,
            Slot::StartStates => self.view.loading_start_states = false,
            _ => {}
        }
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        // At most the bounded event queue per frame; never wait for network or thread completion.
        for _ in 0..32 {
            let Some(worker) = self.worker.as_mut() else {
                break;
            };
            let event = match worker.poll() {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(error) => {
                    self.worker_failed(error);
                    return true;
                }
            };
            let Some((slot, _)) = self
                .pending
                .iter()
                .find(|(_, id)| *id == event.id())
                .copied()
            else {
                continue;
            };
            if !self.view.open {
                continue;
            }
            self.pending.retain(|(s, _)| *s != slot);
            self.set_idle(slot);
            changed = true;
            if slot == Slot::Update {
                let success = event
                    .apply(|_, result| matches!(result, Ok(Reply::Updated(_))))
                    .unwrap_or(false);
                self.finish_update(success);
                continue;
            }
            // Apply public data WHILE the account/revision guard is held. Follow-up requests
            // must be enqueued only AFTER this callback returns (no authority re-entry).
            let mut reload = false;
            if let Err(error) = event.apply_with_permit(|_, result, permit| {
                if slot == Slot::Issues {
                    self.issue_permit = permit;
                }
                reload = self.receive(slot, result);
            }) {
                if slot == Slot::Issues {
                    self.issue_permit = None;
                }
                self.view.error = error;
            }
            if reload {
                self.read(Slot::Teams, Query::Teams, false);
                self.issues(false);
            }
        }
        changed
    }
    fn worker_failed(&mut self, error: String) {
        if self.launch.as_ref().is_some_and(|l| l.accepted) {
            self.finish_update(false);
        } else {
            // A pending task still owns its old permit, but dropping the worker
            // revokes that authority. Its later callback must not touch a new dialog.
            self.launch = None;
        }
        self.requested_start = None;
        self.issue_permit = None;
        self.worker = None;
        self.pending.clear();
        self.view.busy = false;
        self.view.loading_account = false;
        self.view.loading_issues = false;
        self.view.loading_start_states = false;
        self.view.error = error;
    }
    pub(crate) fn begin_launch(
        &mut self,
        epoch: u64,
    ) -> Result<crate::external_issue::Linear, String> {
        let (metadata, permit) = self.capture_selected(epoch)?;
        if self.launch.is_some()
            || self.project.is_none()
            || !self
                .view
                .agents
                .iter()
                .any(|a| a.id == self.view.agent && a.available)
        {
            return Err("Choose a local project and an available native agent".into());
        }
        if self
            .view
            .next_state
            .as_ref()
            .is_some_and(|id| !self.view.start_states.iter().any(|s| &s.id == id))
        {
            return Err("Choose an available Linear state".into());
        }
        let key = self
            .sequence
            .checked_add(1)
            .ok_or("Linear launch identity exhausted")?;
        self.sequence = key;
        self.launch = Some(Launch {
            key,
            issue: metadata.issue_id().into(),
            permit,
            next_state: self.view.next_state.clone(),
            accepted: false,
        });
        self.view.busy = true;
        self.view.error.clear();
        Ok(metadata.with_launch_key(key))
    }
    /// Called only after the controller accepts its native first turn, or definitively fails.
    pub(crate) fn launch_finished(&mut self, key: u64, result: Result<(), String>) -> bool {
        let Some(launch) = self.launch.as_mut().filter(|l| l.key == key && !l.accepted) else {
            return false;
        };
        if let Err(error) = result {
            self.launch = None;
            self.view.busy = false;
            self.view.error = error;
            return false;
        }
        launch.accepted = true;
        if let Some(state) = launch.next_state.clone() {
            let result = self
                .worker
                .as_mut()
                .ok_or("Linear worker unavailable".to_owned())
                .and_then(|w| {
                    w.submit_issue_update(launch.permit.clone(), launch.issue.clone(), state)
                });
            match result {
                Ok(id) => self.pending.push((Slot::Update, id)),
                Err(_) => self.finish_update(false),
            }
        } else {
            self.finish_update(true);
        }
        true
    }
    fn finish_update(&mut self, success: bool) {
        self.launch = None;
        self.view.busy = false;
        self.view.open = false;
        self.pending.clear();
        self.issue_permit = None;
        self.view.selected = None;
        self.view.issues.clear();
        if !success {
            self.warning = Some("The task started, but its Linear state could not be changed. Refresh Linear and change it there.".into());
        }
    }
    pub(crate) fn take_warning(&mut self) -> Option<String> {
        self.warning.take()
    }
    /// Private captured selection for the eventual owner launch controller. No snapshot authority.
    #[allow(dead_code)]
    pub(crate) fn capture_selected(
        &self,
        epoch: u64,
    ) -> Result<(crate::external_issue::Linear, IssuePermit), String> {
        if !self.accepts(epoch) || self.view.loading_account || self.view.loading_issues {
            return Err("Linear selection changed; select the issue again".into());
        }
        let selected = self.view.selected.as_ref().ok_or("Choose a Linear issue")?;
        let issue = self
            .view
            .issues
            .iter()
            .find(|issue| *issue == selected)
            .ok_or("Linear selection changed; select the issue again")?;
        let permit = self
            .issue_permit
            .as_ref()
            .ok_or("Refresh Linear issues first")?;
        let metadata = crate::external_issue::Linear::new(
            &issue.id,
            &issue.identifier,
            &issue.title,
            &issue.url,
        )?
        .authorize(permit.clone())?;
        Ok((metadata, permit.clone()))
    }
    fn receive(&mut self, slot: Slot, reply: Result<Reply, String>) -> bool {
        match (slot, reply) {
            (Slot::Capability, Ok(Reply::Capability(secure))) => self.view.secure = Some(secure),
            (Slot::Account, Ok(Reply::Status(identity))) => {
                self.clear_start_states();
                self.issue_permit = None;
                self.view.identity = identity;
                self.view.confirm_disconnect = false;
                self.view.selection = IssueSelection::default();
                self.view.teams.clear();
                self.view.states.clear();
                self.view.issues.clear();
                self.view.selected = None;
                self.key = Zeroizing::new(String::new());
                return self.view.identity.is_some();
            }
            (Slot::Teams, Ok(Reply::Teams(page))) => self.view.teams = page.items,
            (Slot::States, Ok(Reply::States(page))) => self.view.states = page.items,
            (Slot::StartStates, Ok(Reply::States(page))) => self.view.start_states = page.items,
            (Slot::Issues, Ok(Reply::Issues(page))) => {
                self.view.issues = page.items;
                self.view.has_more = page.has_next_page;
            }
            (_, Err(error)) => self.view.error = error,
            _ => self.view.error = "Unexpected Linear response".into(),
        }
        false
    }
}

pub fn issue_text(issue: &LinearIssue) -> String {
    format!(
        "{} {}\n{}\n\n{}",
        issue.identifier, issue.title, issue.url, issue.description
    )
}

#[cfg(any(test, feature = "visual-test"))]
pub(crate) fn fixture_view() -> Snapshot {
    let issue = LinearIssue {
        id: "fixture".into(), identifier: "TES-42".into(), title: "Améliorer le panneau des projets".into(),
        url: "https://linear.app/demo/issue/TES-42".into(),
        description: "Conserver la sélection et la position de défilement lorsque la liste des projets est actualisée.".into(),
        team: LinearTeam { id: "team".into(), name: "Tessel".into(), ..Default::default() },
        state: LinearState { name: "À faire".into(), ..Default::default() },
        ..Default::default()
    };
    Snapshot {
        open: true,
        epoch: 8,
        secure: Some(true),
        identity: Some(LinearIdentity {
            viewer: LinearPerson {
                name: "Compte de démonstration".into(),
                ..Default::default()
            },
            organization: Default::default(),
        }),
        issues: vec![issue.clone()],
        selected: Some(issue),
        project_name: "Projet de démonstration".into(),
        agents: vec![
            AgentChoice {
                id: "codex".into(),
                label: "Codex".into(),
                available: true,
            },
            AgentChoice {
                id: "claude".into(),
                label: "Claude".into(),
                available: false,
            },
        ],
        agent: "codex".into(),
        isolated: true,
        start_states: vec![LinearState {
            id: "working".into(),
            name: "En cours".into(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::{
        request::Generation,
        service::{Credentials, Remote, Service, Store, Transport},
    };
    use super::*;
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    struct FixtureStore;
    impl Store for FixtureStore {
        fn encryption(&self) -> Result<(), String> {
            Ok(())
        }
        fn load(&self) -> Result<Option<Credentials>, String> {
            Ok(Some(Credentials {
                key: Zeroizing::new("synthetic-secret".into()),
                identity: LinearIdentity {
                    viewer: LinearPerson {
                        id: "viewer".into(),
                        ..Default::default()
                    },
                    organization: Default::default(),
                },
            }))
        }
        fn save(&self, _: &Credentials) -> Result<(), String> {
            panic!("unexpected account change")
        }
        fn remove(&self) -> Result<(), String> {
            Err("synthetic removal refused".into())
        }
    }
    struct FixtureTransport;
    impl Transport for FixtureTransport {
        fn request<'a>(
            &'a self,
            _: &'a str,
            request: Remote,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Reply, String>> + Send + 'a>>
        {
            Box::pin(async move {
                let page = |items| Page {
                    items,
                    has_next_page: false,
                    end_cursor: None,
                };
                match request {
                    Remote::Read {
                        query: Query::States(team),
                        ..
                    } => Ok(Reply::States(Page {
                        items: vec![LinearState {
                            id: format!("{team}-working"),
                            name: "In progress".into(),
                            ..Default::default()
                        }],
                        has_next_page: false,
                        end_cursor: None,
                    })),
                    Remote::Read {
                        query: Query::Teams,
                        ..
                    } => Ok(Reply::Teams(Page {
                        items: vec![LinearTeam {
                            id: "team".into(),
                            name: "Fixture team".into(),
                            ..Default::default()
                        }],
                        has_next_page: false,
                        end_cursor: None,
                    })),
                    Remote::Read {
                        query: Query::Issues(selection),
                        ..
                    } => {
                        if selection.filter == LinearFilter::Created {
                            tokio::time::sleep(Duration::from_millis(40)).await;
                        }
                        Ok(Reply::Issues(page(vec![LinearIssue {
                            id: selection.filter.id_str().into(),
                            identifier: "TEAM-1".into(),
                            url: "https://linear.app/team/issue/TEAM-1".into(),
                            title: "Synthetic issue".into(),
                            team: LinearTeam {
                                id: "team".into(),
                                name: "Fixture team".into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }])))
                    }
                    _ => panic!("unexpected fixture request"),
                }
            })
        }
    }
    fn wait(panel: &mut Panel, ready: impl Fn(&Panel) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            panel.poll();
            if ready(panel) {
                return;
            }
            assert!(Instant::now() < deadline, "fixture worker timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    struct LaunchTransport {
        updates: Arc<std::sync::atomic::AtomicUsize>,
        fail: bool,
    }
    impl Transport for LaunchTransport {
        fn request<'a>(
            &'a self,
            key: &'a str,
            request: Remote,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Reply, String>> + Send + 'a>>
        {
            Box::pin(async move {
                if let Remote::SetState { issue, state } = request {
                    assert_eq!(issue, "assigned");
                    assert_eq!(state, "team-working");
                    self.updates
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if self.fail {
                        return Err("synthetic update refused".into());
                    }
                    Ok(Reply::Updated(Box::new(LinearIssue {
                        id: issue,
                        ..Default::default()
                    })))
                } else {
                    FixtureTransport.request(key, request).await
                }
            })
        }
    }
    pub(crate) fn launch_fixture(fail: bool) -> (Panel, Arc<std::sync::atomic::AtomicUsize>) {
        let updates = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let owner = Generation::default();
        let service = Service::with_backends(
            owner.scope(),
            Arc::new(FixtureStore),
            Arc::new(LaunchTransport {
                updates: updates.clone(),
                fail,
            }),
        );
        let mut panel = Panel {
            worker: Some(Worker::with_service(owner, service).unwrap()),
            ..Default::default()
        };
        panel.open(None);
        wait(&mut panel, |p| {
            !p.view.loading_account && !p.view.loading_issues && !p.view.issues.is_empty()
        });
        panel.command(panel.view.epoch, Command::Select("assigned".into()));
        wait(&mut panel, |p| !p.view.loading_start_states);
        panel.project = Some((1, PathBuf::from("synthetic")));
        panel.view.project_name = "Fixture project".into();
        panel.view.agents = vec![AgentChoice {
            id: "codex".into(),
            label: "Codex".into(),
            available: true,
        }];
        panel.view.agent = "codex".into();
        (panel, updates)
    }
    #[test]
    fn launch_updates_only_explicit_state_after_acceptance_and_never_retries_task() {
        use std::sync::atomic::Ordering;
        for fail in [false, true] {
            let (mut panel, updates) = launch_fixture(fail);
            let epoch = panel.view.epoch;
            panel.command(epoch, Command::NextState(Some("foreign".into())));
            assert!(panel.view.next_state.is_none());
            panel.command(epoch, Command::NextState(Some("team-working".into())));
            let metadata = panel.begin_launch(epoch).unwrap();
            let key = metadata.launch_key().unwrap();
            assert!(panel.begin_launch(epoch).is_err());
            panel.command(epoch, Command::Close);
            panel.command(epoch, Command::NextState(None));
            assert!(panel.view.open && panel.view.busy);
            assert_eq!(updates.load(Ordering::SeqCst), 0);
            assert!(!panel.launch_finished(key + 1, Ok(())));
            assert!(panel.launch_finished(key, Ok(())));
            assert!(!panel.launch_finished(key, Ok(())));
            wait(&mut panel, |p| !p.view.open);
            assert_eq!(updates.load(Ordering::SeqCst), 1);
            assert_eq!(panel.take_warning().is_some(), fail);
            assert!(!panel.launch_finished(key, Ok(())));
            assert!(panel.begin_launch(epoch).is_err());
        }
    }
    #[test]
    fn failed_worker_revokes_pending_launch_and_old_callbacks_cannot_touch_reopened_dialog() {
        let (mut panel, _) = launch_fixture(false);
        let epoch = panel.view.epoch;
        let metadata = panel.begin_launch(epoch).unwrap();
        let key = metadata.launch_key().unwrap();
        panel.worker_failed("Synthetic worker failure".into());
        assert!(metadata.commit(|| Ok(())).is_err());
        assert!(!panel.view.busy);
        panel.command(epoch, Command::Close);
        let (mut replacement, _) = launch_fixture(false);
        panel.worker = replacement.worker.take();
        panel.open(None);
        wait(&mut panel, |p| {
            !p.view.loading_account && !p.view.loading_issues
        });
        assert!(panel.view.error.is_empty());
        assert!(!panel.launch_finished(key, Err("Old launch failed".into())));
        assert!(!panel.launch_finished(key, Ok(())));
        assert!(panel.view.open && panel.view.error.is_empty());
    }

    #[test]
    fn worker_failure_after_submission_warns_without_reopening_a_startable_task() {
        let (mut panel, _) = launch_fixture(false);
        let epoch = panel.view.epoch;
        panel.command(epoch, Command::NextState(Some("team-working".into())));
        let key = panel.begin_launch(epoch).unwrap().launch_key().unwrap();
        assert!(panel.launch_finished(key, Ok(())));
        panel.worker_failed("Synthetic worker failure".into());
        assert!(!panel.view.open);
        assert!(panel.take_warning().is_some());
        assert!(!panel.launch_finished(key, Ok(())));
        assert!(panel.begin_launch(epoch).is_err());
    }

    #[test]
    fn launch_failure_keeps_selection_and_default_success_does_not_mutate_linear() {
        use std::sync::atomic::Ordering;
        let (mut panel, updates) = launch_fixture(false);
        let epoch = panel.view.epoch;
        let key = panel.begin_launch(epoch).unwrap().launch_key().unwrap();
        assert!(!panel.launch_finished(key, Err("synthetic launch failure".into())));
        assert!(panel.view.open && !panel.view.busy && panel.view.selected.is_some());
        assert_eq!(updates.load(Ordering::SeqCst), 0);
        let next = panel.begin_launch(epoch).unwrap().launch_key().unwrap();
        assert_ne!(next, key);
        assert!(!panel.launch_finished(key, Ok(())));
        assert!(panel.launch_finished(next, Ok(())));
        assert!(!panel.view.open);
        assert_eq!(updates.load(Ordering::SeqCst), 0);
        assert!(panel.take_warning().is_none());
    }
    #[test]
    fn actual_worker_delivery_obeys_latest_filter_close_and_failed_disconnect() {
        let owner = Generation::default();
        let service = Service::with_backends(
            owner.scope(),
            Arc::new(FixtureStore),
            Arc::new(FixtureTransport),
        );
        let worker = Worker::with_service(owner, service).unwrap();
        let mut panel = Panel {
            worker: Some(worker),
            ..Default::default()
        };
        panel.open(None); // no default profile fallback: exclusively injected fixture worker
        wait(&mut panel, |p| {
            !p.view.loading_account && !p.view.loading_issues && !p.view.issues.is_empty()
        });
        assert_eq!(panel.view.issues[0].id, "assigned");
        let epoch = panel.view.epoch;
        panel.command(epoch, Command::Filter(LinearFilter::Created));
        panel.command(epoch, Command::Filter(LinearFilter::Open));
        wait(&mut panel, |p| !p.view.loading_issues);
        assert_eq!(panel.view.issues[0].id, "open");
        // Drain even after the older delayed request has completed.
        std::thread::sleep(Duration::from_millis(70));
        panel.poll();
        assert_eq!(panel.view.issues[0].id, "open");
        panel.command(epoch, Command::Select("open".into()));
        assert!(panel.view.selected.is_some());
        let (metadata, permit) = panel.capture_selected(epoch).unwrap();
        assert_eq!(metadata.issue_id(), "open");
        assert!(permit.commit("open", || Ok(())).is_ok());
        assert!(panel.capture_selected(epoch + 1).is_err());
        panel.view.selected.as_mut().unwrap().title = "Changed snapshot".into();
        assert!(panel.capture_selected(epoch).is_err());
        panel.command(epoch, Command::Select("open".into()));
        panel.command(epoch, Command::Back);
        assert!(panel.view.selected.is_none());
        assert!(panel.capture_selected(epoch).is_err());
        panel.command(epoch, Command::Select("open".into()));
        panel.command(epoch, Command::ConfirmDisconnect(true));
        panel.command(epoch, Command::Disconnect);
        assert!(panel.capture_selected(epoch).is_err());
        wait(&mut panel, |p| !p.view.busy);
        assert!(panel.view.identity.is_some());
        assert_eq!(panel.view.error, "synthetic removal refused");
        // Failed disconnect still revoked the captured account generation.
        assert!(permit.commit("open", || Ok(())).is_err());
        assert!(panel.capture_selected(epoch).is_err());
        panel.command(epoch, Command::Close);
        panel.open(None);
        panel.command(epoch, Command::Close);
        assert!(panel.view.open);
        wait(&mut panel, |p| {
            !p.view.loading_account && !p.view.loading_issues && !p.view.issues.is_empty()
        });
        assert_eq!(panel.view.issues[0].id, "assigned");
        let current = panel.view.epoch;
        panel.command(current, Command::Select("assigned".into()));
        assert!(panel.capture_selected(current).is_ok());
        panel.command(current, Command::Refresh);
        assert!(panel.capture_selected(current).is_err());
        wait(&mut panel, |p| !p.view.loading_issues);
        panel.command(current, Command::Select("assigned".into()));
        assert!(panel.capture_selected(current).is_ok());
        panel.command(current, Command::Close);
        assert!(panel.issue_permit.is_none());
        assert!(panel.capture_selected(current).is_err());
    }
    #[test]
    fn stale_modal_callbacks_and_busy_close_do_not_change_current_dialog() {
        let mut panel = Panel::default();
        panel.view.open = true;
        panel.view.epoch = 2;
        panel.command(1, Command::Close);
        assert!(panel.view.open);
        panel.view.busy = true;
        panel.command(2, Command::Close);
        assert!(panel.view.open);
        panel.view.busy = false;
        panel.key = Zeroizing::new("fixture-secret".into());
        panel.command(2, Command::Close);
        assert!(!panel.view.open);
        assert!(panel.key.is_empty());
        assert!(!format!("{:?}", panel.view).contains("fixture-secret"));
    }
    #[test]
    fn switching_team_discards_state_and_rejects_unknown_selection() {
        let mut panel = Panel::default();
        panel.view.open = true;
        panel.view.identity = Some(LinearIdentity {
            viewer: Default::default(),
            organization: Default::default(),
        });
        panel.view.selection.team_id = Some("old".into());
        panel.view.selection.state_id = Some("old-state".into());
        panel.command(0, Command::Team(Some("foreign".into())));
        assert_eq!(panel.view.selection.team_id.as_deref(), Some("old"));
        panel.command(0, Command::Team(None));
        assert_eq!(panel.view.selection.team_id, None);
        assert_eq!(panel.view.selection.state_id, None);
        assert!(panel.view.states.is_empty());
    }
}
