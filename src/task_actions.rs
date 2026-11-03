//! Applies board actions only within the currently selected, existing project.
use crate::{
    tasks_board_panel::{MAX_TASK_TITLE_CHARS, TASK_COLUMN_COUNT, TasksBoardAction},
    workspace::{Task, Workspace},
};

pub fn apply(
    state: &mut Workspace,
    project: u64,
    action: TasksBoardAction,
) -> Result<bool, String> {
    if state.active_project != Some(project) || !state.projects.iter().any(|p| p.id == project) {
        return Ok(false);
    }
    match action {
        TasksBoardAction::Add(title) => {
            let title: String = title.trim().chars().take(MAX_TASK_TITLE_CHARS).collect();
            if title.is_empty() {
                return Ok(false);
            }
            if state.tasks.len() >= 10_000 {
                return Err("Task board is full.".into());
            }
            let id = state.id()?;
            state.tasks.push(Task {
                id,
                project,
                title,
                column: 0,
            });
            Ok(true)
        }
        TasksBoardAction::Move { id, column } if column < TASK_COLUMN_COUNT => {
            if let Some(task) = state
                .tasks
                .iter_mut()
                .find(|task| task.id == id && task.project == project)
                && task.column != column
            {
                task.column = column;
                return Ok(true);
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Project;
    fn workspace() -> Workspace {
        Workspace {
            projects: vec![
                Project {
                    id: 1,
                    name: "One".into(),
                    path: "one".into(),
                },
                Project {
                    id: 2,
                    name: "Two".into(),
                    path: "two".into(),
                },
            ],
            active_project: Some(1),
            next_id: 3,
            ..Default::default()
        }
    }
    #[test]
    fn stale_project_and_foreign_card_cannot_be_mutated() {
        let mut state = workspace();
        assert!(apply(&mut state, 1, TasksBoardAction::Add("  first  ".into())).unwrap());
        assert_eq!(state.tasks[0].title, "first");
        let id = state.tasks[0].id;
        state.active_project = Some(2);
        assert!(!apply(&mut state, 1, TasksBoardAction::Move { id, column: 3 }).unwrap());
        assert!(!apply(&mut state, 2, TasksBoardAction::Move { id, column: 3 }).unwrap());
        assert_eq!(state.tasks[0].column, 0);
        state.active_project = Some(1);
        assert!(!apply(&mut state, 1, TasksBoardAction::Move { id, column: 4 }).unwrap());
        assert!(apply(&mut state, 1, TasksBoardAction::Move { id, column: 3 }).unwrap());
        assert!(!apply(&mut state, 1, TasksBoardAction::Move { id, column: 3 }).unwrap());
    }
    #[test]
    fn full_board_refuses_add_without_consuming_an_id_or_changing_state() {
        let mut state = workspace();
        state.tasks = (0..10_000)
            .map(|n| Task {
                id: n + 3,
                project: 1,
                title: "Existing".into(),
                column: 0,
            })
            .collect();
        state.next_id = 10_003;
        assert!(apply(&mut state, 1, TasksBoardAction::Add("Next".into())).is_err());
        assert_eq!(state.tasks.len(), 10_000);
        assert_eq!(state.next_id, 10_003);
    }
}
