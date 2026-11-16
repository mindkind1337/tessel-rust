//! Persist observed dependency transitions before notifying an assigned teammate.
use crate::{
    task_dependencies::Index,
    teams_relay::TeamsRelayHub,
    workspace::{Store, Workspace},
};

pub fn poll(
    state: &mut Workspace,
    store: Option<&Store>,
    hub: &mut TeamsRelayHub,
    now: u64,
) -> Result<bool, String> {
    if state
        .tasks
        .iter()
        .all(|task| task.deps.is_empty() && task.last_waiting.is_empty())
    {
        return Ok(false);
    }
    let index = Index::new(&state.tasks);
    let mut changes: std::collections::HashMap<_, _> = state
        .tasks
        .iter()
        .filter_map(|task| {
            let waiting = index.waiting_on(task);
            (waiting != task.last_waiting).then_some((task.id, waiting))
        })
        .collect();
    if changes.is_empty() {
        return Ok(false);
    }
    let mut draft = state.clone();
    let mut ready = Vec::new();
    for task in &mut draft.tasks {
        let Some(waiting) = changes.remove(&task.id) else {
            continue;
        };
        if !task.last_waiting.is_empty()
            && waiting.is_empty()
            && !task.deps.is_empty()
            && task.column == 0
        {
            ready.push(task.id);
        }
        task.last_waiting = waiting;
    }
    let bytes = draft.prepare_task_change_from(state.task_cards_size()?)?;
    store
        .ok_or("Workspace storage is unavailable.")?
        .save_task_bytes(&bytes)?;
    *state = draft;
    for id in ready {
        hub.notify_task_ready(state, id, now);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    struct Fixture {
        dir: tempfile::TempDir,
        store: Store,
        state: Workspace,
        hub: TeamsRelayHub,
        project: u64,
        prerequisite: String,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let (store, mut state) = Store::open(dir.path()).unwrap();
            let project = state.add_project(dir.path()).unwrap();
            state.sessions.push(
                serde_json::from_value(json!({"id":41,"project":project,
                "title":"Synthetic","launch":"CodexChat","cwd":state.projects[0].path}))
                .unwrap(),
            );
            state.teams.push(crate::teams_relay::SavedTeam {
                id: "fixture-team".into(),
                project,
                name: "Fixture".into(),
                lead_pane: Some(41),
                members: vec![41],
            });
            let mut hub = TeamsRelayHub::default();
            let (first, _) = crate::agent_tools::execute_persisted_for_pane(
                &mut state,
                Some(&store),
                &mut hub,
                41,
                project,
                "team_task_add",
                &json!({"title":"Prerequisite"}),
            )
            .unwrap();
            crate::agent_tools::execute_persisted_for_pane(
                &mut state,
                Some(&store),
                &mut hub,
                41,
                project,
                "team_task_add",
                &json!({"title":"Dependent","after":[first["id"]]}),
            )
            .unwrap();
            Self {
                dir,
                store,
                state,
                hub,
                project,
                prerequisite: first["id"].as_str().unwrap().into(),
            }
        }
        fn move_prerequisite(&mut self, column: &str) {
            crate::agent_tools::execute_persisted_for_pane(
                &mut self.state,
                Some(&self.store),
                &mut self.hub,
                41,
                self.project,
                "team_task_move",
                &json!({"id":self.prerequisite,"column":column}),
            )
            .unwrap();
        }
        fn poll(&mut self) -> Result<bool, String> {
            poll(&mut self.state, Some(&self.store), &mut self.hub, 100)
        }
    }
    #[test]
    fn readiness_notifies_once_per_transition_and_survives_reload() {
        let mut f = Fixture::new();
        assert!(!f.poll().unwrap());
        f.state.tasks[1].guest_by = Some("Synthetic guest".into());
        f.move_prerequisite("done");
        assert!(f.poll().unwrap());
        let messages = f.hub.claim_team_inbox(41);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].text.contains("can start now"));
        assert!(messages[0].text.contains("Live Share guest, not the user"));
        assert!(!f.poll().unwrap());
        assert!(f.hub.claim_team_inbox(41).is_empty());
        f.state =
            serde_json::from_slice(&std::fs::read(f.dir.path().join("workspace.json")).unwrap())
                .unwrap();
        f.hub = TeamsRelayHub::default();
        assert!(!f.poll().unwrap());
        assert!(f.hub.messages.is_empty());
        f.move_prerequisite("todo");
        assert!(f.poll().unwrap());
        f.move_prerequisite("done");
        assert!(f.poll().unwrap());
        assert_eq!(f.hub.messages.len(), 1);
    }
    #[test]
    fn readiness_save_failure_does_not_consume_or_deliver_transition() {
        let mut f = Fixture::new();
        f.move_prerequisite("done");
        let before = f.state.tasks.clone();
        let path = f.dir.path().join("workspace.json");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(f.poll().is_err());
        assert_eq!(f.state.tasks, before);
        assert!(f.hub.messages.is_empty());
        std::fs::remove_dir(&path).unwrap();
        assert!(f.poll().unwrap());
        assert_eq!(f.hub.messages.len(), 1);
        assert!(!f.poll().unwrap());
    }
    #[test]
    fn readiness_requires_todo_assignment_same_project_team_and_existing_dependencies() {
        for mode in 0..5 {
            let mut f = Fixture::new();
            match mode {
                0 => {
                    f.state.tasks[1].column = 2;
                }
                1 => {
                    f.state.tasks[1].timeline.pane_id = None;
                }
                2 => {
                    f.state.teams.clear();
                }
                3 => {
                    f.state.tasks[1].timeline.pane_id = Some(99);
                }
                _ => {
                    f.state.tasks.remove(0);
                }
            }
            if mode != 4 {
                f.move_prerequisite("done");
            }
            f.poll().unwrap();
            assert!(f.hub.messages.is_empty(), "mode {mode}");
            if mode == 4 {
                assert!(!f.state.tasks[0].last_waiting.is_empty());
            }
        }
    }
    #[test]
    fn readiness_cannot_notify_a_foreign_session_or_team() {
        for foreign_session in [false, true] {
            let mut f = Fixture::new();
            let elsewhere = tempfile::tempdir().unwrap();
            let other = f.state.add_project(elsewhere.path()).unwrap();
            if foreign_session {
                f.state.sessions[0].project = Some(other);
                f.state.sessions[0].cwd = f
                    .state
                    .projects
                    .iter()
                    .find(|p| p.id == other)
                    .unwrap()
                    .path
                    .clone();
            } else {
                f.state.teams[0].project = other;
            }
            f.move_prerequisite("done");
            f.poll().unwrap();
            assert!(f.hub.messages.is_empty());
        }
    }
}
