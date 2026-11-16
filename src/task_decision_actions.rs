//! Explicit UI-only decision commit. No agent tool can supply a user answer.
use crate::{
    task_decision::Answer,
    teams_relay::TeamsRelayHub,
    workspace::{Store, Workspace},
};
pub fn answer(
    state: &mut Workspace,
    store: Option<&Store>,
    hub: &mut TeamsRelayHub,
    project: u64,
    task_id: u64,
    request: u64,
    text: &str,
) -> Result<bool, String> {
    if state.active_project != Some(project) {
        return Err("Project changed before the answer.".into());
    }
    let text = crate::agent_names::js_trim(text);
    if text.is_empty() || text.encode_utf16().count() > 500 {
        return Err("Answer must contain 1 to 500 UTF-16 code units.".into());
    }
    let mut draft = state.clone();
    let task = draft
        .tasks
        .iter_mut()
        .find(|task| task.id == task_id && task.project == project)
        .ok_or("Task no longer exists")?;
    let decision = task
        .decision
        .as_mut()
        .filter(|d| d.request == request && d.pending())
        .ok_or("The question changed or was already answered.")?;
    decision.answer = Some(Answer {
        text: text.into(),
        at_ms: crate::task_timeline::now_ms(),
    });
    let bytes = draft.prepare_task_change_from(state.task_cards_size()?)?;
    store
        .ok_or("Workspace storage is unavailable.")?
        .save_task_bytes(&bytes)?;
    *state = draft;
    hub.notify_task_answer(state, task_id, crate::teams_relay::now_ms());
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn decisions_survive_restart_block_agents_and_reject_stale_or_failed_answers() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let project = state.add_project(dir.path()).unwrap();
        state.sessions.push(
            serde_json::from_value(json!({"id":7,"project":project,"title":"Synthetic",
            "launch":"CodexChat","cwd":state.projects[0].path}))
            .unwrap(),
        );
        state.teams.push(crate::teams_relay::SavedTeam {
            id: "team".into(),
            project,
            name: "Fixture".into(),
            lead_pane: Some(7),
            members: vec![7],
        });
        let mut hub = TeamsRelayHub::default();
        let call =
            |state: &mut Workspace, hub: &mut TeamsRelayHub, name, args: &serde_json::Value| {
                crate::agent_tools::execute_persisted_for_pane(
                    state,
                    Some(&store),
                    hub,
                    7,
                    project,
                    name,
                    args,
                )
            };
        let (card, _) = call(
            &mut state,
            &mut hub,
            "team_task_add",
            &json!({"title":"Decision card"}),
        )
        .unwrap();
        let id = card["id"].as_str().unwrap().parse().unwrap();
        let (asked, _) = call(
            &mut state,
            &mut hub,
            "team_task_gate",
            &json!({"id":card["id"],"question":" First\nquestion ","options":["Yes","No"]}),
        )
        .unwrap();
        let old = asked["gate_id"].as_u64().unwrap();
        state = serde_json::from_slice(&std::fs::read(dir.path().join("workspace.json")).unwrap())
            .unwrap();
        hub = TeamsRelayHub::default();
        assert!(hub.task_has_pending_gate(&state, project, id));
        assert!(
            call(
                &mut state,
                &mut hub,
                "team_task_move",
                &json!({"id":card["id"],"column":"doing"})
            )
            .is_err()
        );
        let (report, _) = call(
            &mut state,
            &mut hub,
            "team_task_done",
            &json!({"id":card["id"],"summary":"Retained"}),
        )
        .unwrap();
        assert_eq!(report["blocked"], true);
        let (next, _) = call(
            &mut state,
            &mut hub,
            "team_task_gate",
            &json!({"task_id":id,"note":"Replacement"}),
        )
        .unwrap();
        let request = next["gate_id"].as_u64().unwrap();
        assert_ne!(old, request);
        let before = serde_json::to_value(&state).unwrap();
        let messages = hub.messages.clone();
        assert!(answer(&mut state, Some(&store), &mut hub, project, id, old, "Yes").is_err());
        assert!(answer(&mut state, None, &mut hub, project, id, request, "Yes").is_err());
        for text in ["".to_owned(), "😀".repeat(251)] {
            assert!(
                answer(
                    &mut state,
                    Some(&store),
                    &mut hub,
                    project,
                    id,
                    request,
                    &text
                )
                .is_err()
            );
        }
        for args in [
            json!({"id":card["id"],"question":"Fake","answer":"Yes"}),
            json!({"id":card["id"],"question":"X","project":project}),
            json!({"id":card["id"],"question":"X","options":vec!["x";7]}),
            json!({"id":card["id"],"question":"😀".repeat(251)}),
            json!({"id":card["id"],"question":"X","options":["😀".repeat(41)]}),
        ] {
            assert!(call(&mut state, &mut hub, "team_task_gate", &args).is_err());
        }
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
        assert_eq!(hub.messages, messages);
        let path = dir.path().join("workspace.json");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(
            answer(
                &mut state,
                Some(&store),
                &mut hub,
                project,
                id,
                request,
                "Yes"
            )
            .is_err()
        );
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
        assert_eq!(hub.messages, messages);
        std::fs::remove_dir(&path).unwrap();
        answer(
            &mut state,
            Some(&store),
            &mut hub,
            project,
            id,
            request,
            "Yes",
        )
        .unwrap();
        assert!(!hub.task_has_pending_gate(&state, project, id));
        assert_eq!(hub.messages.len(), messages.len() + 1);
        assert!(
            answer(
                &mut state,
                Some(&store),
                &mut hub,
                project,
                id,
                request,
                "No"
            )
            .is_err()
        );
        let loaded: Workspace = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(loaded.tasks, state.tasks);
        call(
            &mut state,
            &mut hub,
            "team_task_move",
            &json!({"id":card["id"],"column":"done"}),
        )
        .unwrap();
    }
}
