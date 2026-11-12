//! Project-local choices and stale-action validation for task assignees.
use crate::{launch::Launch, workspace::Workspace};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: u64,
    pub label: String,
    pub provider: String,
}
pub fn choices(state: &Workspace, project: u64) -> Vec<Choice> {
    from_sessions(
        project,
        state.sessions.iter().map(|s| {
            (
                s.id,
                s.project,
                &s.launch,
                s.pane_name.as_deref(),
                s.title.as_str(),
            )
        }),
    )
}
pub fn from_sessions<'a>(
    project: u64,
    sessions: impl Iterator<Item = (u64, Option<u64>, &'a Launch, Option<&'a str>, &'a str)>,
) -> Vec<Choice> {
    sessions
        .filter_map(|(id, pid, launch, name, title)| {
            if pid != Some(project) {
                return None;
            }
            let provider = match launch {
                Launch::Agent(name) => name.as_str(),
                Launch::CodexChat => "Codex",
                Launch::ClaudeChat => "Claude",
                _ => return None,
            };
            let label = name.filter(|v| !v.trim().is_empty()).unwrap_or(title);
            Some(Choice {
                id,
                label: if label.trim().is_empty() {
                    provider.to_owned()
                } else {
                    label.to_owned()
                },
                provider: provider.to_owned(),
            })
        })
        .collect()
}
pub fn can_focus(state: &Workspace, project: u64, id: u64, pane: u64) -> bool {
    state.active_project == Some(project)
        && state.tasks.iter().any(|t| {
            t.project == project
                && t.id == id
                && matches!(t.column, 1 | 2)
                && t.timeline.pane_id == Some(pane)
        })
        && choices(state, project).iter().any(|p| p.id == pane)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_agent_choices_and_focus_never_escape_current_project_or_assignment() {
        let mut state = Workspace::default();
        state.active_project = Some(1);
        for (id, project, launch, title) in [
            (7, 1, "CodexChat", "Worker"),
            (8, 2, "ClaudeChat", "Other"),
            (9, 1, "Shell", "Shell"),
        ] {
            state.sessions.push(serde_json::from_value(serde_json::json!({"id":id,"project":project,"launch":launch,"title":title,"cwd":std::env::temp_dir()})).unwrap());
        }
        state.tasks.push(crate::workspace::Task {
            id: 2,
            project: 1,
            title: "Task".into(),
            column: 1,
            guest_by: None,
            timeline: crate::task_timeline::Timeline::created(100, 1, Some(7)),
        });
        assert_eq!(
            choices(&state, 1).iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![7]
        );
        assert!(can_focus(&state, 1, 2, 7));
        assert!(!can_focus(&state, 1, 2, 8));
        state.tasks[0].column = 3;
        assert!(!can_focus(&state, 1, 2, 7));
        state.tasks[0].column = 2;
        state.sessions.retain(|s| s.id != 7);
        assert!(!can_focus(&state, 1, 2, 7));
        assert_eq!(state.tasks[0].timeline.pane_id, Some(7));
    }
}
