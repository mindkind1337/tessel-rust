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

#[derive(Debug, Default)]
pub struct History {
    scm_unavailable: bool,
    pub saves: Vec<crate::local_file_history::Entry>,
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
        scm_unavailable: false,
        saves: Vec::new(),
        root,
        relative,
        commits,
        dirty: !status.bytes.is_empty(),
        untracked: status.bytes.starts_with(b"?? "),
        more,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sources {
    pub git: bool,
    pub local: bool,
}
impl Default for Sources {
    fn default() -> Self {
        Self {
            git: true,
            local: true,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Source {
    Git,
    Local,
}
impl Source {
    pub fn label(self, fr: bool) -> &'static str {
        match (self, fr) {
            (Self::Git, true) => "Historique Git",
            (Self::Git, false) => "Git History",
            (Self::Local, true) => "Historique local",
            (Self::Local, false) => "Local History",
        }
    }
    pub fn enabled(self, sources: Sources) -> bool {
        match self {
            Self::Git => sources.git,
            Self::Local => sources.local,
        }
    }
}
fn load(
    file: &Path,
    limit: usize,
    profile: Option<&Path>,
    sources: Sources,
) -> Result<History, String> {
    let mut history = if sources.git {
        read(file, limit).unwrap_or_else(|_| History {
            scm_unavailable: true,
            ..Default::default()
        })
    } else {
        History::default()
    };
    if sources.local
        && let Some(dir) = profile
    {
        match crate::local_file_history::list(dir, file) {
            Ok(saves) => history.saves = saves,
            Err(error) if !history.dirty && history.commits.is_empty() => {
                return Err(error.to_string());
            }
            Err(_) => {}
        }
    }
    Ok(history)
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
    pub pin: Option<PinnedFile>,
    sources: Sources,
    pub filters_open: bool,
    pub profile: Option<PathBuf>,
    pub file: Option<PathBuf>,
    pub data: Option<Result<Arc<History>, String>>,
    limit: usize,
    revision: u64,
    save_cursor: u64,
    last_age_update: Option<std::time::Instant>,
    pending: Option<(u64, mpsc::Receiver<Result<History, String>>)>,
}
impl State {
    pub fn select(&mut self, file: Option<PathBuf>, sources: Sources) {
        if self.file != file || self.sources != sources {
            if self.file != file {
                self.limit = 20;
            }
            self.sources = sources;
            self.file = file;
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
        self.last_age_update = None;
        self.start();
    }
    fn start(&mut self) {
        if self.data.is_some() {
            return;
        }
        let Some(file) = self.file.clone() else {
            return;
        };
        let sources = self.sources;
        if !sources.git && !sources.local {
            self.data = Some(Ok(Arc::default()));
            return;
        }
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let limit = self.limit;
        let profile = self.profile.clone();
        self.pending = Some((self.revision, rx));
        thread::spawn(move || {
            let _ = tx.send(load(&file, limit, profile.as_deref(), sources));
        });
    }
    pub fn poll(&mut self) -> bool {
        if crate::local_file_history::changed_since(&mut self.save_cursor, self.file.as_deref())
            && (self.sources.git || self.sources.local)
        {
            self.refresh(false);
        }
        let Some((revision, rx)) = &self.pending else {
            if self
                .last_age_update
                .is_some_and(|t| t.elapsed().as_secs() >= 30)
            {
                self.last_age_update = Some(std::time::Instant::now());
                return true;
            }
            return false;
        };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => Err("File history worker stopped.".into()),
        };
        if *revision == self.revision {
            self.last_age_update = result
                .as_ref()
                .ok()
                .filter(|h| !h.commits.is_empty() || !h.saves.is_empty())
                .map(|_| std::time::Instant::now());
            self.data = Some(result.map(Arc::new));
        }
        self.pending = None;
        self.start();
        true
    }
}

#[derive(Clone, Debug)]
pub struct PinnedFile {
    pub project: Option<u64>,
    pub root: PathBuf,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub enum Action {
    Toggle,
    TogglePin(PathBuf),
    Filters(bool),
    ToggleSource(Source),
    Refresh(PathBuf, bool),
    Open(PathBuf, Option<String>, bool),
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub shown: bool,
    pub filters_open: bool,
    pub sources: Sources,
    pub pinned: bool,
    pub file: Option<PathBuf>,
    pub data: Option<Result<Arc<History>, String>>,
}
impl Snapshot {
    pub fn file_label(&self, file: &Path, fr: bool) -> String {
        let name = file.file_name().unwrap_or_default().to_string_lossy();
        if self.pinned {
            format!("{name} ({})", if fr { "épinglé" } else { "pinned" })
        } else {
            name.into_owned()
        }
    }
    pub fn pin_label(&self, fr: bool) -> &'static str {
        match (self.pinned, fr) {
            (true, true) => "Désépingler",
            (true, false) => "Unpin",
            (false, true) => "Épingler",
            (false, false) => "Pin",
        }
    }
    pub fn status(&self, fr: bool) -> Option<String> {
        let file = match &self.file {
            Some(file) => file,
            None => {
                return Some(
                    if fr {
                        "L'éditeur actif ne peut pas fournir d'informations de chronologie."
                    } else {
                        "The active editor cannot provide timeline information."
                    }
                    .into(),
                );
            }
        };
        let h = match &self.data {
            Some(Ok(h)) => h,
            Some(Err(e)) => return Some(e.clone()),
            None => {
                let name = file.file_name().unwrap_or_default().to_string_lossy();
                return Some(if fr {
                    format!("Chargement de la chronologie pour {name}...")
                } else {
                    format!("Loading timeline for {name}...")
                });
            }
        };
        if h.dirty || !h.commits.is_empty() || !h.saves.is_empty() {
            return None;
        }
        let mut text = match (self.sources.git,self.sources.local,fr) {
            (false,false,true) => "Toutes les sources de chronologie ont été filtrées.",
            (false,false,false) => "All timeline sources have been filtered out.",
            (_,true,true) => "L'historique local suivra les modifications récentes à mesure que vous les enregistrez, sauf si le fichier a été exclu ou s'il est trop volumineux.",
            (_,true,false) => "Local History will track recent changes as you save them unless the file has been excluded or is too large.",
            (_,false,true) => "Aucune information de chronologie filtrée n'a été fournie.",
            (_,false,false) => "No filtered timeline information was provided.",
        }.to_owned();
        if h.scm_unavailable {
            text.push_str(if fr {
                " Le contrôle de code source n'a pas été configuré."
            } else {
                " Source Control has not been configured."
            });
        }
        Some(text)
    }
}
fn relative_time(time: i64, now: i64, fr: bool) -> String {
    let delta = now.saturating_sub(time);
    if (-30..30).contains(&delta) {
        return if fr { "maintenant" } else { "now" }.into();
    }
    let seconds = delta.unsigned_abs();
    let (scale, english, french) = match seconds {
        0..60 => (1, ("sec", "secs"), ("s", "s")),
        60..3600 => (60, ("min", "mins"), ("min", "min")),
        3600..86400 => (3600, ("hr", "hrs"), ("h", "h")),
        86400..604800 => (86400, ("day", "days"), ("jour", "jours")),
        604800..2592000 => (604800, ("wk", "wks"), ("sem.", "sem.")),
        2592000..31536000 => (2592000, ("mo", "mos"), ("mois", "mois")),
        _ => (31536000, ("yr", "yrs"), ("an", "ans")),
    };
    let n = (seconds + scale / 2) / scale;
    let units = if fr { french } else { english };
    let prefix = if delta >= 0 {
        ""
    } else if fr {
        "dans "
    } else {
        "in "
    };
    let unit = if n == 1 { units.0 } else { units.1 };
    format!("{prefix}{n} {unit}")
}

pub struct Row<'a> {
    pub commit: Option<&'a Commit>,
    pub saved: Option<&'a crate::local_file_history::Entry>,
    pub label: &'a str,
    pub age: Option<String>,
}

impl Row<'_> {
    pub fn key(&self) -> Option<String> {
        self.saved
            .map(|s| format!("local:{}", s.id))
            .or_else(|| self.commit.map(|c| c.id.clone()))
    }
}
impl History {
    pub fn rows(&self, now: i64, fr: bool) -> Vec<Row<'_>> {
        let working = self.dirty.then_some((now.saturating_mul(1000), None, None));
        let mut entries: Vec<_> = working
            .into_iter()
            .chain(
                self.commits
                    .iter()
                    .map(|c| (c.time.saturating_mul(1000), Some(c), None)),
            )
            .chain(self.saves.iter().map(|e| (e.timestamp, None, Some(e))))
            .collect();
        entries.sort_by_key(|(time, _, _)| std::cmp::Reverse(*time));
        let mut previous = String::new();
        entries
            .into_iter()
            .map(|(time, commit, saved)| {
                let age = relative_time(time.div_euclid(1000), now, fr);
                let visible = (age != previous).then(|| age.clone());
                previous = age;
                let label = match commit {
                    Some(c) => c.message.lines().next().unwrap_or("").trim(),
                    None if saved.is_some() => {
                        if fr {
                            "Fichier enregistré"
                        } else {
                            "File Saved"
                        }
                    }
                    None if fr => "Modifications non commitées",
                    None => "Uncommitted Changes",
                };
                Row {
                    saved,
                    commit,
                    label,
                    age: visible,
                }
            })
            .collect()
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
    ui.menu_button(
        if fr {
            "Filtrer la chronologie"
        } else {
            "Filter Timeline"
        },
        |ui| {
            for source in [Source::Git, Source::Local] {
                if ui
                    .selectable_label(source.enabled(snap.sources), source.label(fr))
                    .clicked()
                {
                    action = Some(Action::ToggleSource(source));
                    ui.close();
                }
            }
        },
    );
    if let Some(file) = &snap.file {
        ui.horizontal(|ui| {
            ui.label(snap.file_label(file, fr));
            if ui
                .selectable_label(snap.pinned, snap.pin_label(fr))
                .clicked()
            {
                action = Some(Action::TogglePin(file.clone()));
            }
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
                for row in history.rows(chrono::Utc::now().timestamp(), fr) {
                    let r = ui
                        .horizontal(|ui| {
                            let mut response = ui.selectable_label(false, row.label);
                            if let Some(c) = row.commit {
                                response |= ui.selectable_label(false, &c.author);
                            }
                            if let Some(age) = &row.age {
                                response |= ui.selectable_label(false, age.as_str());
                            }
                            response
                        })
                        .inner;
                    let r = if let Some(c) = row.commit {
                        r.on_hover_text(format!(
                            "{} <{}>\n{}\n{}",
                            c.author, c.email, c.id, c.message
                        ))
                    } else {
                        r
                    };
                    if r.clicked() {
                        action = Some(Action::Open(file.clone(), row.key(), r.double_clicked()));
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
    #[test]
    fn file_timeline_relative_time_rounds_and_groups_like_vue() {
        for (delta, en, fr) in [
            (-300, "in 5 mins", "dans 5 min"),
            (-30, "now", "maintenant"),
            (29, "now", "maintenant"),
            (30, "30 secs", "30 s"),
            (89, "1 min", "1 min"),
            (90, "2 mins", "2 min"),
            (3599, "60 mins", "60 min"),
            (3600, "1 hr", "1 h"),
            (86400, "1 day", "1 jour"),
            (604800, "1 wk", "1 sem."),
            (2592000, "1 mo", "1 mois"),
            (63072000, "2 yrs", "2 ans"),
        ] {
            assert_eq!(relative_time(0, delta, false), en);
            assert_eq!(relative_time(0, delta, true), fr);
        }
        let commit = |time: i64| Commit {
            id: time.to_string(),
            author: String::new(),
            email: String::new(),
            time,
            message: " title\nbody".into(),
            path: String::new(),
            old_path: None,
        };
        let h = History {
            scm_unavailable: false,
            saves: Vec::new(),
            root: PathBuf::new(),
            relative: String::new(),
            commits: vec![commit(300), commit(0), commit(-2)],
            dirty: true,
            untracked: false,
            more: false,
        };
        let rows = h.rows(0, false);
        assert_eq!(
            rows.iter()
                .map(|r| (r.label, r.age.as_deref()))
                .collect::<Vec<_>>(),
            [
                ("title", Some("in 5 mins")),
                ("Uncommitted Changes", Some("now")),
                ("title", None),
                ("title", None)
            ]
        );
    }

    #[test]
    fn file_timeline_age_tick_repaints_without_reading_git_again() {
        let mut state = State {
            last_age_update: Some(std::time::Instant::now() - std::time::Duration::from_secs(30)),
            ..Default::default()
        };
        assert!(state.poll());
        assert!(state.pending.is_none());
        assert!(!state.poll());
        state.refresh(false);
        assert!(state.last_age_update.is_none());
    }

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
    fn file_timeline_loads_only_selected_sources() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("project");
        std::fs::create_dir(&repo).unwrap();
        let file = repo.join("file.txt");
        std::fs::write(&file, "one").unwrap();
        git_fixture(&repo, &["init", "-q"]);
        git_fixture(&repo, &["add", "."]);
        git_fixture(&repo, &["commit", "-qm", "first"]);
        let profile = dir.path().join("profile");
        crate::local_file_history::record(&profile, &file, "one").unwrap();
        std::fs::write(&file, "two").unwrap();
        for git in [false, true] {
            for local in [false, true] {
                let data = load(&file, 20, Some(&profile), Sources { git, local }).unwrap();
                assert_eq!(data.commits.len(), usize::from(git));
                assert_eq!(data.dirty, git);
                assert_eq!(data.saves.len(), usize::from(local));
            }
        }
    }

    #[test]
    fn file_timeline_abandons_old_worker_when_active_file_changes() {
        let (tx, rx) = mpsc::channel();
        let mut state = State {
            pending: Some((0, rx)),
            file: Some(PathBuf::from("previous")),
            ..State::default()
        };
        state.select(None, Sources::default());
        tx.send(Err("stale error".into())).unwrap();
        assert!(state.poll());
        assert!(state.file.is_none() && state.data.is_none() && state.pending.is_none());
    }
}
