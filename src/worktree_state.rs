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
    ShellNewPane,
    Menu {
        item: LaunchMenuItem,
        placement: LaunchPlacement,
    },
}

pub enum WorktreeEvent {
    Inspected {
        project_id: u64,
        summary: Option<GitRepoSummary>,
    },
    Created {
        project_id: u64,
        target: WorktreeLaunchTarget,
        result: Result<(CreatedWorktree, Option<GitRepoSummary>), String>,
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
                    summary,
                });
            });
        }
    }

    pub fn request_create(
        &mut self,
        project_id: u64,
        cwd: PathBuf,
        label: String,
        general_prefs: GeneralPreferences,
        git_prefs: GitPreferences,
        target: WorktreeLaunchTarget,
    ) {
        self.busy_ops += 1;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = worktree::create_worktree(
                &cwd,
                &label,
                None,
                true,
                &general_prefs,
                &git_prefs,
                None,
            )
            .map(|created| {
                let refreshed = worktree::inspect_project(&cwd).ok();
                (created, refreshed)
            });
            let _ = tx.send(WorktreeEvent::Created {
                project_id,
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

    pub fn poll(&mut self) -> Vec<WorktreeEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            match &ev {
                WorktreeEvent::Inspected {
                    project_id,
                    summary,
                } => {
                    self.in_flight_inspects.remove(project_id);
                    self.summaries.insert(*project_id, summary.clone());
                }
                WorktreeEvent::Created {
                    project_id,
                    result: Ok((_, Some(summary))),
                    ..
                } => {
                    self.busy_ops = self.busy_ops.saturating_sub(1);
                    self.summaries.insert(*project_id, Some(summary.clone()));
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
