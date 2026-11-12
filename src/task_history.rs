//! Local history updated in the same workspace transaction as each card.
use crate::{
    side_panels_model::{MAX_TASK_HISTORY, TaskHistoryRecord, TaskHistoryStore},
    workspace::Workspace,
};
use std::{collections::HashSet, io::Read, path::Path};

pub fn is_empty(value: &TaskHistoryStore) -> bool {
    value.records.is_empty() && value.next_id == 0
}
pub fn not_migrated(value: &bool) -> bool {
    !value
}
pub fn clear(state: &mut Workspace, store: Option<&crate::workspace::Store>) -> Result<(), String> {
    let mut draft = state.clone();
    draft.task_history.clear();
    let bytes = draft.prepare_task_change_from(state.task_cards_size()?)?;
    store
        .ok_or("Workspace storage is unavailable.")?
        .save_task_bytes(&bytes)?;
    *state = draft;
    Ok(())
}

pub fn assign(
    state: &mut Workspace,
    project: u64,
    id: u64,
    pane: Option<u64>,
    now: u64,
) -> Result<bool, String> {
    if pane.is_some_and(|pane| {
        !state.sessions.iter().any(|s| {
            s.id == pane
                && s.project == Some(project)
                && matches!(
                    s.launch,
                    crate::launch::Launch::Agent(_)
                        | crate::launch::Launch::CodexChat
                        | crate::launch::Launch::ClaudeChat
                )
        })
    }) {
        return Err("Choose an agent from this project.".into());
    }
    let task = state
        .tasks
        .iter_mut()
        .find(|t| t.id == id && t.project == project)
        .ok_or("Task not found in this project.")?;
    if task.column > 1 {
        return Err("Only To do and Doing tasks can be assigned.".into());
    }
    Ok(task.timeline.assign(task.column, pane, now))
}

pub fn record(state: &mut Workspace, id: u64) {
    let Some(task) = state.tasks.iter().find(|t| t.id == id) else {
        return;
    };
    let old = state
        .task_history
        .records
        .iter()
        .find(|r| r.task_id == id)
        .cloned();
    if task.column != 3 {
        state.task_history.records.retain(|r| r.task_id != id);
        return;
    }
    let pane_id = task
        .timeline
        .pane_id
        .or_else(|| task.timeline.work_periods.last().and_then(|p| p.pane_id));
    let session = pane_id.and_then(|pane| {
        state
            .sessions
            .iter()
            .find(|s| s.id == pane && s.project == Some(task.project))
    });
    let (agent_name, agent_kind) = session
        .map(|s| {
            let kind = match &s.launch {
                crate::launch::Launch::Agent(a) => a.clone(),
                crate::launch::Launch::CodexChat => "codex".into(),
                crate::launch::Launch::ClaudeChat => "claude".into(),
                crate::launch::Launch::Shell => "shell".into(),
                crate::launch::Launch::Ssh(h) => format!("ssh:{}", h.alias),
            };
            (s.pane_name.clone().unwrap_or_else(|| s.title.clone()), kind)
        })
        .unwrap_or_else(|| {
            old.as_ref()
                .filter(|_| pane_id.is_some())
                .map(|r| (r.agent_name.clone(), r.agent_kind.clone()))
                .unwrap_or_default()
        });
    let now = task.timeline.done_at_ms.unwrap_or(0);
    let rec_id = old.as_ref().map(|r| r.id).unwrap_or_else(|| {
        state.task_history.next_id = state.task_history.next_id.saturating_add(1);
        state.task_history.next_id
    });
    let rec = TaskHistoryRecord {
        id: rec_id,
        task_id: id,
        title: task.title.clone(),
        project_id: task.project,
        project_name: state
            .projects
            .iter()
            .find(|p| p.id == task.project)
            .map(|p| p.name.clone())
            .unwrap_or_default(),
        agent_name,
        agent_kind,
        pane_id,
        created_at_ms: task.timeline.created_at_ms.unwrap_or(0),
        started_at_ms: task.timeline.started_at_ms.unwrap_or(0),
        done_at_ms: now,
        duration_ms: task.timeline.duration_ms(now),
        work_periods: task.timeline.closed_periods(now),
        tokens: old.as_ref().map_or(0, |r| r.tokens),
        cost_usd: old.as_ref().map_or(0.0, |r| r.cost_usd),
    };
    state.task_history.records.retain(|r| r.task_id != id);
    state.task_history.records.push(rec);
    let excess = state
        .task_history
        .records
        .len()
        .saturating_sub(MAX_TASK_HISTORY);
    state.task_history.records.drain(..excess);
}

pub fn validate(history: &TaskHistoryStore) -> Result<(), String> {
    let mut ids = HashSet::new();
    let mut tasks = HashSet::new();
    if history.records.len() > MAX_TASK_HISTORY || history.next_id == u64::MAX {
        return Err("Task history exceeds its limit.".into());
    }
    for r in &history.records {
        if history.next_id == u64::MAX
            || r.task_id == u64::MAX
            || r.project_id == u64::MAX
            || r.pane_id == Some(u64::MAX)
            || r.work_periods.iter().any(|p| p.pane_id == Some(u64::MAX))
            || !ids.insert(r.id)
            || !tasks.insert(r.task_id)
            || r.id > history.next_id
            || [&r.title, &r.project_name, &r.agent_name, &r.agent_kind]
                .iter()
                .any(|s| s.len() > 4096)
            || !r.cost_usd.is_finite()
            || r.cost_usd < 0.0
        {
            return Err("Invalid task history.".into());
        }
        crate::task_timeline::Timeline {
            pane_id: r.pane_id,
            work_periods: r.work_periods.clone(),
            ..Default::default()
        }
        .validate()?;
        if r.work_periods.iter().any(|p| p.end_ms.is_none()) {
            return Err("Task history contains an unfinished period.".into());
        }
    }
    Ok(())
}

pub fn load_legacy(dir: &Path) -> Result<TaskHistoryStore, String> {
    let file = match std::fs::File::open(TaskHistoryStore::file_path(dir)) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(_) => return Err("Cannot read existing task history; original retained.".into()),
    };
    let mut bytes = Vec::new();
    file.take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read existing task history.")?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("Existing task history is too large; original retained.".into());
    }
    let mut history: TaskHistoryStore = serde_json::from_slice(&bytes)
        .map_err(|_| "Invalid existing task history; original retained.")?;
    // Older builds appended another record on each re-completion. Keep the
    // newest record for each card, leaving the original file untouched.
    let mut seen = HashSet::new();
    history.records.reverse();
    history.records.retain(|r| seen.insert(r.task_id));
    history.records.reverse();
    let excess = history.records.len().saturating_sub(MAX_TASK_HISTORY);
    history.records.drain(..excess);
    validate(&history)?;
    Ok(history)
}

pub fn raise_identity_floors(state: &mut Workspace) -> Result<(), String> {
    let highest = state
        .task_history
        .records
        .iter()
        .flat_map(|r| [r.task_id, r.project_id])
        .max()
        .unwrap_or(0);
    state.next_id = state.next_id.max(
        highest
            .checked_add(1)
            .ok_or("Task history identifiers are exhausted.")?,
    );
    let pane = state
        .task_history
        .records
        .iter()
        .flat_map(|r| {
            r.pane_id
                .into_iter()
                .chain(r.work_periods.iter().filter_map(|p| p.pane_id))
        })
        .max()
        .unwrap_or(0);
    state.next_pane = state.next_pane.max(
        pane.checked_add(1)
            .ok_or("Task history pane identifiers are exhausted.")?,
    );
    Ok(())
}

pub fn merge_legacy(
    current: &mut TaskHistoryStore,
    legacy: TaskHistoryStore,
    live: &[crate::workspace::Task],
    next_task_id: u64,
) -> Result<(), String> {
    let known: HashSet<u64> = current.records.iter().map(|r| r.task_id).collect();
    let mut next = current.next_id.max(legacy.next_id);
    let live_ids: HashSet<u64> = live.iter().map(|t| t.id).collect();
    let mut free = live_ids
        .iter()
        .copied()
        .chain(current.records.iter().map(|r| r.task_id))
        .chain(legacy.records.iter().map(|r| r.task_id))
        .chain(std::iter::once(next_task_id.saturating_sub(1)))
        .max()
        .unwrap_or(0);
    let mut records = Vec::new();
    for mut record in legacy.records {
        if known.contains(&record.task_id) {
            continue;
        }
        next = next
            .checked_add(1)
            .ok_or("Task history identifiers are exhausted.")?;
        record.id = next;
        // A recovered archive must not acquire the identity of a newer live card.
        if live_ids.contains(&record.task_id) {
            free = free
                .checked_add(1)
                .ok_or("Task history identifiers are exhausted.")?;
            record.task_id = free;
        }
        records.push(record);
    }
    records.append(&mut current.records);
    let excess = records.len().saturating_sub(MAX_TASK_HISTORY);
    records.drain(..excess);
    current.records = records;
    current.next_id = next;
    validate(current)
}

pub fn preserve_for_restore(before: &Workspace, after: &mut Workspace) -> Result<(), String> {
    let id_max = before
        .task_history
        .records
        .iter()
        .flat_map(|r| [r.task_id, r.project_id])
        .chain(after.projects.iter().map(|p| p.id))
        .chain(after.tasks.iter().map(|t| t.id))
        .max()
        .unwrap_or(0);
    after.next_id = after.next_id.max(before.next_id).max(
        id_max
            .checked_add(1)
            .ok_or("Restored task identifiers are exhausted.")?,
    );
    let mut archive = before.task_history.clone();
    let historical_panes: Vec<u64> = archive
        .records
        .iter()
        .flat_map(|r| {
            r.pane_id
                .into_iter()
                .chain(r.work_periods.iter().filter_map(|p| p.pane_id))
        })
        .collect();
    let imported_panes: HashSet<u64> = after
        .sessions
        .iter()
        .map(|s| s.id)
        .chain(after.native_threads.keys().copied())
        .chain(after.chat_threads.keys().copied())
        .chain(after.claude_threads.keys().copied())
        .collect();
    let floor = historical_panes
        .into_iter()
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("Restored pane identifiers are exhausted.")?;
    after.next_pane = after.next_pane_id().max(before.next_pane_id()).max(floor);
    for record in &mut archive.records {
        record.project_id = before
            .projects
            .iter()
            .find(|p| p.id == record.project_id)
            .and_then(|old| {
                after
                    .projects
                    .iter()
                    .find(|p| path_key(&p.path) == path_key(&old.path))
            })
            .map_or(0, |p| p.id);
        if record
            .pane_id
            .is_some_and(|id| imported_panes.contains(&id))
        {
            record.pane_id = None;
        }
        for period in &mut record.work_periods {
            if period
                .pane_id
                .is_some_and(|id| imported_panes.contains(&id))
            {
                period.pane_id = None;
            }
        }
    }
    for index in 0..after.tasks.len() {
        let id = after.id()?;
        let task = &mut after.tasks[index];
        task.id = id;
        task.timeline = Default::default();
        task.guest_by = None;
    }
    after.task_history = archive;
    after.task_history_migrated = before.task_history_migrated;
    Ok(())
}
fn path_key(path: &Path) -> String {
    #[cfg(windows)]
    {
        dunce::simplified(path)
            .to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().trim_end_matches('/').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_history_legacy_migration_keeps_original_and_does_not_resurrect_after_clear() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = TaskHistoryStore {
            next_id: 2,
            records: vec![
                TaskHistoryRecord {
                    id: 1,
                    task_id: 42,
                    title: "Old".into(),
                    ..Default::default()
                },
                TaskHistoryRecord {
                    id: 2,
                    task_id: 42,
                    title: "Newest".into(),
                    ..Default::default()
                },
            ],
        };
        let bytes = serde_json::to_vec(&legacy).unwrap();
        let path = TaskHistoryStore::file_path(dir.path());
        std::fs::write(&path, &bytes).unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        assert_eq!(state.task_history.records.len(), 1);
        assert_eq!(state.task_history.records[0].title, "Newest");
        assert!(state.next_id > 42);
        clear(&mut state, Some(&store)).unwrap();
        drop(store);
        let (_store, reopened) = Store::open(dir.path()).unwrap();
        assert!(reopened.task_history.records.is_empty());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    #[test]
    fn task_history_recovered_archive_cannot_attach_cost_to_a_new_live_card() {
        let mut state = fixture();
        let id = state.tasks[0].id;
        let legacy = TaskHistoryStore {
            next_id: 1,
            records: vec![TaskHistoryRecord {
                id: 1,
                task_id: id,
                title: "Old archive".into(),
                cost_usd: 12.5,
                ..Default::default()
            }],
        };
        merge_legacy(&mut state.task_history, legacy, &state.tasks, state.next_id).unwrap();
        let archive_id = state.task_history.records[0].task_id;
        assert_ne!(archive_id, id);
        assert!(archive_id >= state.next_id);
        raise_identity_floors(&mut state).unwrap();
        assert!(state.next_id > archive_id);
        state.tasks[0].column = 3;
        record(&mut state, id);
        assert_eq!(state.task_history.records.len(), 2);
        assert_eq!(
            state
                .task_history
                .records
                .iter()
                .find(|r| r.task_id == id)
                .unwrap()
                .cost_usd,
            0.0
        );
        assert_eq!(
            state
                .task_history
                .records
                .iter()
                .find(|r| r.task_id == archive_id)
                .unwrap()
                .cost_usd,
            12.5
        );
    }
    #[test]
    fn task_history_corrupt_legacy_warns_and_retry_preserves_new_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = TaskHistoryStore::file_path(dir.path());
        std::fs::write(&path, b"broken legacy fixture").unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        assert!(!store.warning.is_empty());
        assert!(!state.task_history_migrated);
        let project = state.add_project(dir.path()).unwrap();
        crate::agent_tools::execute(
            &mut state,
            project,
            "tessel_task_add",
            &serde_json::json!({"title":"New record","column":3}),
        )
        .unwrap();
        store.save(&state).unwrap();
        drop(store);
        assert_eq!(std::fs::read(&path).unwrap(), b"broken legacy fixture");
        let fixed = TaskHistoryStore {
            next_id: 1,
            records: vec![TaskHistoryRecord {
                id: 1,
                task_id: 999,
                title: "Recovered".into(),
                ..Default::default()
            }],
        };
        std::fs::write(&path, serde_json::to_vec(&fixed).unwrap()).unwrap();
        let (store, state) = Store::open(dir.path()).unwrap();
        assert!(store.warning.is_empty());
        assert!(state.task_history_migrated);
        assert_eq!(state.task_history.records.len(), 2);
        assert!(
            state
                .task_history
                .records
                .iter()
                .any(|r| r.title == "New record")
        );
        assert!(state.next_id > 999);
    }
    #[test]
    fn task_history_legacy_oversized_cards_can_be_reduced_but_not_grown() {
        let mut state = fixture();
        state.tasks = (2..902)
            .map(|id| Task {
                id,
                project: 1,
                title: "x".repeat(2500),
                column: 0,
                guest_by: None,
                timeline: Default::default(),
            })
            .collect();
        state.next_id = 10000;
        assert!(state.task_cards_size().unwrap() > 2 * 1024 * 1024);
        assert!(
            crate::task_actions::apply(
                &mut state,
                1,
                crate::tasks_board_panel::TasksBoardAction::Delete(2)
            )
            .unwrap()
        );
        assert_eq!(state.tasks.len(), 899);
        assert!(
            crate::task_actions::apply(
                &mut state,
                1,
                crate::tasks_board_panel::TasksBoardAction::Add("New".into())
            )
            .is_err()
        );
        assert_eq!(state.tasks.len(), 899);
        let mut restored = state.clone();
        preserve_for_restore(&state, &mut restored).unwrap();
        restored
            .prepare_task_change_from(restored.task_cards_size().unwrap())
            .unwrap();
    }
    #[test]
    fn task_history_large_restore_trims_before_atomic_restore() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(dir.path()).unwrap();
        let mut before = fixture();
        before.next_id = 20000;
        before.task_history.next_id = 1100;
        before.task_history.records = (1..=1100)
            .map(|id| TaskHistoryRecord {
                id,
                task_id: 10000 + id,
                title: "x".repeat(3000),
                ..Default::default()
            })
            .collect();
        let mut after = fixture();
        after.tasks = (2..402)
            .map(|id| Task {
                id,
                project: 1,
                title: "y".repeat(2000),
                column: 0,
                guest_by: None,
                timeline: Default::default(),
            })
            .collect();
        after.next_id = 500;
        preserve_for_restore(&before, &mut after).unwrap();
        assert!(serde_json::to_vec_pretty(&after).unwrap().len() > 4 * 1024 * 1024);
        after.prepare_task_change().unwrap();
        store.restore(&before, &after).unwrap();
        assert!(
            std::fs::read(dir.path().join("workspace.json"))
                .unwrap()
                .len()
                < 4 * 1024 * 1024
        );
        assert!(after.task_history.records.len() < before.task_history.records.len());
    }
    use crate::{
        task_timeline::Timeline,
        workspace::{Project, Store, Task},
    };
    fn fixture() -> Workspace {
        Workspace {
            projects: vec![Project {
                id: 1,
                name: "Project".into(),
                path: std::env::temp_dir(),
            }],
            active_project: Some(1),
            next_id: 100,
            task_history_migrated: true,
            tasks: vec![Task {
                id: 2,
                project: 1,
                title: "Task".into(),
                column: 0,
                guest_by: None,
                timeline: Timeline::created(100, 0, None),
            }],
            ..Default::default()
        }
    }
    fn move_at(state: &mut Workspace, column: usize, now: u64) {
        let task = &mut state.tasks[0];
        task.timeline.move_to(task.column, column, now);
        task.column = column;
        record(state, 2);
    }
    #[test]
    fn task_history_is_unique_reopens_and_survives_deletion_without_false_agent() {
        let mut state = fixture();
        state.sessions.push(serde_json::from_value(serde_json::json!({
            "id":7,"project":1,"title":"Unrelated agent","launch":"CodexChat","cwd":std::env::temp_dir()
        })).unwrap());
        move_at(&mut state, 1, 200);
        move_at(&mut state, 2, 300);
        move_at(&mut state, 1, 900);
        move_at(&mut state, 3, 1000);
        assert_eq!(state.task_history.records[0].duration_ms, 200);
        assert!(state.task_history.records[0].agent_name.is_empty());
        record(&mut state, 2);
        assert_eq!(state.task_history.records.len(), 1);
        move_at(&mut state, 0, 1100);
        assert!(state.task_history.records.is_empty());
        state.tasks[0].timeline.assign(0, Some(7), 1150);
        move_at(&mut state, 1, 1200);
        move_at(&mut state, 3, 1250);
        assert_eq!(state.task_history.records[0].agent_name, "Unrelated agent");
        assert_eq!(state.task_history.records[0].duration_ms, 250);
        state.tasks.clear();
        state.projects.clear();
        record(&mut state, 2);
        assert_eq!(state.task_history.records[0].project_name, "Project");
        validate(&state.task_history).unwrap();
    }
    #[test]
    fn task_history_and_cards_persist_atomically_and_failed_write_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(dir.path()).unwrap();
        let mut state = fixture();
        crate::task_actions::commit(
            &mut state,
            Some(&store),
            1,
            crate::tasks_board_panel::TasksBoardAction::Move { id: 2, column: 3 },
        )
        .unwrap();
        let disk: Workspace =
            serde_json::from_slice(&std::fs::read(dir.path().join("workspace.json")).unwrap())
                .unwrap();
        assert_eq!(disk.task_history, state.task_history);
        assert_eq!(disk.tasks, state.tasks);
        std::fs::remove_file(dir.path().join("workspace.json")).unwrap();
        std::fs::create_dir(dir.path().join("workspace.json")).unwrap();
        assert!(
            crate::task_actions::commit(
                &mut state,
                Some(&store),
                1,
                crate::tasks_board_panel::TasksBoardAction::Move { id: 2, column: 0 }
            )
            .is_err()
        );
        assert_eq!(state.tasks[0].column, 3);
        assert_eq!(state.task_history.records.len(), 1);
    }
    #[test]
    fn task_history_restore_retains_local_records_and_never_lowers_next_pane() {
        let mut before = fixture();
        move_at(&mut before, 3, 1000);
        before.next_pane = 500;
        let mut after = fixture();
        after.tasks[0].timeline.pane_id = Some(9);
        preserve_for_restore(&before, &mut after).unwrap();
        assert_eq!(before.task_history, after.task_history);
        assert_eq!(after.next_pane, 500);
        assert!(after.tasks[0].timeline.is_empty());
        assert_ne!(after.tasks[0].id, before.tasks[0].id);
        assert!(after.next_id > before.next_id);
    }
    #[test]
    fn task_history_restore_separates_cards_costs_projects_and_panes() {
        let mut before = fixture();
        move_at(&mut before, 3, 1000);
        before.task_history.records[0].tokens = 123;
        before.task_history.records[0].cost_usd = 4.0;
        before.task_history.records[0].pane_id = Some(700);
        let mut after = fixture();
        after.projects[0].path = std::env::temp_dir().join("different-project");
        after.sessions.push(serde_json::from_value(serde_json::json!({"id":700,"project":1,"title":"Imported","launch":"CodexChat","cwd":std::env::temp_dir()})).unwrap());
        preserve_for_restore(&before, &mut after).unwrap();
        let imported = after.tasks[0].id;
        assert_ne!(imported, 2);
        assert_eq!(after.task_history.records[0].project_id, 0);
        assert_eq!(after.task_history.records[0].pane_id, None);
        assert!(after.next_pane > 700);
        record(&mut after, imported);
        assert_eq!(after.task_history.records.len(), 1);
        after.tasks[0].column = 3;
        record(&mut after, imported);
        assert_eq!(after.task_history.records.len(), 2);
        assert_eq!(after.task_history.records[1].tokens, 0);
        assert_eq!(after.task_history.records[1].cost_usd, 0.0);
        let mut same = fixture();
        same.projects[0].id = 50;
        same.tasks[0].project = 50;
        preserve_for_restore(&before, &mut same).unwrap();
        assert_eq!(same.task_history.records[0].project_id, 50);
    }
    #[test]
    fn task_history_budget_trims_old_records_and_rejects_oversized_board() {
        let mut state = fixture();
        state.task_history.records = (1..=1500)
            .map(|id| TaskHistoryRecord {
                id,
                task_id: id + 10000,
                title: "x".repeat(3000),
                ..Default::default()
            })
            .collect();
        state.task_history.next_id = 1500;
        let bytes = state.prepare_task_change().unwrap();
        assert!(bytes.len() < 4 * 1024 * 1024);
        assert!(state.task_history.records.len() < 1500);
        assert_eq!(state.task_history.records.last().unwrap().id, 1500);
        state.tasks = (2..1002)
            .map(|id| Task {
                id,
                project: 1,
                title: "x".repeat(3000),
                column: 0,
                guest_by: None,
                timeline: Default::default(),
            })
            .collect();
        state.next_id = 2000;
        assert!(state.prepare_task_change().is_err());
    }
}
