//! Applies board actions only within the currently selected, existing project.
use std::collections::HashSet;

use crate::{
    tasks_board_panel::{MAX_TASK_TITLE_CHARS, TASK_COLUMN_COUNT, TasksBoardAction},
    workspace::{Task, Workspace},
};

pub fn commit(
    state: &mut Workspace,
    store: Option<&crate::workspace::Store>,
    project: u64,
    action: TasksBoardAction,
) -> Result<bool, String> {
    let mut draft = state.clone();
    let changed = apply_inner(&mut draft, project, action)?;
    if changed {
        let bytes = draft.prepare_task_change_from(state.task_cards_size()?)?;
        store
            .ok_or("Workspace storage is unavailable.")?
            .save_task_bytes(&bytes)?;
        *state = draft;
    }
    Ok(changed)
}

#[cfg(test)]
pub fn apply(
    state: &mut Workspace,
    project: u64,
    action: TasksBoardAction,
) -> Result<bool, String> {
    let mut draft = state.clone();
    let changed = apply_inner(&mut draft, project, action)?;
    if changed {
        draft.prepare_task_change_from(state.task_cards_size()?)?;
        *state = draft;
    }
    Ok(changed)
}
fn apply_inner(
    state: &mut Workspace,
    project: u64,
    action: TasksBoardAction,
) -> Result<bool, String> {
    if state.active_project != Some(project) || !state.projects.iter().any(|p| p.id == project) {
        return Ok(false);
    }
    match action {
        TasksBoardAction::Assign { id, pane } => {
            crate::task_history::assign(state, project, id, pane, crate::task_timeline::now_ms())
        }
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
                deps: Vec::new(),
                timeline: crate::task_timeline::Timeline::created(
                    crate::task_timeline::now_ms(),
                    0,
                    None,
                ),
                guest_by: None,
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
            {
                let changed = task.timeline.transition(
                    task.column,
                    column,
                    task.timeline.pane_id,
                    crate::task_timeline::now_ms(),
                );
                task.column = column;
                if changed {
                    crate::task_history::record(state, id);
                }
                return Ok(changed);
            }
            Ok(false)
        }
        TasksBoardAction::Rename { id, title } => {
            let title: String = title.trim().chars().take(MAX_TASK_TITLE_CHARS).collect();
            if title.is_empty() {
                return Ok(false);
            }
            if let Some(task) = state
                .tasks
                .iter_mut()
                .find(|task| task.id == id && task.project == project)
                && task.column == 0
                && task.title != title
            {
                task.title = title;
                return Ok(true);
            }
            Ok(false)
        }
        TasksBoardAction::Delete(id) => {
            let before = state.tasks.len();
            state
                .tasks
                .retain(|task| !(task.id == id && task.project == project));
            Ok(state.tasks.len() != before)
        }
        TasksBoardAction::DeleteMany(ids) => {
            if ids.is_empty() {
                return Ok(false);
            }
            let target_ids: HashSet<u64> = ids.into_iter().collect();
            let before = state.tasks.len();
            state
                .tasks
                .retain(|task| !(task.project == project && target_ids.contains(&task.id)));
            Ok(state.tasks.len() != before)
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
                    path: std::env::temp_dir().join("one"),
                },
                Project {
                    id: 2,
                    name: "Two".into(),
                    path: std::env::temp_dir().join("two"),
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
                deps: Vec::new(),
                timeline: Default::default(),
                guest_by: None,
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
    #[test]
    fn rename_delete_and_delete_many_are_scoped_to_active_project() {
        let mut state = workspace();
        state.next_id = 20;
        state.tasks = vec![
            Task {
                deps: Vec::new(),
                timeline: Default::default(),
                guest_by: None,
                id: 10,
                project: 1,
                title: "Alpha".into(),
                column: 0,
            },
            Task {
                deps: Vec::new(),
                timeline: Default::default(),
                guest_by: None,
                id: 11,
                project: 1,
                title: "Beta".into(),
                column: 3,
            },
            Task {
                deps: Vec::new(),
                timeline: Default::default(),
                guest_by: None,
                id: 12,
                project: 2,
                title: "Foreign".into(),
                column: 3,
            },
        ];
        assert!(
            apply(
                &mut state,
                1,
                TasksBoardAction::Rename {
                    id: 10,
                    title: "  Alpha v2  ".into(),
                }
            )
            .unwrap()
        );
        assert_eq!(state.tasks[0].title, "Alpha v2");
        assert!(
            !apply(
                &mut state,
                1,
                TasksBoardAction::Rename {
                    id: 10,
                    title: "   ".into(),
                }
            )
            .unwrap()
        );
        assert!(
            !apply(
                &mut state,
                1,
                TasksBoardAction::Rename {
                    id: 12,
                    title: "Hijack".into(),
                }
            )
            .unwrap()
        );
        assert!(apply(&mut state, 1, TasksBoardAction::DeleteMany(vec![11, 12])).unwrap());
        assert_eq!(state.tasks.len(), 2);
        assert!(state.tasks.iter().any(|t| t.id == 12));
        assert!(apply(&mut state, 1, TasksBoardAction::Delete(10)).unwrap());
        assert_eq!(state.tasks.len(), 1);
        assert_eq!(state.tasks[0].id, 12);
    }
}
