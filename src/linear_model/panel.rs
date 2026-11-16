//! Owner-local dialog controller. Neither its key draft nor worker is snapshot/Debug data.
use super::{
    service::{Action, IssuePermit, Query, Reply},
    worker::Worker,
    *,
};

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
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Capability,
    Account,
    Teams,
    States,
    Issues,
}
#[derive(Default)]
pub struct Panel {
    pub view: Snapshot,
    worker: Option<Worker>,
    path: Option<PathBuf>,
    pending: Vec<(Slot, u64)>,
    issue_permit: Option<IssuePermit>,
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
            ..Default::default()
        };
        self.key = Zeroizing::new(String::new());
        self.rejected_key = false;
        self.path = path;
        self.pending.clear();
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
                self.view.selected = self
                    .view
                    .issues
                    .iter()
                    .find(|issue| issue.id == id)
                    .cloned();
            }
            Command::Back => self.view.selected = None,
            _ => {}
        }
    }
    fn read(&mut self, slot: Slot, query: Query, refresh: bool) {
        self.send(slot, Action::Read { query, refresh });
    }
    fn issues(&mut self, refresh: bool) {
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
                    self.issue_permit = None;
                    self.worker = None;
                    self.pending.clear();
                    self.view.busy = false;
                    self.view.loading_account = false;
                    self.view.loading_issues = false;
                    self.view.error = error;
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

#[cfg(test)]
mod tests {
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
