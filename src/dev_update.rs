//! Dev-checkout Git build tracker & fast-forward updater (`bXXXX (7chars)` + `↑ bXXXX` pill).
//!
//! Parity with `dev-update`:
//! - Refreshes the live local `HEAD` build count + short hash every `15 s` in Git checkouts.
//! - Fetches upstream after `90 s`, then every `30 min`.
//! - Computes incoming files (`git diff --name-only HEAD..@{u}`) vs locally modified tracked files
//!   (`git status --porcelain --untracked-files=no`), blocking fast-forward ONLY when local changes
//!   overlap incoming files (or when `HEAD` is ahead of `@{u}`).
//! - Applies updates via `git merge --ff-only --quiet @{u}`, keeping non-overlapping local edits intact.

#![allow(dead_code)]

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const BUILD_REFRESH_EVERY: Duration = Duration::from_secs(15);
pub const INITIAL_FETCH_DELAY: Duration = Duration::from_secs(90);
pub const FETCH_EVERY: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DevUpdateBlocked {
    Ahead { count: usize },
    Dirty { overlap: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevUpdateState {
    pub local_build: u64,
    pub local_commit: String,
    pub ahead: usize,
    pub behind: usize,
    pub remote_build: u64,
    pub incoming_files: Vec<String>,
    pub incoming_log: Vec<String>,
    pub blocked: Option<DevUpdateBlocked>,
    pub last_checked_unix_ms: u64,
}

impl DevUpdateState {
    #[must_use]
    pub fn build_label(&self) -> String {
        format!("b{} ({})", self.local_build, self.local_commit)
    }

    #[must_use]
    pub fn pill_label(&self) -> Option<String> {
        (self.behind > 0).then(|| format!("↑ b{}", self.remote_build))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyDevUpdateResult {
    Updated(DevUpdateState),
    UpToDate(DevUpdateState),
    Ahead { count: usize },
    Dirty { overlap: Vec<String> },
    Error(String),
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn normalize_rel_path(raw: &str) -> String {
    unquote_git_path(raw.trim()).replace('\\', "/")
}

fn unquote_git_path(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let inner = &s[1..s.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                if let Some(next) = chars.next() {
                    match next {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        other => {
                            out.push('\\');
                            out.push(other);
                        }
                    }
                } else {
                    out.push('\\');
                }
            } else {
                out.push(ch);
            }
        }
        out
    } else {
        s.to_owned()
    }
}

fn is_porcelain_status_char(b: u8) -> bool {
    matches!(
        b,
        b' ' | b'M' | b'A' | b'D' | b'R' | b'C' | b'U' | b'T' | b'?' | b'!'
    )
}

fn strip_porcelain_prefix(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    // Standard 2-column porcelain (`XY path`), e.g. `" M src/a.rs"`, `"MM src/a.rs"`, `"R  a -> b"`
    if bytes.len() >= 4
        && is_porcelain_status_char(bytes[0])
        && is_porcelain_status_char(bytes[1])
        && bytes[2] == b' '
    {
        return Some(&line[3..]);
    }
    // Trimmed-leading-space first line (`"M src/a.rs"` when a caller `.trim()`s stdout)
    if bytes.len() >= 3
        && bytes[0] != b' '
        && is_porcelain_status_char(bytes[0])
        && bytes[1] == b' '
    {
        return Some(&line[2..]);
    }
    None
}

/// Parses `git status --porcelain --untracked-files=no` output into normalized repo-relative paths.
/// Handles trimmed leading spaces on the first line (`"M src/a.rs\n M src/b.rs"`) and returns both
/// old and new paths on renames (`"R  old -> new"`).
#[must_use]
pub fn porcelain_paths(out: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut push_unique = |p: String| {
        if !p.is_empty() && !paths.contains(&p) {
            paths.push(p);
        }
    };

    for raw_line in out.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let Some(rest) = strip_porcelain_prefix(line) else {
            continue;
        };
        if let Some((old_raw, new_raw)) = rest.split_once(" -> ") {
            push_unique(normalize_rel_path(old_raw));
            push_unique(normalize_rel_path(new_raw));
        } else {
            push_unique(normalize_rel_path(rest));
        }
    }
    paths
}

/// Returns locally modified files that intersect `incoming` upstream changes.
#[must_use]
pub fn compute_overlap(edited: &[String], incoming: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for item in edited {
        let norm = item.replace('\\', "/");
        if incoming.iter().any(|inc| inc.replace('\\', "/") == norm) && !out.contains(&norm) {
            out.push(norm);
        }
    }
    out
}

/// Formats the overlap block message in EN/FR using `«file»` markers.
#[must_use]
pub fn format_overlap_message(overlap: &[String], french: bool) -> String {
    let list = overlap
        .iter()
        .map(|f| format!("«{f}»"))
        .collect::<Vec<_>>()
        .join(", ");
    if french {
        format!(
            "Vos modifications locales touchent des fichiers entrants : {list}. Commitez ou mettez-les de côté (stash), ou lancez git pull --ff-only vous-même."
        )
    } else {
        format!(
            "Local changes touch incoming files: {list}. Commit or stash them, or run git pull --ff-only yourself."
        )
    }
}

fn git_cmd(repo_dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    crate::agent_env::clean_command(&mut cmd);
    cmd.arg("-C")
        .arg(repo_dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let output = cmd.output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end_matches(['\r', '\n'])
            .to_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Err(if !stderr.is_empty() { stderr } else { stdout })
    }
}

/// Inspects the local Git checkout and its upstream tracking branch without network I/O.
#[must_use]
pub fn look(repo_dir: &Path) -> Option<DevUpdateState> {
    if !repo_dir.join(".git").exists() {
        return None;
    }
    let local_build = git_cmd(repo_dir, &["rev-list", "--count", "HEAD"])
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    let local_commit = git_cmd(repo_dir, &["rev-parse", "--short=7", "HEAD"])
        .ok()?
        .trim()
        .to_owned();
    if local_commit.is_empty() {
        return None;
    }

    let (ahead, behind) = match git_cmd(
        repo_dir,
        &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
    ) {
        Ok(counts) => {
            let mut parts = counts.split_whitespace();
            let a = parts
                .next()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            let b = parts
                .next()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            (a, b)
        }
        Err(_) => (0, 0),
    };

    let porcelain =
        git_cmd(repo_dir, &["status", "--porcelain", "--untracked-files=no"]).unwrap_or_default();
    let edited = porcelain_paths(&porcelain);

    let mut incoming_files = Vec::new();
    let mut incoming_log = Vec::new();
    let mut blocked = None;

    if behind > 0 {
        if let Ok(diff_out) = git_cmd(repo_dir, &["diff", "--name-only", "HEAD..@{u}"]) {
            for line in diff_out.lines() {
                let norm = normalize_rel_path(line);
                if !norm.is_empty() && !incoming_files.contains(&norm) {
                    incoming_files.push(norm);
                }
            }
        }
        if let Ok(log_out) = git_cmd(repo_dir, &["log", "--oneline", "-n", "8", "HEAD..@{u}"]) {
            incoming_log = log_out
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect();
        }
        let overlap = compute_overlap(&edited, &incoming_files);
        if !overlap.is_empty() {
            blocked = Some(DevUpdateBlocked::Dirty { overlap });
        } else if ahead > 0 {
            blocked = Some(DevUpdateBlocked::Ahead { count: ahead });
        }
    }

    Some(DevUpdateState {
        local_build,
        local_commit,
        ahead,
        behind,
        remote_build: local_build.saturating_add(behind as u64),
        incoming_files,
        incoming_log,
        blocked,
        last_checked_unix_ms: now_unix_ms(),
    })
}

/// Fetches upstream (`git fetch --quiet`) and returns the updated [`DevUpdateState`].
#[must_use]
pub fn fetch_and_look(repo_dir: &Path) -> Option<DevUpdateState> {
    if !repo_dir.join(".git").exists() {
        return None;
    }
    let _ = git_cmd(repo_dir, &["fetch", "--quiet"]);
    look(repo_dir)
}

/// Fast-forwards the checkout to `@{u}` using `git merge --ff-only --quiet @{u}` when safe.
/// Uncommitted changes to files outside `HEAD..@{u}` are preserved.
pub fn apply(repo_dir: &Path) -> ApplyDevUpdateResult {
    let Some(state) = look(repo_dir) else {
        return ApplyDevUpdateResult::Error("Not a Git checkout.".into());
    };
    if state.behind == 0 {
        return ApplyDevUpdateResult::UpToDate(state);
    }
    if let Some(DevUpdateBlocked::Dirty { overlap }) = state.blocked {
        return ApplyDevUpdateResult::Dirty { overlap };
    }
    if state.ahead > 0 {
        return ApplyDevUpdateResult::Ahead { count: state.ahead };
    }
    match git_cmd(repo_dir, &["merge", "--ff-only", "--quiet", "@{u}"]) {
        Ok(_) => match look(repo_dir) {
            Some(updated) => ApplyDevUpdateResult::Updated(updated),
            None => ApplyDevUpdateResult::Updated(state),
        },
        Err(err) => ApplyDevUpdateResult::Error(err),
    }
}

/// Locates the Tessel source Git checkout when running from `cargo run` or a local dev binary.
#[must_use]
pub fn detect_dev_repo_dir() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        for anc in exe.ancestors().skip(1).take(5) {
            if anc.join(".git").exists() && anc.join("Cargo.toml").exists() {
                return Some(anc.to_path_buf());
            }
        }
    }
    if cfg!(debug_assertions) {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        if manifest_dir.join(".git").exists() {
            return Some(manifest_dir);
        }
    }
    None
}

enum WorkerMsg {
    Look(Option<DevUpdateState>),
    Applied(ApplyDevUpdateResult),
}

pub struct DevUpdatePoller {
    repo_dir: Option<PathBuf>,
    state: Option<DevUpdateState>,
    rx: Option<Receiver<WorkerMsg>>,
    busy_applying: bool,
    started_at: Instant,
    last_look_at: Option<Instant>,
    last_fetch_at: Option<Instant>,
}

impl DevUpdatePoller {
    #[must_use]
    pub fn new(probe_environment: bool) -> Self {
        let repo_dir = if probe_environment {
            detect_dev_repo_dir()
        } else {
            None
        };
        let mut poller = Self {
            repo_dir,
            state: None,
            rx: None,
            busy_applying: false,
            started_at: Instant::now(),
            last_look_at: None,
            last_fetch_at: None,
        };
        poller.spawn_look(false);
        poller
    }

    #[must_use]
    pub fn state(&self) -> Option<&DevUpdateState> {
        self.state.as_ref()
    }

    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.rx.is_some()
    }

    #[must_use]
    pub fn is_applying(&self) -> bool {
        self.busy_applying
    }

    #[must_use]
    pub fn live_build_label(&self) -> String {
        self.state
            .as_ref()
            .map(DevUpdateState::build_label)
            .unwrap_or_else(|| env!("TESSEL_BUILD").to_owned())
    }

    pub fn set_state_for_test(&mut self, state: Option<DevUpdateState>) {
        self.state = state;
    }

    pub fn refresh_now(&mut self, with_fetch: bool) {
        self.spawn_look(with_fetch);
    }

    pub fn apply_now(&mut self) {
        if self.rx.is_some() {
            return;
        }
        let Some(dir) = self.repo_dir.clone() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        self.busy_applying = true;
        thread::spawn(move || {
            let _ = tx.send(WorkerMsg::Applied(apply(&dir)));
        });
        self.rx = Some(rx);
    }

    fn spawn_look(&mut self, with_fetch: bool) {
        if self.rx.is_some() {
            return;
        }
        let Some(dir) = self.repo_dir.clone() else {
            return;
        };
        let now = Instant::now();
        self.last_look_at = Some(now);
        if with_fetch {
            self.last_fetch_at = Some(now);
        }
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let st = if with_fetch {
                fetch_and_look(&dir)
            } else {
                look(&dir)
            };
            let _ = tx.send(WorkerMsg::Look(st));
        });
        self.rx = Some(rx);
    }

    /// Polls background Git checks and returns `Some(ApplyDevUpdateResult)` if an `apply_now()` completed.
    pub fn poll(&mut self) -> Option<ApplyDevUpdateResult> {
        let mut applied_out = None;
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(WorkerMsg::Look(st)) => {
                    if st.is_some() {
                        self.state = st;
                    }
                    self.rx = None;
                    self.busy_applying = false;
                }
                Ok(WorkerMsg::Applied(res)) => {
                    match &res {
                        ApplyDevUpdateResult::Updated(st) | ApplyDevUpdateResult::UpToDate(st) => {
                            self.state = Some(st.clone());
                        }
                        _ => {}
                    }
                    self.rx = None;
                    self.busy_applying = false;
                    applied_out = Some(res);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.rx = None;
                    self.busy_applying = false;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        if self.rx.is_none() && self.repo_dir.is_some() {
            let now = Instant::now();
            let due_fetch = match self.last_fetch_at {
                None => now.duration_since(self.started_at) >= INITIAL_FETCH_DELAY,
                Some(prev) => now.duration_since(prev) >= FETCH_EVERY,
            };
            if due_fetch {
                self.spawn_look(true);
            } else {
                let due_look = self
                    .last_look_at
                    .is_none_or(|prev| now.duration_since(prev) >= BUILD_REFRESH_EVERY);
                if due_look {
                    self.spawn_look(false);
                }
            }
        }

        applied_out
    }
}
