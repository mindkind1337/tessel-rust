//! Transient, project-scoped native board interactions. No persistence or side effects.
use crate::{
    tasks_board_panel::{MAX_TASK_TITLE_CHARS, TasksBoardAction},
    workspace::Task,
};
use std::collections::{BTreeSet, HashMap};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    pub project: u64,
    pub generation: u64,
}
#[derive(Clone, Debug)]
pub struct Edit {
    pub id: u64,
    pub serial: u64,
    pub draft: String,
}
#[derive(Clone, Debug, Default)]
pub struct State {
    pub scope: Option<Scope>,
    generation: u64,
    serial: u64,
    pub selecting: bool,
    pub picked: BTreeSet<u64>,
    pub open_reports: BTreeSet<u64>,
    pub edit: Option<Edit>,
    pub pending_delete: Option<(Vec<u64>, bool)>,
}
#[derive(Clone, Debug)]
pub enum Command {
    ToggleReport(u64),
    AnswerDecision {
        id: u64,
        request: u64,
        answer: String,
    },
    Assign {
        id: u64,
        pane: Option<u64>,
    },
    ShowAgent {
        id: u64,
        pane: u64,
    },
    BeginRename(u64),
    Edit {
        serial: u64,
        base: String,
        text: String,
    },
    Save {
        serial: u64,
        base: String,
    },
    CancelEdit {
        serial: u64,
    },
    ToggleSelection,
    TogglePick(u64),
    ToggleAll,
    RequestDelete,
    RequestDeleteOne(u64),
    ConfirmDelete,
    CancelDelete,
    Step {
        id: u64,
        delta: i8,
    },
    Move {
        id: u64,
        column: usize,
    },
}
impl State {
    pub fn reconcile(&mut self, project: Option<u64>, tasks: &[Task]) {
        if self.scope.map(|s| s.project) != project {
            self.generation = self
                .generation
                .checked_add(1)
                .expect("board identity exhausted");
            self.scope = project.map(|project| Scope {
                project,
                generation: self.generation,
            });
            self.selecting = false;
            self.picked.clear();
            self.open_reports.clear();
            self.edit = None;
            self.pending_delete = None;
        }
        // Index once: Select All on a full board must not rescan 10,000 tasks per id.
        let cards: HashMap<_, _> = tasks
            .iter()
            .filter(|t| Some(t.project) == project)
            .map(|t| (t.id, t))
            .collect();
        self.picked
            .retain(|id| cards.get(id).is_some_and(|t| t.column == 3));
        self.open_reports
            .retain(|id| cards.get(id).is_some_and(|t| t.report.is_some()));
        if !cards.values().any(|t| t.column == 3) {
            self.selecting = false;
        }
        if let Some((ids, only_done)) = &mut self.pending_delete {
            ids.retain(|id| cards.get(id).is_some_and(|t| !*only_done || t.column == 3));
            if ids.is_empty() {
                self.pending_delete = None;
            }
        }
        if self
            .edit
            .as_ref()
            .is_some_and(|e| !cards.get(&e.id).is_some_and(|t| t.column == 0))
        {
            self.edit = None;
        }
    }

    pub fn apply(
        &mut self,
        scope: Scope,
        command: Command,
        tasks: &[Task],
    ) -> Option<TasksBoardAction> {
        if self.scope != Some(scope) {
            return None;
        }
        let cards: HashMap<_, _> = tasks
            .iter()
            .filter(|t| t.project == scope.project)
            .map(|t| (t.id, t))
            .collect();
        let find = |id| cards.get(&id).copied();
        if self.pending_delete.is_some()
            && !matches!(command, Command::ConfirmDelete | Command::CancelDelete)
        {
            return None;
        }
        match command {
            Command::ToggleReport(id) => {
                if find(id).is_some_and(|t| t.report.is_some()) && !self.open_reports.remove(&id) {
                    self.open_reports.insert(id);
                }
            }
            Command::AnswerDecision {
                id,
                request,
                answer,
            } => {
                if find(id).is_some_and(|task| {
                    task.decision
                        .as_ref()
                        .is_some_and(|d| d.request == request && d.pending())
                }) {
                    return Some(TasksBoardAction::AnswerDecision {
                        id,
                        request,
                        answer,
                    });
                }
            }
            Command::Assign { id, pane } => {
                if find(id).is_some_and(|t| t.column <= 1) && self.edit.is_none() {
                    return Some(TasksBoardAction::Assign { id, pane });
                }
            }
            Command::ShowAgent { id, pane } => {
                if find(id)
                    .is_some_and(|t| matches!(t.column, 1 | 2) && t.timeline.pane_id == Some(pane))
                {
                    return Some(TasksBoardAction::ShowAgent { id, pane });
                }
            }
            Command::BeginRename(id) => {
                if let Some(t) = find(id).filter(|t| t.column == 0) {
                    self.serial = self.serial.checked_add(1).expect("edit identity exhausted");
                    self.edit = Some(Edit {
                        id,
                        serial: self.serial,
                        draft: t.title.clone(),
                    });
                }
            }
            Command::Edit { serial, base, text } => {
                if text.chars().count() <= MAX_TASK_TITLE_CHARS
                    && let Some(e) = self
                        .edit
                        .as_mut()
                        .filter(|e| e.serial == serial && e.draft == base)
                {
                    e.draft = text;
                }
            }
            Command::Save { serial, base } => {
                if self
                    .edit
                    .as_ref()
                    .is_some_and(|e| e.serial == serial && e.draft == base)
                {
                    let e = self.edit.take().unwrap();
                    if !base.trim().is_empty() && find(e.id).is_some_and(|t| t.column == 0) {
                        return Some(TasksBoardAction::Rename {
                            id: e.id,
                            title: base,
                        });
                    }
                }
            }
            Command::CancelEdit { serial } => {
                if self.edit.as_ref().is_some_and(|e| e.serial == serial) {
                    self.edit = None;
                }
            }
            Command::ToggleSelection => {
                self.selecting = !self.selecting;
                self.picked.clear();
            }
            Command::TogglePick(id) => {
                if self.selecting
                    && find(id).is_some_and(|t| t.column == 3)
                    && !self.picked.remove(&id)
                {
                    self.picked.insert(id);
                }
            }
            Command::ToggleAll => {
                if self.selecting {
                    let done: BTreeSet<_> = tasks
                        .iter()
                        .filter(|t| t.project == scope.project && t.column == 3)
                        .map(|t| t.id)
                        .collect();
                    self.picked = if done == self.picked {
                        BTreeSet::new()
                    } else {
                        done
                    };
                }
            }
            Command::RequestDelete => {
                if self.selecting && !self.picked.is_empty() {
                    self.pending_delete = Some((self.picked.iter().copied().collect(), true));
                }
            }
            Command::RequestDeleteOne(id) => {
                if find(id).is_some() {
                    self.pending_delete = Some((vec![id], false));
                }
            }
            Command::CancelDelete => self.pending_delete = None,
            Command::ConfirmDelete => {
                if let Some((ids, only_done)) = self.pending_delete.take() {
                    let ids: Vec<_> = ids
                        .into_iter()
                        .filter(|id| find(*id).is_some_and(|t| !only_done || t.column == 3))
                        .collect();
                    if only_done {
                        self.selecting = false;
                        self.picked.clear();
                    }
                    if !ids.is_empty() {
                        return Some(TasksBoardAction::DeleteMany(ids));
                    }
                }
            }
            Command::Step { id, delta } => {
                if matches!(delta, -1 | 1)
                    && self.edit.as_ref().is_none_or(|e| e.id != id)
                    && let Some(t) = find(id)
                    && let Some(column) = t.column.checked_add_signed(isize::from(delta))
                    && column < 4
                {
                    return Some(TasksBoardAction::Move { id, column });
                }
            }
            Command::Move { id, column } => {
                if find(id).is_some() && column < 4 && self.edit.as_ref().is_none_or(|e| e.id != id)
                {
                    return Some(TasksBoardAction::Move { id, column });
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tasks() -> Vec<Task> {
        vec![
            Task {
                preparation: None,
                created_by: None,
                report: None,
                decision: None,
                last_waiting: Vec::new(),
                deps: Vec::new(),
                timeline: Default::default(),
                guest_by: None,
                id: 1,
                project: 1,
                title: "To do".into(),
                column: 0,
            },
            Task {
                preparation: None,
                created_by: None,
                report: None,
                decision: None,
                last_waiting: Vec::new(),
                deps: Vec::new(),
                timeline: Default::default(),
                guest_by: None,
                id: 2,
                project: 1,
                title: "Done".into(),
                column: 3,
            },
            Task {
                preparation: None,
                created_by: None,
                report: None,
                decision: None,
                last_waiting: Vec::new(),
                deps: Vec::new(),
                timeline: Default::default(),
                guest_by: None,
                id: 3,
                project: 1,
                title: "Done too".into(),
                column: 3,
            },
            Task {
                preparation: None,
                created_by: None,
                report: None,
                decision: None,
                last_waiting: Vec::new(),
                deps: Vec::new(),
                timeline: Default::default(),
                guest_by: None,
                id: 4,
                project: 2,
                title: "Foreign".into(),
                column: 3,
            },
        ]
    }
    #[test]
    fn move_and_step_revalidate_live_card_scope_bounds_and_editing() {
        let mut tasks = tasks();
        let mut state = State::default();
        state.reconcile(Some(1), &tasks);
        let scope = state.scope.unwrap();
        assert_eq!(
            state.apply(scope, Command::Step { id: 1, delta: 1 }, &tasks),
            Some(TasksBoardAction::Move { id: 1, column: 1 })
        );
        assert_eq!(
            state.apply(scope, Command::Step { id: 1, delta: -1 }, &tasks),
            None
        );
        tasks[0].column = 2;
        assert_eq!(
            state.apply(scope, Command::Step { id: 1, delta: -1 }, &tasks),
            Some(TasksBoardAction::Move { id: 1, column: 1 })
        );
        for command in [
            Command::Step { id: 4, delta: -1 },
            Command::Step { id: 1, delta: 2 },
            Command::Step { id: 2, delta: 1 },
            Command::Move { id: 4, column: 0 },
            Command::Move { id: 1, column: 4 },
        ] {
            assert_eq!(state.apply(scope, command, &tasks), None);
        }
        tasks[0].column = 0;
        state.apply(scope, Command::BeginRename(1), &tasks);
        assert_eq!(
            state.apply(scope, Command::Move { id: 1, column: 1 }, &tasks),
            None
        );
        assert_eq!(
            state.apply(scope, Command::Step { id: 1, delta: 1 }, &tasks),
            None
        );
        state.edit = None;
        tasks[0].project = 2;
        assert_eq!(
            state.apply(scope, Command::Move { id: 1, column: 1 }, &tasks),
            None
        );
        state.reconcile(None, &tasks);
        state.reconcile(Some(1), &tasks);
        assert_eq!(
            state.apply(scope, Command::Move { id: 2, column: 1 }, &tasks),
            None
        );
    }
    #[test]
    fn rename_requires_exact_edit_and_unstarted_card() {
        let mut tasks = tasks();
        let mut state = State::default();
        state.reconcile(Some(1), &tasks);
        let scope = state.scope.unwrap();
        state.apply(scope, Command::BeginRename(2), &tasks);
        assert!(state.edit.is_none());
        state.apply(scope, Command::BeginRename(1), &tasks);
        let serial = state.edit.as_ref().unwrap().serial;
        state.apply(
            scope,
            Command::Edit {
                serial,
                base: "wrong".into(),
                text: "replacement".into(),
            },
            &tasks,
        );
        assert_eq!(state.edit.as_ref().unwrap().draft, "To do");
        state.apply(
            scope,
            Command::Edit {
                serial,
                base: "To do".into(),
                text: "Québec 😀".into(),
            },
            &tasks,
        );
        assert!(matches!(
            state.apply(
                scope,
                Command::Save {
                    serial,
                    base: "Québec 😀".into()
                },
                &tasks
            ),
            Some(TasksBoardAction::Rename { id: 1, .. })
        ));
        state.apply(scope, Command::BeginRename(1), &tasks);
        let serial = state.edit.as_ref().unwrap().serial;
        tasks[0].column = 1;
        state.reconcile(Some(1), &tasks);
        assert!(state.edit.is_none());
        assert!(
            state
                .apply(
                    scope,
                    Command::Save {
                        serial,
                        base: "To do".into()
                    },
                    &tasks
                )
                .is_none()
        );
    }
    #[test]
    fn selection_confirmation_rechecks_done_project_and_stale_generations() {
        let mut tasks = tasks();
        let mut state = State::default();
        state.reconcile(Some(1), &tasks);
        let scope = state.scope.unwrap();
        state.apply(scope, Command::ToggleSelection, &tasks);
        state.apply(scope, Command::ToggleAll, &tasks);
        assert_eq!(state.picked, BTreeSet::from([2, 3]));
        state.apply(scope, Command::RequestDelete, &tasks);
        assert!(
            state
                .apply(scope, Command::Move { id: 1, column: 3 }, &tasks)
                .is_none()
        );
        state.apply(scope, Command::CancelDelete, &tasks);
        assert_eq!(state.picked.len(), 2);
        state.apply(scope, Command::RequestDelete, &tasks);
        tasks[1].column = 1;
        state.reconcile(Some(1), &tasks);
        assert_eq!(
            state.apply(scope, Command::ConfirmDelete, &tasks),
            Some(TasksBoardAction::DeleteMany(vec![3]))
        );
        assert!(!state.selecting && state.picked.is_empty());
        state.reconcile(Some(2), &tasks);
        state.reconcile(Some(1), &tasks);
        assert!(
            state
                .apply(scope, Command::RequestDeleteOne(1), &tasks)
                .is_none()
        );
        assert!(state.pending_delete.is_none());
        let scope = state.scope.unwrap();
        state.apply(scope, Command::RequestDeleteOne(1), &tasks);
        tasks.retain(|t| t.id != 1);
        state.reconcile(Some(1), &tasks);
        assert!(state.apply(scope, Command::ConfirmDelete, &tasks).is_none());
    }
}
