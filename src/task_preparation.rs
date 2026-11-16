//! Durable task instructions and preparation receipts, never launch authority.
//! Restoring these values does not enqueue work, run setup or start an agent.
use crate::workspace::{Store, Task, Workspace};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_BRIEF_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Worktree {
    pub path: PathBuf,
    pub root: PathBuf,
    pub branch: String,
    pub base_branch: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Details {
    pub source_cwd: PathBuf,
    pub brief: String,
    pub reviewer_id: Option<u64>,
    pub worktree: Option<Worktree>,
}

fn bounded_path(path: &std::path::Path) -> bool {
    path.is_absolute()
        && path.to_string_lossy().len() <= 32 * 1024
        && !path.to_string_lossy().chars().any(char::is_control)
}
fn branch(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
}
impl Details {
    pub fn validate(&self) -> Result<(), String> {
        if !bounded_path(&self.source_cwd)
            || self.brief.len() > MAX_BRIEF_BYTES
            || self.brief.contains('\0')
            || self.reviewer_id.is_some_and(|id| id == 0 || id == u64::MAX)
            || self.worktree.as_ref().is_some_and(|w| {
                !bounded_path(&w.path)
                    || !bounded_path(&w.root)
                    || !branch(&w.branch)
                    || w.base_branch.as_ref().is_some_and(|b| !branch(b))
            })
        {
            return Err("Invalid task preparation metadata.".into());
        }
        Ok(())
    }
}

/// In-memory identity returned only after the card has been saved.
/// It is not reconstructed from imported or restored task metadata.
#[derive(Clone, Debug)]
pub struct Ticket {
    task: u64,
    project: u64,
    source_cwd: PathBuf,
}
impl Ticket {
    pub fn task_id(&self) -> u64 {
        self.task
    }
    pub fn project_id(&self) -> u64 {
        self.project
    }
    pub fn source_cwd(&self) -> &std::path::Path {
        &self.source_cwd
    }
    /// Project check only, independent of selection. A launch adapter must also
    /// recheck the task, agent availability and its own live operation identity.
    pub fn project_is_current(&self, state: &Workspace) -> bool {
        state
            .projects
            .iter()
            .any(|p| p.id == self.project && p.path == self.source_cwd)
    }
}

fn save(state: &mut Workspace, store: Option<&Store>, draft: Workspace) -> Result<(), String> {
    let mut draft = draft;
    let bytes = draft.prepare_task_change_from(state.task_cards_size()?)?;
    store
        .ok_or("Workspace storage is unavailable.")?
        .save_task_bytes(&bytes)?;
    *state = draft;
    Ok(())
}

/// Owner's explicit New Task preparation. Remains in To do until a launch succeeds.
pub fn create(
    state: &mut Workspace,
    store: Option<&Store>,
    project: u64,
    title: &str,
    details: Details,
) -> Result<Ticket, String> {
    let title = title.trim();
    details.validate()?;
    if title.is_empty() || title.len() > 4096 || title.chars().any(char::is_control) {
        return Err("Enter a task title of at most 4096 bytes.".into());
    }
    if state.tasks.len() >= 10_000 || details.worktree.is_some() {
        return Err("Cannot prepare this task.".into());
    }
    if !state
        .projects
        .iter()
        .any(|p| p.id == project && p.path == details.source_cwd)
    {
        return Err("The workspace folder changed. No agent was started.".into());
    }
    if details.reviewer_id.is_some_and(|id| {
        !crate::task_agents_model::choices(state, project)
            .iter()
            .any(|p| p.id == id)
            || state
                .tasks
                .iter()
                .any(|t| t.column != 3 && t.timeline.pane_id == Some(id))
    }) {
        return Err("Choose an available reviewer from this project.".into());
    }
    let mut draft = state.clone();
    let id = draft.id()?;
    let ticket = Ticket {
        task: id,
        project,
        source_cwd: details.source_cwd.clone(),
    };
    draft.tasks.push(Task {
        preparation: Some(details),
        created_by: None,
        report: None,
        decision: None,
        last_waiting: Vec::new(),
        deps: Vec::new(),
        id,
        project,
        title: title.into(),
        column: 0,
        guest_by: None,
        timeline: crate::task_timeline::Timeline::created(crate::task_timeline::now_ms(), 0, None),
    });
    save(state, store, draft)?;
    Ok(ticket)
}

/// Keep the prepared copy on its original card even if the project moved meanwhile.
/// This receipt does not authorize launching from that copy.
pub fn record_worktree(
    state: &mut Workspace,
    store: Option<&Store>,
    ticket: &Ticket,
    worktree: Worktree,
) -> Result<(), String> {
    let mut draft = state.clone();
    let task = draft
        .tasks
        .iter_mut()
        .find(|t| t.id == ticket.task && t.project == ticket.project)
        .ok_or("The prepared task no longer exists; its copy was kept.")?;
    let details = task
        .preparation
        .as_mut()
        .ok_or("Task preparation changed.")?;
    if task.column != 0 || details.source_cwd != ticket.source_cwd || details.worktree.is_some() {
        return Err("Task preparation changed; its copy was kept.".into());
    }
    details.worktree = Some(worktree);
    details.validate()?;
    save(state, store, draft)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Project;

    fn fixture() -> (tempfile::TempDir, Store, Workspace, Details) {
        let temp = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(&temp.path().join("state")).unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let id = state.add_project(&root).unwrap();
        assert_eq!(id, 1);
        store.save(&state).unwrap();
        let details = Details {
            source_cwd: state.projects[0].path.clone(),
            brief: "Instructions fixture\nSecond line".into(),
            reviewer_id: None,
            worktree: None,
        };
        (temp, store, state, details)
    }
    fn copy(temp: &tempfile::TempDir, root: PathBuf) -> Worktree {
        let path = temp.path().join("prepared");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("keep"), "synthetic").unwrap();
        Worktree {
            path,
            root,
            branch: "agent/fixture".into(),
            base_branch: Some("main".into()),
        }
    }

    #[test]
    fn task_preparation_survives_reopen_without_starting_or_sharing_private_details() {
        let (temp, store, mut state, details) = fixture();
        let ticket = create(&mut state, Some(&store), 1, "  New task  ", details.clone()).unwrap();
        let worktree = copy(&temp, details.source_cwd.clone());
        record_worktree(&mut state, Some(&store), &ticket, worktree.clone()).unwrap();
        let shared = serde_json::to_string(&crate::shared_task_apply::snapshot(&state)).unwrap();
        assert!(!shared.contains("Instructions fixture"));
        assert!(!shared.contains("source_cwd"));
        assert!(!shared.contains("worktree"));
        drop(store);
        let (_store, reopened) = Store::open(&temp.path().join("state")).unwrap();
        let card = &reopened.tasks[0];
        assert_eq!(card.id, ticket.task_id());
        assert_eq!(card.title, "New task");
        assert_eq!(card.preparation.as_ref().unwrap().brief, details.brief);
        assert_eq!(card.preparation.as_ref().unwrap().worktree, Some(worktree));
        assert_eq!(card.column, 0);
        assert!(card.timeline.pane_id.is_none());
        assert!(reopened.sessions.is_empty());
    }

    #[test]
    fn task_preparation_storage_failure_leaves_card_identity_and_receipt_unchanged() {
        let (temp, store, mut state, details) = fixture();
        let before = serde_json::to_vec(&state).unwrap();
        assert!(create(&mut state, None, 1, "Task", details.clone()).is_err());
        assert_eq!(serde_json::to_vec(&state).unwrap(), before);
        let ticket = create(&mut state, Some(&store), 1, "Task", details.clone()).unwrap();
        let worktree = copy(&temp, details.source_cwd);
        let before = serde_json::to_vec(&state).unwrap();
        let path = store.directory().join("workspace.json");
        let backup = store.directory().join("saved.json");
        std::fs::rename(&path, &backup).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(record_worktree(&mut state, Some(&store), &ticket, worktree.clone()).is_err());
        assert_eq!(serde_json::to_vec(&state).unwrap(), before);
        assert!(worktree.path.join("keep").exists());
        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(backup, path).unwrap();
        record_worktree(&mut state, Some(&store), &ticket, worktree.clone()).unwrap();
        assert!(record_worktree(&mut state, Some(&store), &ticket, worktree).is_err());
    }

    #[test]
    fn task_preparation_retains_receipt_but_rejects_changed_project_for_launch() {
        let (temp, store, mut state, details) = fixture();
        let ticket = create(&mut state, Some(&store), 1, "Task", details.clone()).unwrap();
        assert!(ticket.project_is_current(&state));
        state.projects[0].path = temp.path().join("moved");
        assert!(!ticket.project_is_current(&state));
        assert!(
            create(
                &mut state,
                Some(&store),
                1,
                "Wrong context",
                details.clone()
            )
            .is_err()
        );
        let worktree = copy(&temp, details.source_cwd);
        record_worktree(&mut state, Some(&store), &ticket, worktree.clone()).unwrap();
        assert_eq!(
            state.tasks[0].preparation.as_ref().unwrap().worktree,
            Some(worktree)
        );
        state.projects.clear();
        assert!(!ticket.project_is_current(&state));
        assert!(state.sessions.is_empty());
    }

    #[test]
    fn task_preparation_settings_import_preserves_receipt_but_drops_reviewer_assignment() {
        let (temp, store, mut state, details) = fixture();
        let ticket = create(&mut state, Some(&store), 1, "Task", details.clone()).unwrap();
        let worktree = copy(&temp, details.source_cwd.clone());
        record_worktree(&mut state, Some(&store), &ticket, worktree.clone()).unwrap();
        let mut imported = state.clone();
        imported.tasks[0].preparation.as_mut().unwrap().reviewer_id = Some(70);
        crate::task_history::preserve_for_restore(&state, &mut imported).unwrap();
        let receipt = imported.tasks[0].preparation.as_ref().unwrap();
        assert!(receipt.reviewer_id.is_none());
        assert_eq!(receipt.brief, details.brief);
        assert_eq!(receipt.worktree, Some(worktree.clone()));
        assert_ne!(imported.tasks[0].id, ticket.task_id());
        assert!(record_worktree(&mut imported, Some(&store), &ticket, worktree).is_err());
        imported.validate().unwrap();
    }

    #[test]
    fn task_preparation_validates_legacy_metadata_limits_and_reviewer_scope() {
        let (temp, store, mut state, mut details) = fixture();
        let old: Task = serde_json::from_value(
            serde_json::json!({"id":2,"project":1,"title":"Legacy","column":0}),
        )
        .unwrap();
        assert!(old.preparation.is_none());
        details.brief = "x".repeat(MAX_BRIEF_BYTES + 1);
        assert!(create(&mut state, Some(&store), 1, "Task", details.clone()).is_err());
        details.brief = "fixture".into();
        details.reviewer_id = Some(0);
        assert!(details.validate().is_err());
        state.next_id = 10;
        state.projects.push(Project {
            id: 2,
            name: "Other".into(),
            path: temp.path().join("other"),
        });
        state.sessions.push(serde_json::from_value(serde_json::json!({"id":70,"project":2,"launch":"CodexChat","title":"Other reviewer","cwd":temp.path().join("other")})).unwrap());
        details.reviewer_id = Some(70);
        assert!(create(&mut state, Some(&store), 1, "Task", details.clone()).is_err());
        state.sessions[0].project = Some(1);
        state.sessions[0].cwd = details.source_cwd.clone();
        let ticket = create(&mut state, Some(&store), 1, "Task", details).unwrap();
        assert_eq!(
            state.tasks[0].preparation.as_ref().unwrap().reviewer_id,
            Some(70)
        );
        assert_eq!(ticket.task_id(), 10);
        state.tasks[0]
            .timeline
            .assign(0, Some(70), crate::task_timeline::now_ms());
        let busy = state.tasks[0].preparation.clone().unwrap();
        assert!(create(&mut state, Some(&store), 1, "Busy reviewer", busy).is_err());
        let mut malformed = serde_json::to_value(&state.tasks[0].preparation).unwrap();
        malformed["execute"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Details>(malformed).is_err());
    }
}
