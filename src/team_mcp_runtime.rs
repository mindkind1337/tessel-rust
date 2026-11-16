//! Main-thread team dispatch. No provider process is started by this module.
use crate::team_mcp_broker::{Binding, Broker, Launch};
use crate::team_mcp_stdio::{BackendError, Operation};
use crate::teams_relay::TeamsRelayHub;
use crate::workspace::{Store, Workspace};
use serde_json::Value;
use std::path::PathBuf;

const MAX_PER_POLL: usize = 8;

/// Construct only from the desktop's current local session, never a saved/imported
/// session or an MCP argument. `active` includes an actual in-progress child spawn.
#[derive(Clone, Debug)]
pub struct LivePane {
    pub pane: u64,
    pub project: u64,
    pub cwd: PathBuf,
    pub local: bool,
    pub active: bool,
}

impl LivePane {
    fn matches(&self, binding: &Binding) -> bool {
        self.local
            && self.active
            && self.pane == binding.pane
            && self.project == binding.project
            && self.cwd.is_absolute()
            && self.cwd.is_dir()
            && dunce::canonicalize(&self.cwd).is_ok_and(|cwd| cwd == binding.cwd)
            && dunce::canonicalize(&binding.cwd).is_ok_and(|cwd| cwd == binding.cwd)
    }
}

#[derive(Default)]
pub struct Runtime {
    broker: Option<Broker>,
}

fn allowed(name: &str) -> bool {
    matches!(
        name,
        "tessel_agents"
            | "tessel_tasks"
            | "tessel_task_add"
            | "tessel_task_assign"
            | "tessel_task_move"
            | "team_tasks"
            | "team_task_move"
            | "team_task_add"
            | "team_create"
            | "team_bootstrap"
            | "team_members"
            | "team_send"
            | "team_ask"
            | "team_inbox"
            | "team_task_gate"
            | "team_gates"
            | "team_worker_start"
            | "team_worker_list"
            | "team_worker_read"
            | "team_worker_stop"
            | "team_worker_release"
            | "team_worker_done"
            | "team_heartbeat"
            | "project_agents"
            | "project_send"
            | "project_inbox"
    )
}

fn catalogue() -> Value {
    let definitions = crate::agent_tools::definitions();
    Value::Array(
        definitions
            .as_array()
            .into_iter()
            .flatten()
            .filter(|tool| tool["name"].as_str().is_some_and(allowed))
            .cloned()
            .collect::<Vec<_>>(),
    )
}

fn current(binding: &Binding, panes: &[LivePane], state: &Workspace) -> bool {
    let mut matching = panes.iter().filter(|pane| pane.pane == binding.pane);
    matching.next().is_some_and(|pane| pane.matches(binding))
        && matching.next().is_none()
        && state
            .projects
            .iter()
            .any(|project| project.id == binding.project)
}

impl Runtime {
    #[cfg(test)]
    pub(crate) fn queued_for_test(&self) -> usize {
        self.broker.as_ref().map_or(0, Broker::queued_for_test)
    }

    pub fn enabled(&self) -> bool {
        self.broker.is_some()
    }

    /// Call immediately before each actual child spawn, including reconnects.
    /// The returned owner must live in that transport and be revoked on cancellation.
    /// No call sites in production launch providers until their adapters are wired.
    pub fn begin(&mut self, pane: &LivePane, state: &Workspace) -> Result<Launch, String> {
        self.revoke(pane.pane);
        if !pane.local
            || !pane.active
            || !pane.cwd.is_absolute()
            || !state
                .projects
                .iter()
                .any(|project| project.id == pane.project)
        {
            return Err("No active local pane for team transport".into());
        }
        let cwd =
            dunce::canonicalize(&pane.cwd).map_err(|_| "Cannot resolve team working directory")?;
        if !cwd.is_dir() {
            return Err("Team working directory is not a directory".into());
        }
        if self.broker.is_none() {
            self.broker = Some(Broker::bind()?);
        }
        self.broker
            .as_ref()
            .ok_or("Team broker unavailable")?
            .launch(pane.pane, pane.project, &cwd)
    }

    pub fn revoke(&mut self, pane: u64) {
        if let Some(broker) = &self.broker {
            broker.revoke(pane);
        }
    }

    /// The application calls this on its main thread with a fresh runtime snapshot.
    /// Returns whether a tool changed workspace/team metadata; callers retain the
    /// ordinary save/repaint flow. Task mutations use their existing persisted path.
    pub fn poll(
        &mut self,
        panes: &[LivePane],
        state: &mut Workspace,
        store: Option<&Store>,
        hub: &mut TeamsRelayHub,
    ) -> bool {
        let Some(broker) = &self.broker else {
            return false;
        };
        let Ok(bindings) = broker.bindings() else {
            // Poisoned authority must not leave a listener accepting further work.
            self.broker = None;
            return false;
        };
        for binding in bindings {
            if !current(&binding, panes, state) {
                broker.revoke(binding.pane);
            }
        }
        let mut changed = false;
        for _ in 0..MAX_PER_POLL {
            let Some(pending) = broker.try_next() else {
                break;
            };
            if !pending.is_current() || !current(pending.binding(), panes, state) {
                continue;
            }
            let result = match pending.operation() {
                Operation::List => Ok(catalogue()),
                Operation::Call { name, arguments } if allowed(name) => {
                    let binding = pending.binding();
                    // Recheck cancellation after catalogue/routing work and directly
                    // before execution. A mutation already executing is not rolled back
                    // by a later client disconnect; completion then remains uncertain.
                    if !pending.is_current() || !current(binding, panes, state) {
                        continue;
                    }
                    crate::agent_tools::execute_persisted_for_pane(
                        state,
                        store,
                        hub,
                        binding.pane,
                        binding.project,
                        name,
                        arguments,
                    )
                    .map(|(value, did_change)| {
                        changed |= did_change;
                        value
                    })
                    .map_err(BackendError::Failed)
                }
                Operation::Call { .. } => Err(BackendError::UnknownTool),
            };
            // Never replay a failed or late completion: it may already have mutated.
            let _ = pending.finish(result);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        sync::{Arc, mpsc},
        thread,
        time::{Duration, Instant},
    };

    struct Fixture {
        _dir: tempfile::TempDir,
        state: Workspace,
        store: Store,
        hub: TeamsRelayHub,
        runtime: Runtime,
        pane: LivePane,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let (store, mut state) = Store::open(dir.path()).unwrap();
            let project = state.add_project(dir.path()).unwrap();
            let cwd = state
                .projects
                .iter()
                .find(|p| p.id == project)
                .unwrap()
                .path
                .clone();
            state.sessions.push(
                serde_json::from_value(json!({"id":41,"project":project,
                "title":"Synthetic worker","launch":"ClaudeChat","cwd":cwd}))
                .unwrap(),
            );
            store.save(&state).unwrap();
            Self {
                _dir: dir,
                state,
                store,
                hub: Default::default(),
                runtime: Default::default(),
                pane: LivePane {
                    pane: 41,
                    project,
                    cwd,
                    local: true,
                    active: true,
                },
            }
        }

        fn launch(&mut self) -> Arc<Launch> {
            Arc::new(self.runtime.begin(&self.pane, &self.state).unwrap())
        }

        fn queued(&self) {
            wait(|| self.runtime.broker.as_ref().unwrap().queued_for_test() > 0);
        }

        fn poll(&mut self, panes: &[LivePane]) -> bool {
            self.runtime
                .poll(panes, &mut self.state, Some(&self.store), &mut self.hub)
        }

        fn exchange(
            &mut self,
            launch: &Arc<Launch>,
            operation: Operation,
        ) -> Result<Value, BackendError> {
            let (receiver, worker) = request(launch, operation);
            let deadline = Instant::now() + Duration::from_secs(3);
            let pane = self.pane.clone();
            loop {
                self.poll(std::slice::from_ref(&pane));
                if let Ok(result) = receiver.try_recv() {
                    worker.join().unwrap();
                    return result.unwrap();
                }
                assert!(
                    Instant::now() < deadline,
                    "synthetic broker did not respond"
                );
                thread::sleep(Duration::from_millis(2));
            }
        }
    }

    type Reply = Result<Result<Value, BackendError>, String>;
    fn request(
        launch: &Arc<Launch>,
        operation: Operation,
    ) -> (mpsc::Receiver<Reply>, thread::JoinHandle<()>) {
        let launch = launch.clone();
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _ = sender.send(launch.client().call(operation));
        });
        (receiver, worker)
    }
    fn wait(mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !ready() {
            assert!(Instant::now() < deadline, "synthetic request did not queue");
            thread::sleep(Duration::from_millis(2));
        }
    }
    fn add() -> Operation {
        Operation::Call {
            name: "tessel_task_add".into(),
            arguments: json!({"title":"Synthetic task"}),
        }
    }

    #[test]
    fn dormant_runtime_has_no_listener_and_invalid_launch_cannot_activate_one() {
        let mut fixture = Fixture::new();
        assert!(!fixture.runtime.enabled());
        assert!(!fixture.poll(&[fixture.pane.clone()]));
        for bad in [
            LivePane {
                local: false,
                ..fixture.pane.clone()
            },
            LivePane {
                active: false,
                ..fixture.pane.clone()
            },
            LivePane {
                cwd: "relative".into(),
                ..fixture.pane.clone()
            },
            LivePane {
                project: u64::MAX,
                ..fixture.pane.clone()
            },
        ] {
            assert!(fixture.runtime.begin(&bad, &fixture.state).is_err());
            assert!(!fixture.runtime.enabled());
        }
    }

    #[test]
    fn authenticated_dispatch_uses_persisted_tools_and_a_restricted_catalogue() {
        let mut fixture = Fixture::new();
        let launch = fixture.launch();
        let list = fixture.exchange(&launch, Operation::List).unwrap();
        let names: Vec<_> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"team_inbox") && names.contains(&"tessel_task_add"));
        assert!(names.contains(&"team_tasks") && names.contains(&"team_task_move"));
        assert!(names.contains(&"team_task_add"));
        assert!(!names.contains(&"tessel_browser"));
        assert!(names.iter().all(|name| allowed(name)));
        let result = fixture.exchange(&launch, add()).unwrap();
        assert!(result["id"].as_u64().is_some());
        assert_eq!(fixture.state.tasks.len(), 1);
        let loaded: Workspace = serde_json::from_slice(
            &std::fs::read(fixture._dir.path().join("workspace.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(loaded.tasks, fixture.state.tasks);
        let listed = fixture
            .exchange(
                &launch,
                Operation::Call {
                    name: "team_tasks".into(),
                    arguments: json!({}),
                },
            )
            .unwrap();
        let id = listed["tasks"][0]["id"].as_str().unwrap();
        let moved = fixture
            .exchange(
                &launch,
                Operation::Call {
                    name: "team_task_move".into(),
                    arguments: json!({"id":id,"column":"review"}),
                },
            )
            .unwrap();
        assert_eq!(moved["column"], "review");
        let reloaded: Workspace = serde_json::from_slice(
            &std::fs::read(fixture._dir.path().join("workspace.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(reloaded.tasks[0].column, 2);
        let canonical = fixture
            .exchange(
                &launch,
                Operation::Call {
                    name: "team_task_add".into(),
                    arguments: json!({"title":"Canonical dependent","column":"doing","after":[id]}),
                },
            )
            .unwrap();
        assert_eq!(canonical["column"], "todo");
        assert_eq!(canonical["assigneePane"], 41);
        assert_eq!(canonical["waitingOn"], json!([id]));
        assert!(matches!(
            fixture.exchange(
                &launch,
                Operation::Call {
                    name: "tessel_browser".into(),
                    arguments: json!({})
                }
            ),
            Err(BackendError::UnknownTool)
        ));
        assert!(matches!(
            fixture.exchange(
                &launch,
                Operation::Call {
                    name: "tessel_task_add".into(),
                    arguments: json!({"title":"No authority", "project":999})
                }
            ),
            Err(BackendError::Failed(_))
        ));
        assert_eq!(fixture.state.tasks.len(), 2);
    }

    #[test]
    fn persistence_failure_is_a_tool_error_without_mutating_memory() {
        let mut fixture = Fixture::new();
        let launch = fixture.launch();
        // Replace only this fixture's own saved workspace with a directory.
        let saved = fixture._dir.path().join("workspace.json");
        std::fs::remove_file(&saved).unwrap();
        std::fs::create_dir(&saved).unwrap();
        assert!(matches!(
            fixture.exchange(&launch, add()),
            Err(BackendError::Failed(_))
        ));
        assert!(fixture.state.tasks.is_empty());
    }

    #[test]
    fn resolved_directory_spellings_match_but_missing_or_replaced_directories_revoke() {
        let mut fixture = Fixture::new();
        let work = fixture._dir.path().join("working");
        std::fs::create_dir(&work).unwrap();
        fixture.pane.cwd = work.join(".");
        let launch = fixture.launch();
        assert_eq!(launch.binding().cwd, dunce::canonicalize(&work).unwrap());
        fixture.pane.cwd = work.clone();
        assert!(fixture.exchange(&launch, Operation::List).is_ok());
        let (reply, worker) = request(&launch, add());
        fixture.queued();
        // Only this empty synthetic directory is removed, never the project root.
        std::fs::remove_dir(&work).unwrap();
        std::fs::write(&work, b"not a working directory").unwrap();
        assert!(!fixture.poll(&[fixture.pane.clone()]));
        assert!(reply.recv_timeout(Duration::from_secs(3)).unwrap().is_err());
        worker.join().unwrap();
        assert!(fixture.state.tasks.is_empty());
        fixture.pane.cwd = fixture._dir.path().join("never-created");
        assert!(
            fixture
                .runtime
                .begin(&fixture.pane, &fixture.state)
                .is_err()
        );
    }

    #[test]
    fn moved_closed_remote_sleeping_duplicate_and_removed_project_scopes_revoke_queued_mutations() {
        for case in 0..7 {
            let mut fixture = Fixture::new();
            let launch = fixture.launch();
            let (receiver, worker) = request(&launch, add());
            fixture.queued();
            let mut panes = vec![fixture.pane.clone()];
            match case {
                0 => panes.clear(),
                1 => panes[0].cwd = fixture.pane.cwd.join("moved"),
                2 => panes[0].project += 1,
                3 => panes[0].local = false,
                4 => panes[0].active = false,
                5 => panes.push(fixture.pane.clone()),
                _ => fixture.state.projects.clear(),
            }
            assert!(!fixture.poll(&panes));
            assert!(
                receiver
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap()
                    .is_err()
            );
            worker.join().unwrap();
            assert!(fixture.state.tasks.is_empty());
            // The persisted session still exists; it never restores lost authority.
            assert_eq!(fixture.state.sessions.len(), 1);
            assert!(launch.client().call(Operation::List).is_err());
        }
    }

    #[test]
    fn old_transport_drop_never_revokes_its_replacement_and_new_drop_revokes_pending_work() {
        let mut fixture = Fixture::new();
        let old = fixture.launch();
        let (old_reply, old_worker) = request(&old, add());
        fixture.queued();
        let new = fixture.launch();
        assert_ne!(old.binding().generation, new.binding().generation);
        assert!(
            old_reply
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .is_err()
        );
        old_worker.join().unwrap();
        drop(old);
        assert!(fixture.exchange(&new, Operation::List).is_ok());
        assert!(fixture.state.tasks.is_empty());
        let (reply, worker) = request(&new, add());
        fixture.queued();
        new.revoke();
        assert!(!fixture.poll(&[fixture.pane.clone()]));
        assert!(reply.recv_timeout(Duration::from_secs(3)).unwrap().is_err());
        worker.join().unwrap();
        assert!(fixture.state.tasks.is_empty());
        drop(new);
        let another = fixture.launch();
        assert_eq!(
            fixture
                .runtime
                .broker
                .as_ref()
                .unwrap()
                .bindings()
                .unwrap()
                .len(),
            1
        );
        drop(another);
        assert!(
            fixture
                .runtime
                .broker
                .as_ref()
                .unwrap()
                .bindings()
                .unwrap()
                .is_empty()
        );
    }
}
