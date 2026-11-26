//! Git history for the active local file in the explorer Timeline.
use crate::git;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
};

#[derive(Clone, Debug)]
pub struct Commit {
    pub id: String,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub message: String,
    path: String,
    old_path: Option<String>,
}

#[derive(Debug)]
pub struct History {
    root: PathBuf,
    relative: String,
    pub commits: Vec<Commit>,
    pub dirty: bool,
    untracked: bool,
    pub more: bool,
}

fn read(file: &Path, limit: usize) -> Result<History, String> {
    let dir = file.parent().ok_or("Choose a file.")?;
    let top = git::run(dir, &["rev-parse", "--show-toplevel"], None)?;
    let root = Path::new(top.trim_end_matches(['\r', '\n']))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let absolute = dir
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(file.file_name().ok_or("Choose a file.")?);
    let rel = absolute
        .strip_prefix(&root)
        .map_err(|_| "File outside repository.")?
        .to_str()
        .ok_or("Invalid file name.")?;
    let relative = if cfg!(windows) {
        rel.replace('\\', "/")
    } else {
        rel.to_owned()
    };
    let limit = limit.clamp(1, 500);
    let count = format!("-n{}", limit + 1);
    let log = git::run_bytes(
        &root,
        &[
            "--literal-pathspecs",
            "log",
            "--no-show-signature",
            "--no-color",
            "--follow",
            "--name-status",
            "-z",
            &count,
            "--format=%H%n%aN%n%aE%n%at%n%B",
            "--",
            &relative,
        ],
        None,
    )?;
    if log.truncated {
        return Err("File history exceeds the Git output limit.".into());
    }
    let raw = String::from_utf8_lossy(&log.bytes);
    let mut fields = raw.split_terminator('\0').peekable();
    let mut commits = Vec::new();
    let mut historical = relative.clone();
    while let Some(record) = fields.next() {
        let mut meta = record.trim_start_matches('\n').splitn(5, '\n');
        let id = meta.next().unwrap_or_default();
        if !matches!(id.len(), 40 | 64) || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Invalid Git history record.".into());
        }
        let mut commit = Commit {
            id: id.to_owned(),
            author: meta.next().unwrap_or_default().into(),
            email: meta.next().unwrap_or_default().into(),
            time: meta.next().and_then(|s| s.parse().ok()).unwrap_or_default(),
            message: meta.next().unwrap_or_default().trim().into(),
            path: historical.clone(),
            old_path: None,
        };
        // Rename metadata is NUL-delimited, including paths with tabs and newlines.
        while fields
            .peek()
            .is_some_and(|s| !s.trim_start_matches('\n').contains('\n'))
        {
            let status = fields.next().unwrap().trim_start_matches('\n');
            let path = fields.next().ok_or("Incomplete Git history path.")?;
            if status.starts_with(['R', 'C']) {
                let to = fields.next().ok_or("Incomplete Git rename.")?;
                if to == historical {
                    commit.old_path = Some(path.into());
                    historical = path.into();
                }
            }
        }
        commits.push(commit);
    }
    let more = commits.len() > limit && limit < 500;
    commits.truncate(limit);
    commits.sort_by_key(|c| std::cmp::Reverse(c.time));
    let status = git::run_bytes(
        &root,
        &[
            "--literal-pathspecs",
            "status",
            "--porcelain",
            "-z",
            "--",
            &relative,
        ],
        None,
    )?;
    Ok(History {
        root,
        relative,
        commits,
        dirty: !status.bytes.is_empty(),
        untracked: status.bytes.starts_with(b"?? "),
        more,
    })
}

impl History {
    pub fn diff(&self, commit: Option<&str>) -> Result<String, String> {
        if let Some(id) = commit {
            let c = self
                .commits
                .iter()
                .find(|c| c.id == id)
                .ok_or("Choose a file history entry.")?;
            let mut args = vec![
                "--literal-pathspecs",
                "show",
                "--format=",
                "--no-show-signature",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--find-renames",
                &c.id,
                "--",
                &c.path,
            ];
            if let Some(path) = &c.old_path {
                args.push(path);
            }
            git::run(&self.root, &args, None)
        } else if self.untracked {
            git::file_diff(&self.root, &self.relative, None, false, true)
        } else {
            git::run(
                &self.root,
                &[
                    "--literal-pathspecs",
                    "diff",
                    "--no-color",
                    "--no-ext-diff",
                    "--no-textconv",
                    "HEAD",
                    "--",
                    &self.relative,
                ],
                None,
            )
        }
    }
}

#[derive(Default)]
pub struct State {
    pub file: Option<PathBuf>,
    pub data: Option<Result<Arc<History>, String>>,
    limit: usize,
    revision: u64,
    pending: Option<(u64, mpsc::Receiver<Result<History, String>>)>,
}
impl State {
    pub fn select(&mut self, file: Option<PathBuf>) {
        if self.file != file {
            self.file = file;
            self.limit = 20;
            self.refresh(false);
        }
        self.start();
    }
    pub fn refresh(&mut self, more: bool) {
        if more {
            self.limit = (self.limit + 20).min(500);
        }
        self.revision = self.revision.wrapping_add(1);
        self.data = None;
        self.start();
    }
    fn start(&mut self) {
        if self.pending.is_some() || self.data.is_some() {
            return;
        }
        let Some(file) = self.file.clone() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let limit = self.limit;
        self.pending = Some((self.revision, rx));
        thread::spawn(move || {
            let _ = tx.send(read(&file, limit));
        });
    }
    pub fn poll(&mut self) -> bool {
        let Some((revision, rx)) = &self.pending else {
            return false;
        };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => Err("File history worker stopped.".into()),
        };
        if *revision == self.revision {
            self.data = Some(result.map(Arc::new));
        }
        self.pending = None;
        self.start();
        true
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    Toggle,
    Refresh(PathBuf, bool),
    Open(PathBuf, Option<String>, bool),
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub shown: bool,
    pub file: Option<PathBuf>,
    pub data: Option<Result<Arc<History>, String>>,
}
impl Snapshot {
    pub fn status(&self, fr: bool) -> Option<&str> {
        match (&self.file, &self.data) {
            (None, _) => Some(if fr {
                "Aucun fichier actif."
            } else {
                "No active file."
            }),
            (_, None) => Some(if fr {
                "Chargement de la chronologie…"
            } else {
                "Loading timeline…"
            }),
            (_, Some(Err(e))) => Some(e),
            (_, Some(Ok(h))) if !h.dirty && h.commits.is_empty() => Some(if fr {
                "Aucune modification Git pour ce fichier."
            } else {
                "No Git changes for this file."
            }),
            _ => None,
        }
    }
}
impl Commit {
    pub fn label(&self) -> String {
        let date = chrono::DateTime::from_timestamp(self.time, 0)
            .map(|d| {
                d.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_default();
        format!(
            "{} · {} · {date}",
            self.message.lines().next().unwrap_or(&self.id),
            self.author
        )
    }
}
pub fn compat(ui: &mut eframe::egui::Ui, snap: &Snapshot, fr: bool) -> Option<Action> {
    let mut action = None;
    if ui
        .selectable_label(snap.shown, if fr { "Chronologie" } else { "Timeline" })
        .clicked()
    {
        action = Some(Action::Toggle);
    }
    if !snap.shown {
        return action;
    }
    if let Some(file) = &snap.file {
        ui.horizontal(|ui| {
            ui.label(file.file_name().unwrap_or_default().to_string_lossy());
            if ui
                .small_button(if fr { "Actualiser" } else { "Refresh" })
                .clicked()
            {
                action = Some(Action::Refresh(file.clone(), false));
            }
        });
    }
    if let Some(status) = snap.status(fr) {
        ui.label(status);
    }
    if let (Some(file), Some(Ok(history))) = (&snap.file, &snap.data) {
        eframe::egui::ScrollArea::vertical()
            .id_salt("timeline")
            .max_height(160.)
            .show(ui, |ui| {
                if history.dirty {
                    let r = ui.selectable_label(
                        false,
                        if fr {
                            "Modifications non validées"
                        } else {
                            "Uncommitted Changes"
                        },
                    );
                    if r.clicked() {
                        action = Some(Action::Open(file.clone(), None, r.double_clicked()));
                    }
                }
                for c in &history.commits {
                    let r = ui.selectable_label(false, c.label()).on_hover_text(format!(
                        "{} <{}>\n{}\n{}",
                        c.author, c.email, c.id, c.message
                    ));
                    if r.clicked() {
                        action = Some(Action::Open(
                            file.clone(),
                            Some(c.id.clone()),
                            r.double_clicked(),
                        ));
                    }
                }
                if history.more
                    && ui
                        .button(if fr { "Charger la suite" } else { "Load More" })
                        .clicked()
                {
                    action = Some(Action::Refresh(file.clone(), true));
                }
            });
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    fn git_fixture(dir: &Path, args: &[&str]) -> String {
        let mut opts = vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ];
        opts.extend_from_slice(args);
        git::run(dir, &opts, None).unwrap()
    }
    #[test]
    fn file_timeline_follows_literal_renames_pages_and_opens_original_patch() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        git_fixture(root, &["init", "-q"]);
        std::fs::create_dir(root.join("docs")).unwrap();
        let original = "docs/[old].txt";
        let current = "docs/[new].txt";
        std::fs::write(root.join(original), "original\n").unwrap();
        git_fixture(root, &["add", "."]);
        git_fixture(
            root,
            &["commit", "-qm", "first", "-m", "Body with\nmultiple lines"],
        );
        git_fixture(root, &["mv", original, current]);
        git_fixture(root, &["commit", "-qm", "rename"]);
        std::fs::write(root.join("docs/n.txt"), "not this file").unwrap();
        git_fixture(root, &["add", "."]);
        git_fixture(root, &["commit", "-qm", "unrelated"]);
        let file = root.join(current);
        let first = read(&file, 1).unwrap();
        assert!(first.more && !first.dirty);
        assert_eq!(first.commits[0].message, "rename");
        std::fs::write(&file, "original\nworking\n").unwrap();
        let h = read(&file, 20).unwrap();
        assert!(h.dirty && !h.more);
        assert_eq!(h.commits.len(), 2);
        let old = &h.commits[1];
        assert_eq!(old.email, "fixture@example.invalid");
        assert!(old.message.contains("multiple lines"));
        assert!(h.diff(Some(&old.id)).unwrap().contains("+original"));
        assert!(
            h.diff(Some(&h.commits[0].id))
                .unwrap()
                .contains("rename from docs/[old].txt")
        );
        assert!(h.diff(None).unwrap().contains("+working"));
    }

    #[test]
    fn file_timeline_abandons_old_worker_when_active_file_changes() {
        let (tx, rx) = mpsc::channel();
        let mut state = State {
            pending: Some((0, rx)),
            file: Some(PathBuf::from("previous")),
            ..State::default()
        };
        state.select(None);
        tx.send(Err("stale error".into())).unwrap();
        assert!(state.poll());
        assert!(state.file.is_none() && state.data.is_none() && state.pending.is_none());
    }
}
