//! Non-blocking background coordinator for Git worktree discovery, creation, cleanup scanning,
//! and removal. Bounds concurrent `git` subprocesses so sidebar refreshes never stall UI frames.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use crate::{
    finder_model::{LaunchMenuItem, LaunchPlacement},
    workspace::{GeneralPreferences, GitPreferences, Project},
    worktree::{self, CleanupItem, CreatedWorktree, GitRepoSummary},
};

const MAX_CONCURRENT_INSPECTS: usize = 2;

#[derive(Clone, Debug)]
pub enum WorktreeLaunchTarget {
    Task(u64),
    ShellNewPane,
    Menu {
        item: LaunchMenuItem,
        placement: LaunchPlacement,
    },
}

/// Captured preparation options. External issue launches explicitly disable secret copying.
pub struct CreateRequest {
    pub project_id: u64,
    pub cwd: PathBuf,
    pub label: String,
    pub base_branch: Option<String>,
    pub copy_env: bool,
    pub run_setup: bool,
    pub general_prefs: GeneralPreferences,
    pub git_prefs: GitPreferences,
    pub target: WorktreeLaunchTarget,
}

pub enum WorktreeEvent {
    Inspected {
        project_id: u64,
        source_cwd: PathBuf,
        summary: Option<GitRepoSummary>,
    },
    Created {
        project_id: u64,
        source_cwd: PathBuf,
        target: WorktreeLaunchTarget,
        result: Result<(Box<CreatedWorktree>, Option<GitRepoSummary>), String>,
    },
    CleanupScanned {
        project_id: u64,
        result: Result<(String, Vec<CleanupItem>), String>,
    },
    Removed {
        project_id: u64,
        removed: Vec<PathBuf>,
        errors: Vec<String>,
        summary: Option<GitRepoSummary>,
    },
}

pub struct WorktreeState {
    summaries: HashMap<u64, Option<GitRepoSummary>>,
    queued_inspects: VecDeque<(u64, PathBuf)>,
    in_flight_inspects: HashSet<u64>,
    busy_ops: usize,
    tx: Sender<WorktreeEvent>,
    rx: Receiver<WorktreeEvent>,
}

impl Default for WorktreeState {
    fn default() -> Self {
        Self::new()
    }
}

impl WorktreeState {
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            summaries: HashMap::new(),
            queued_inspects: VecDeque::new(),
            in_flight_inspects: HashSet::new(),
            busy_ops: 0,
            tx,
            rx,
        }
    }

    #[must_use]
    pub fn summary(&self, project_id: u64) -> Option<&GitRepoSummary> {
        self.summaries.get(&project_id).and_then(Option::as_ref)
    }

    #[cfg(windows)]
    pub fn has_inspected(&self, project_id: u64) -> bool {
        self.summaries.contains_key(&project_id)
    }

    #[must_use]
    pub const fn is_busy(&self) -> bool {
        self.busy_ops > 0
    }

    pub fn retain_projects(&mut self, projects: &[Project]) {
        let valid: HashSet<u64> = projects.iter().map(|p| p.id).collect();
        self.summaries.retain(|id, _| valid.contains(id));
        self.queued_inspects.retain(|(id, _)| valid.contains(id));
    }

    pub fn refresh_project(&mut self, project_id: u64, cwd: PathBuf) {
        if self.in_flight_inspects.contains(&project_id)
            || self.queued_inspects.iter().any(|(id, _)| *id == project_id)
        {
            return;
        }
        self.queued_inspects.push_back((project_id, cwd));
        self.pump_inspect_queue();
    }

    pub fn refresh_missing(&mut self, projects: &[Project]) {
        for project in projects {
            if !self.summaries.contains_key(&project.id) {
                self.refresh_project(project.id, project.path.clone());
            }
        }
    }

    pub fn refresh_all(&mut self, projects: &[Project]) {
        for project in projects {
            self.refresh_project(project.id, project.path.clone());
        }
    }

    fn pump_inspect_queue(&mut self) {
        while self.in_flight_inspects.len() < MAX_CONCURRENT_INSPECTS {
            let Some((project_id, cwd)) = self.queued_inspects.pop_front() else {
                break;
            };
            self.in_flight_inspects.insert(project_id);
            let tx = self.tx.clone();
            thread::spawn(move || {
                let summary = worktree::inspect_project(&cwd).ok();
                let _ = tx.send(WorktreeEvent::Inspected {
                    project_id,
                    source_cwd: cwd,
                    summary,
                });
            });
        }
    }

    pub fn request_create(&mut self, request: CreateRequest) {
        let CreateRequest {
            project_id,
            cwd,
            label,
            base_branch,
            copy_env,
            run_setup,
            general_prefs,
            git_prefs,
            target,
        } = request;
        self.busy_ops += 1;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = worktree::create_worktree(
                &cwd,
                &label,
                base_branch.as_deref(),
                copy_env,
                &general_prefs,
                &git_prefs,
                None,
            )
            .map(|mut created| {
                if run_setup
                    && let crate::worktree_setup::Outcome::Failed { ran, error } =
                        crate::worktree_setup::run(&created.path, &created.base_commit)
                {
                    created.setup_warning = Some(format!(
                        "Worktree setup {}: {error} Prepared copy kept at {}.",
                        if ran { "failed" } else { "was not run" },
                        created.path.display()
                    ));
                }
                let refreshed = worktree::inspect_project(&cwd).ok();
                (Box::new(created), refreshed)
            });
            let _ = tx.send(WorktreeEvent::Created {
                project_id,
                source_cwd: cwd,
                target,
                result,
            });
        });
    }

    pub fn request_cleanup_scan(
        &mut self,
        project_id: u64,
        cwd: PathBuf,
        open_pane_counts: HashMap<String, usize>,
    ) {
        self.busy_ops += 1;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = worktree::scan_cleanup(&cwd, &open_pane_counts);
            let _ = tx.send(WorktreeEvent::CleanupScanned { project_id, result });
        });
    }

    pub fn request_remove_batch(
        &mut self,
        project_id: u64,
        cwd: PathBuf,
        targets: Vec<(PathBuf, String, bool)>,
    ) {
        if targets.is_empty() {
            return;
        }
        self.busy_ops += 1;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let mut removed = Vec::new();
            let mut errors = Vec::new();
            for (wt_path, branch, force) in targets {
                let branch_opt = (!branch.is_empty()).then_some(branch.as_str());
                match worktree::remove_worktree(&cwd, &wt_path, branch_opt, force) {
                    Ok(()) => removed.push(wt_path),
                    Err(err) => errors.push(format!("{}: {err}", wt_path.display())),
                }
            }
            let summary = worktree::inspect_project(&cwd).ok();
            let _ = tx.send(WorktreeEvent::Removed {
                project_id,
                removed,
                errors,
                summary,
            });
        });
    }

    pub fn poll(&mut self, projects: &[Project]) -> Vec<WorktreeEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            match &ev {
                WorktreeEvent::Inspected {
                    project_id,
                    source_cwd,
                    summary,
                } => {
                    self.in_flight_inspects.remove(project_id);
                    if projects
                        .iter()
                        .any(|p| p.id == *project_id && p.path == *source_cwd)
                    {
                        self.summaries.insert(*project_id, summary.clone());
                    }
                }
                WorktreeEvent::Created {
                    project_id,
                    source_cwd,
                    result: Ok((_, Some(summary))),
                    ..
                } => {
                    self.busy_ops = self.busy_ops.saturating_sub(1);
                    if projects
                        .iter()
                        .any(|p| p.id == *project_id && p.path == *source_cwd)
                    {
                        self.summaries.insert(*project_id, Some(summary.clone()));
                    }
                }
                WorktreeEvent::Created { .. } | WorktreeEvent::CleanupScanned { .. } => {
                    self.busy_ops = self.busy_ops.saturating_sub(1);
                }
                WorktreeEvent::Removed {
                    project_id,
                    summary,
                    ..
                } => {
                    self.busy_ops = self.busy_ops.saturating_sub(1);
                    if let Some(s) = summary {
                        self.summaries.insert(*project_id, Some(s.clone()));
                    }
                }
            }
            out.push(ev);
        }
        if !self.queued_inspects.is_empty() {
            self.pump_inspect_queue();
        }
        out
    }
}

#[cfg(test)]
mod preparation_tests {
    use super::*;
    use std::{
        fs,
        time::{Duration, Instant},
    };

    #[test]
    fn worktree_preparation_inspection_cannot_replace_retargeted_project_cache() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = WorktreeState::new();
        state.in_flight_inspects.insert(7);
        state
            .tx
            .send(WorktreeEvent::Inspected {
                project_id: 7,
                source_cwd: temp.path().join("old"),
                summary: None,
            })
            .unwrap();
        let projects = [Project {
            id: 7,
            name: "Fixture".into(),
            path: temp.path().join("new"),
        }];
        assert_eq!(state.poll(&projects).len(), 1);
        assert!(!state.summaries.contains_key(&7));
        assert!(!state.in_flight_inspects.contains(&7));
        state
            .tx
            .send(WorktreeEvent::Inspected {
                project_id: 7,
                source_cwd: projects[0].path.clone(),
                summary: None,
            })
            .unwrap();
        state.poll(&projects);
        assert!(state.summaries.contains_key(&7));
    }

    #[test]
    fn worktree_preparation_uses_captured_base_and_secret_policy_without_rebinding_project() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let git = |args: &[&str]| crate::git::run(&repo, args, None).unwrap();
        git(&["init", "-b", "main"]);
        git(&["config", "user.name", "Fixture"]);
        git(&["config", "user.email", "fixture@example.test"]);
        fs::write(repo.join(".gitignore"), ".env\n").unwrap();
        fs::write(repo.join("version"), "base").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "base"]);
        git(&["branch", "task-base"]);
        fs::write(repo.join("version"), "head").unwrap();
        git(&["commit", "-am", "head"]);
        fs::write(repo.join(".env"), "SYNTHETIC_ONLY=fixture").unwrap();
        for (copy_env, moved) in [(false, true), (true, false)] {
            let mut state = WorktreeState::new();
            state.request_create(CreateRequest {
                project_id: 7,
                cwd: repo.clone(),
                label: format!("copy-{copy_env}"),
                base_branch: Some("task-base".into()),
                copy_env,
                run_setup: false,
                general_prefs: GeneralPreferences::default(),
                git_prefs: GitPreferences::default(),
                target: WorktreeLaunchTarget::ShellNewPane,
            });
            let projects = vec![Project {
                id: 7,
                name: "fixture".into(),
                path: if moved {
                    temp.path().join("different")
                } else {
                    repo.clone()
                },
            }];
            let deadline = Instant::now() + Duration::from_secs(30);
            let (source, created) = loop {
                let mut result = None;
                for event in state.poll(&projects) {
                    if let WorktreeEvent::Created {
                        source_cwd,
                        result: outcome,
                        ..
                    } = event
                    {
                        result = Some((source_cwd, outcome.unwrap().0));
                    }
                }
                if let Some(result) = result {
                    break result;
                }
                assert!(
                    Instant::now() < deadline,
                    "background worktree preparation timed out"
                );
                thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(source, repo);
            assert_eq!(
                fs::read_to_string(created.path.join("version")).unwrap(),
                "base"
            );
            assert_eq!(created.path.join(".env").exists(), copy_env);
            assert_eq!(created.copied_env_files, usize::from(copy_env));
            assert_eq!(state.summary(7).is_some(), !moved);
            assert!(!state.is_busy());
            assert!(created.path.exists());
        }
    }
}
