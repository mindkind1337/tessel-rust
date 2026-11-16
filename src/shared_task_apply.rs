//! Commit guest task changes to the same board used by the host and its agents.
use crate::{
    share::{GuestTaskOp, SharedTask},
    workspace::{Store, Task, Workspace},
};

#[derive(Default)]
pub struct SnapshotCache {
    scope: Option<(Vec<u8>, Vec<u64>)>,
    tasks: Vec<Task>,
}
impl SnapshotCache {
    pub fn update(
        &mut self,
        state: &Workspace,
        scope: Option<(Vec<u8>, Vec<u64>)>,
    ) -> Option<Vec<SharedTask>> {
        let changed = scope.is_some() && (scope != self.scope || self.tasks != state.tasks);
        self.scope = scope;
        if changed {
            self.tasks.clone_from(&state.tasks);
            Some(snapshot(state))
        } else {
            if self.scope.is_none() {
                self.tasks.clear();
            }
            None
        }
    }
}

pub fn snapshot(state: &Workspace) -> Vec<SharedTask> {
    state
        .tasks
        .iter()
        .filter_map(|task| {
            Some(SharedTask {
                project_id: task.project,
                id: task.id.to_string(),
                title: task.title.clone(),
                column: ["todo", "doing", "review", "done"]
                    .get(task.column)?
                    .to_string(),
                guest_by: task.guest_by.clone(),
            })
        })
        .collect()
}

pub fn commit(
    state: &mut Workspace,
    store: Option<&Store>,
    project: u64,
    actor: &str,
    op: &GuestTaskOp,
) -> Result<(bool, Vec<SharedTask>), String> {
    transact(state, project, actor, op, |_, bytes| {
        store
            .ok_or("Workspace storage is unavailable.")?
            .save_task_bytes(bytes)
            .map_err(|_| "The host could not save the task board; no change was applied.".into())
    })
}

fn transact(
    state: &mut Workspace,
    project: u64,
    actor: &str,
    op: &GuestTaskOp,
    save: impl FnOnce(&Workspace, &[u8]) -> Result<(), String>,
) -> Result<(bool, Vec<SharedTask>), String> {
    if !state.projects.iter().any(|p| p.id == project) {
        return Err("Shared project is no longer open.".into());
    }
    let title_valid = |s: &str| !s.trim().is_empty() && s.chars().count() <= 300;
    let actor: String = actor.chars().filter(|c| !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{061c}')).take(40).collect();
    let mut draft = state.clone();
    let changed = match op {
        GuestTaskOp::Add { title, column } => {
            if !title_valid(title) || *column > 3 || draft.tasks.len() >= 10_000 {
                return Err("Invalid task or full task board.".into());
            }
            let id = draft.id()?;
            draft.tasks.push(Task {
                created_by: None,
                report: None,
                decision: None,
                last_waiting: Vec::new(),
                deps: Vec::new(),
                timeline: crate::task_timeline::Timeline::created(
                    crate::task_timeline::now_ms(),
                    *column,
                    None,
                ),
                id,
                project,
                title: title.trim().into(),
                column: *column,
                guest_by: Some(actor),
            });
            true
        }
        GuestTaskOp::Move { id, .. }
        | GuestTaskOp::Rename { id, .. }
        | GuestTaskOp::Delete { id } => {
            let index = draft
                .tasks
                .iter()
                .position(|t| t.id == *id && t.project == project)
                .ok_or("Task is no longer available in this shared project.")?;
            match op {
                GuestTaskOp::Move { column, .. } => {
                    if *column > 3 {
                        return Err("Invalid task column.".into());
                    }
                    let task = &mut draft.tasks[index];
                    let changed = task.timeline.transition(
                        task.column,
                        *column,
                        task.timeline.pane_id,
                        crate::task_timeline::now_ms(),
                    );
                    task.column = *column;
                    changed
                }
                GuestTaskOp::Rename { title, .. } => {
                    if draft.tasks[index].column != 0 || !title_valid(title) {
                        return Err("Only To do tasks can be renamed, with a valid title.".into());
                    }
                    let changed = draft.tasks[index].title != title.trim();
                    if changed {
                        draft.tasks[index].title = title.trim().into();
                        draft.tasks[index].guest_by = Some(actor);
                    }
                    changed
                }
                GuestTaskOp::Delete { .. } => {
                    draft.tasks.remove(index);
                    true
                }
                _ => unreachable!(),
            }
        }
    };
    if changed {
        let id = match op {
            GuestTaskOp::Add { .. } => draft.tasks.last().unwrap().id,
            GuestTaskOp::Move { id, .. }
            | GuestTaskOp::Rename { id, .. }
            | GuestTaskOp::Delete { id } => *id,
        };
        crate::task_history::record(&mut draft, id);
        let bytes = draft.prepare_task_change_from(state.task_cards_size()?)?;
        save(&draft, &bytes)?;
        *state = draft;
    }
    Ok((changed, snapshot(state)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guest_board_cache_skips_unchanged_ticks_and_republishes_scope_changes() {
        let mut state = fixture();
        let mut cache = SnapshotCache::default();
        let scope = Some((vec![1, 2, 3], vec![1]));
        assert!(cache.update(&state, None).is_none());
        assert!(cache.update(&state, scope.clone()).is_some());
        assert!(cache.update(&state, scope.clone()).is_none());
        transact(
            &mut state,
            1,
            "Guest",
            &GuestTaskOp::Add {
                title: "Request".into(),
                column: 0,
            },
            |_, _| Ok(()),
        )
        .unwrap();
        assert!(cache.update(&state, scope.clone()).is_some());
        assert!(cache.update(&state, scope.clone()).is_none());
        assert!(
            cache
                .update(&state, Some((vec![1, 2, 3], vec![2])))
                .is_some()
        );
        assert!(
            cache
                .update(&state, Some((vec![4, 5, 6], vec![2])))
                .is_some()
        );
        assert!(cache.update(&state, None).is_none());
        assert!(cache.tasks.is_empty());
        assert!(cache.update(&state, scope).is_some());
    }
    #[test]
    fn guest_board_real_store_and_exports_keep_attribution_local() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(dir.path()).unwrap();
        let mut state = fixture();
        commit(
            &mut state,
            Some(&store),
            1,
            "Synthetic Guest",
            &GuestTaskOp::Add {
                title: "Request".into(),
                column: 0,
            },
        )
        .unwrap();
        let disk: Workspace =
            serde_json::from_slice(&std::fs::read(dir.path().join("workspace.json")).unwrap())
                .unwrap();
        assert_eq!(disk.tasks, state.tasks);
        commit(
            &mut state,
            Some(&store),
            1,
            "Guest",
            &GuestTaskOp::Move { id: 3, column: 3 },
        )
        .unwrap();
        assert_eq!(state.task_history.records.len(), 1);
        #[cfg(windows)]
        {
            let synced = crate::tessel_sync::sanitize_workspace_json_for_sync(
                serde_json::to_value(&state).unwrap(),
            );
            assert!(synced["tasks"][0].get("guest_by").is_none());
            assert!(synced["tasks"][0].get("timeline").is_none());
            assert!(synced.get("task_history").is_none());
            assert_eq!(synced["tasks"][0]["title"], "Request");
        }
        let portable = dir.path().join("portable.json");
        crate::backup::export(&portable, &state).unwrap();
        let portable_json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&portable).unwrap()).unwrap();
        assert!(portable_json["workspace"].get("task_history").is_none());
        assert!(
            portable_json["workspace"]["tasks"][0]
                .get("timeline")
                .is_none()
        );
        assert!(
            crate::backup::import(&portable).unwrap().tasks[0]
                .guest_by
                .is_none()
        );
        assert_eq!(state.tasks[0].guest_by.as_deref(), Some("Synthetic Guest"));
        // Real persist failure must not consume an identifier or publish a task.
        std::fs::remove_file(dir.path().join("workspace.json")).unwrap();
        std::fs::create_dir(dir.path().join("workspace.json")).unwrap();
        assert!(
            commit(
                &mut state,
                Some(&store),
                1,
                "Guest",
                &GuestTaskOp::Add {
                    title: "Not saved".into(),
                    column: 0
                }
            )
            .is_err()
        );
        assert_eq!(state.tasks.len(), 1);
        assert_eq!(state.next_id, 4);
    }
    fn fixture() -> Workspace {
        Workspace {
            projects: vec![
                crate::workspace::Project {
                    id: 1,
                    name: "Shared".into(),
                    path: std::env::temp_dir(),
                },
                crate::workspace::Project {
                    id: 2,
                    name: "Private".into(),
                    path: std::env::temp_dir(),
                },
            ],
            next_id: 3,
            active_project: Some(2),
            ..Default::default()
        }
    }
    #[test]
    fn guest_board_transaction_saves_before_publish_without_switching_project() {
        let mut state = fixture();
        let op = GuestTaskOp::Add {
            title: "Guest request".into(),
            column: 0,
        };
        let mut saved = None;
        let (changed, cards) = transact(&mut state, 1, "Guest", &op, |draft, _| {
            saved = Some(draft.clone());
            Ok(())
        })
        .unwrap();
        assert!(changed);
        assert_eq!(state.active_project, Some(2));
        assert_eq!(saved.unwrap().tasks, state.tasks);
        assert_eq!(cards[0].id, "3");
        assert_eq!(state.tasks[0].guest_by.as_deref(), Some("Guest"));
        assert!(
            transact(
                &mut state,
                2,
                "Guest",
                &GuestTaskOp::Delete { id: 3 },
                |_, _| panic!("foreign mutation")
            )
            .is_err()
        );
        assert_eq!(state.tasks.len(), 1);
    }
    #[test]
    fn failed_guest_save_rolls_back_ids_and_board() {
        let mut state = fixture();
        assert!(
            transact(
                &mut state,
                1,
                "Guest",
                &GuestTaskOp::Add {
                    title: "Request".into(),
                    column: 0
                },
                |_, _| Err("fixture failure".into())
            )
            .is_err()
        );
        assert!(state.tasks.is_empty());
        assert_eq!(state.next_id, 3);
    }
    #[test]
    fn guest_changes_validate_and_noop_does_not_save() {
        let mut state = fixture();
        transact(
            &mut state,
            1,
            &"é".repeat(100),
            &GuestTaskOp::Add {
                title: "Request".into(),
                column: 0,
            },
            |_, _| Ok(()),
        )
        .unwrap();
        assert_eq!(
            state.tasks[0].guest_by.as_ref().unwrap().chars().count(),
            40
        );
        assert!(
            !transact(
                &mut state,
                1,
                "Guest",
                &GuestTaskOp::Move { id: 3, column: 0 },
                |_, _| panic!("noop saved")
            )
            .unwrap()
            .0
        );
        transact(
            &mut state,
            1,
            "Guest",
            &GuestTaskOp::Move { id: 3, column: 1 },
            |_, _| Ok(()),
        )
        .unwrap();
        assert!(
            transact(
                &mut state,
                1,
                "Guest",
                &GuestTaskOp::Rename {
                    id: 3,
                    title: "Rename".into()
                },
                |_, _| panic!("invalid rename saved")
            )
            .is_err()
        );
        assert!(
            transact(
                &mut state,
                1,
                "Guest",
                &GuestTaskOp::Move { id: 3, column: 4 },
                |_, _| panic!("invalid column saved")
            )
            .is_err()
        );
    }
}
