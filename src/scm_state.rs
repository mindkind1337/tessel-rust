//! On-demand status work, scoped to the selected project and its repository prefix.
use crate::{
    git,
    git_result::Owner,
    scm_changes_panel::{self, ParsedGitStatus},
};
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

type StatusResult = Result<ParsedGitStatus, String>;

#[derive(Default)]
pub struct ScmState {
    owner: Option<Owner>,
    pending: Option<(Owner, Receiver<StatusResult>)>,
    pub result: Option<StatusResult>,
}

fn load(owner: &Owner) -> StatusResult {
    let capture = git::scoped_status(&owner.root)?;
    let mut parsed = scm_changes_panel::parse_porcelain_v2_status(
        &capture.output.bytes,
        scm_changes_panel::STATUS_ENTRIES_MAX,
    );
    if capture.output.truncated {
        parsed.mark_input_truncated();
    }
    for entry in &mut parsed.entries {
        entry.path = git::project_relative_path(&capture.project_prefix, &entry.path)
            .ok_or(
                "Git returned a path outside this project. Open the repository root to inspect it.",
            )?
            .to_owned();
        if let Some(old_path) = &mut entry.old_path {
            *old_path = git::project_relative_path(&capture.project_prefix, old_path)
                .ok_or("A rename crosses this project's boundary. Open the repository root to inspect it.")?
                .to_owned();
        }
    }
    Ok(parsed)
}

impl ScmState {
    pub fn select(&mut self, owner: Option<Owner>) -> bool {
        if self.owner == owner {
            return false;
        }
        self.owner = owner;
        self.result = None;
        true
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn refresh(&mut self) {
        if self.busy() {
            return;
        }
        let Some(owner) = self.owner.clone() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.pending = Some((owner.clone(), receiver));
        thread::spawn(move || {
            let _ = sender.send(load(&owner));
        });
    }

    pub fn poll(&mut self) {
        let Some((owner, receiver)) = &self.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("Git status worker stopped without a result.".into())
            }
        };
        if self.owner.as_ref() == Some(owner) {
            self.result = Some(result);
        }
        self.pending = None;
    }

    pub fn refresh_for(&mut self, owner: &Owner) {
        if self.owner.as_ref() == Some(owner) {
            self.refresh();
        }
    }

    pub fn stage_paths(&self, paths: &[String]) -> Result<Vec<String>, String> {
        self.selected_paths(paths, false)
    }

    pub fn unstage_paths(&self, paths: &[String]) -> Result<Vec<String>, String> {
        self.selected_paths(paths, true)
    }

    fn selected_paths(&self, paths: &[String], staged: bool) -> Result<Vec<String>, String> {
        let status = self
            .result
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .ok_or("Refresh Git status first.")?;
        if self.busy()
            || self.owner.is_none()
            || status.blocks_bulk_actions()
            || paths.is_empty()
            || paths.len() > 5000
        {
            return Err("Refresh a complete Git status before changing selected files.".into());
        }
        let mut selected = std::collections::BTreeSet::new();
        for path in paths {
            let entry = status
                .entries
                .iter()
                .find(|entry| {
                    entry.path == *path
                        && if staged {
                            entry.can_unstage()
                        } else {
                            entry.can_stage()
                        }
                })
                .ok_or("Selected Git file changed; refresh status.")?;
            selected.insert(entry.path.clone());
            if entry.status == scm_changes_panel::ScmFileStatus::Renamed
                && let Some(old) = &entry.old_path
            {
                selected.insert(old.clone());
            }
        }
        Ok(selected.into_iter().collect())
    }

    pub fn file_path(&self, relative: &str) -> Option<std::path::PathBuf> {
        let owner = self.owner.as_ref()?;
        let status = self.result.as_ref()?.as_ref().ok()?;
        if !status.entries.iter().any(|entry| entry.path == relative) {
            return None;
        }
        let relative = git::project_relative_path("", relative)?;
        // Document::open revalidates canonical confinement, including symlinks.
        Some(owner.root.join(relative))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_status_is_project_relative_and_open_actions_require_current_entries() {
        let dir = tempfile::tempdir().unwrap();
        git::run(dir.path(), &["init", "-q"], None).unwrap();
        let root = dir.path().join("nested space é");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file é.txt"), "fixture").unwrap();
        std::fs::write(dir.path().join("outside.txt"), "fixture").unwrap();
        let owner = Owner {
            project: 1,
            root: root.clone(),
        };
        let status = load(&owner).unwrap();
        assert_eq!(status.entries.len(), 1);
        assert_eq!(status.entries[0].path, "file é.txt");
        let mut state = ScmState {
            owner: Some(owner),
            result: Some(Ok(status)),
            ..Default::default()
        };
        assert_eq!(state.file_path("file é.txt"), Some(root.join("file é.txt")));
        assert!(state.file_path("../outside.txt").is_none());
        assert!(state.file_path("absent.txt").is_none());
        assert_eq!(
            state.stage_paths(&["file \u{e9}.txt".into()]).unwrap(),
            vec!["file \u{e9}.txt"]
        );
        assert!(state.stage_paths(&["../outside.txt".into()]).is_err());
        assert!(state.stage_paths(&[]).is_err());
        state
            .result
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .mark_input_truncated();
        assert!(state.stage_paths(&["file \u{e9}.txt".into()]).is_err());
        state.select(None);
        assert!(state.file_path("file é.txt").is_none());
    }

    #[test]
    fn old_project_results_and_disconnected_workers_do_not_stick() {
        let first = Owner {
            project: 1,
            root: "first".into(),
        };
        let second = Owner {
            project: 2,
            root: "second".into(),
        };
        let (sender, receiver) = mpsc::channel();
        let mut state = ScmState {
            owner: Some(first.clone()),
            pending: Some((first, receiver)),
            ..Default::default()
        };
        state.select(Some(second.clone()));
        sender.send(Err("old project error".into())).unwrap();
        state.poll();
        assert!(!state.busy());
        assert!(state.result.is_none());
        let (sender, receiver) = mpsc::channel();
        state.pending = Some((second, receiver));
        drop(sender);
        state.poll();
        assert!(!state.busy());
        assert!(state.result.as_ref().unwrap().is_err());
        let same_id_new_path = Owner {
            project: 2,
            root: "replacement".into(),
        };
        let (sender, receiver) = mpsc::channel();
        state.pending = Some((state.owner.clone().unwrap(), receiver));
        state.select(Some(same_id_new_path));
        state.refresh(); // A previous worker still owns the only slot.
        sender.send(Ok(ParsedGitStatus::default())).unwrap();
        state.poll();
        assert!(state.result.is_none());
        assert!(!state.busy());
        state.select(None);
        state.refresh();
        assert!(!state.busy());
    }
    #[test]
    fn unstage_rename_includes_both_paths_and_keeps_the_worktree_rename() {
        let dir = tempfile::tempdir().unwrap();
        git::run(dir.path(), &["init", "-q"], None).unwrap();
        std::fs::write(dir.path().join("old.txt"), "same content").unwrap();
        git::stage_file(dir.path(), "old.txt").unwrap();
        git::run(
            dir.path(),
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "base",
            ],
            None,
        )
        .unwrap();
        std::fs::rename(dir.path().join("old.txt"), dir.path().join("new.txt")).unwrap();
        git::stage_files(dir.path(), &["old.txt".into(), "new.txt".into()]).unwrap();
        let owner = Owner {
            project: 1,
            root: dir.path().into(),
        };
        let state = ScmState {
            result: Some(Ok(load(&owner).unwrap())),
            owner: Some(owner),
            ..Default::default()
        };
        let paths = state.unstage_paths(&["new.txt".into()]).unwrap();
        assert_eq!(paths, vec!["new.txt", "old.txt"]);
        assert!(state.stage_paths(&["new.txt".into()]).is_err());
        git::unstage_files(dir.path(), &paths).unwrap();
        assert!(
            git::run(dir.path(), &["diff", "--cached", "--name-only"], None)
                .unwrap()
                .is_empty()
        );
        assert!(!dir.path().join("old.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("new.txt")).unwrap(),
            "same content"
        );
    }
}
