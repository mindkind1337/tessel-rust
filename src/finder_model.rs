//! Pure data models and exact behavioral parity logic for:
//! - Fuzzy file scoring & filtering (`C:\Tessel\src\shared\fuzzy.js`)
//! - Bounded pure-Rust project file walk & `FileFinder` (`C:\Tessel\src\main\fileOpen.js`, `FileFinder.vue`)
//! - Empty-grid `ProjectLauncher`, remote drop retry & recent sessions (`projectLauncher.js`, `ProjectLauncher.vue`, `sessionHistory.js`)
//! - Toolbar/pane `LaunchMenu` placement, positioning & keyboard navigation (`LaunchMenu.vue`)

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LauncherLang {
    #[default]
    En,
    Fr,
}

// ---------------------------------------------------------------------------
// 1. Exact Fuzzy File Matcher (`C:\Tessel\src\shared\fuzzy.js`)
// ---------------------------------------------------------------------------

pub const FILE_FINDER_LIMIT: usize = 60;
pub const MAX_FINDER_QUERY_CHARS: usize = 256;

#[must_use]
pub fn clamp_finder_query(query: &str) -> String {
    query.chars().take(MAX_FINDER_QUERY_CHARS).collect()
}

// ECMAScript whitespace includes the byte-order mark and excludes NEXT LINE.
fn finder_space(ch: char) -> bool {
    ch == '\u{feff}' || (ch != '\u{0085}' && ch.is_whitespace())
}

fn finder_query(query: &str) -> String {
    query
        .to_lowercase()
        .chars()
        .filter(|&c| !finder_space(c))
        .collect()
}

#[must_use]
pub fn fuzzy_score(query: &str, path: &str) -> f64 {
    score_normalized_query(&finder_query(query), path)
}

fn score_normalized_query(query: &str, path: &str) -> f64 {
    if query.is_empty() {
        return 0.;
    }
    let folded = path.to_lowercase();
    let name = folded.rsplit(['/', '\\']).next().unwrap_or_default();
    // Vue iterates query code points but indexes both path strings in UTF-16 units.
    // Lowercasing can change length, so keep the original positions for word boundaries.
    let lower: Vec<u16> = folded.encode_utf16().collect();
    let original: Vec<u16> = path.encode_utf16().collect();
    let name_start = lower.len() - name.encode_utf16().count();
    let mut from = 0;
    let mut run = 0.0;
    let mut score = 0.0;
    for ch in query.chars() {
        let mut buffer = [0; 2];
        let needle = ch.encode_utf16(&mut buffer);
        let Some(offset) = lower[from..]
            .windows(needle.len())
            .position(|w| w == needle)
        else {
            return -1.;
        };
        let i = from + offset;
        run = if offset == 0 { run + 1.0 } else { 0.0 };
        score += 1.0 + run * 3.0;
        if i >= name_start {
            score += 2.0;
        }
        let boundary = i == 0
            || original
                .get(i - 1)
                .and_then(|&c| char::from_u32(c.into()))
                .is_some_and(|c| matches!(c, '/' | '\\' | '.' | '_' | '-') || finder_space(c))
            || (original.get(i) != lower.get(i) && original.get(i - 1) == lower.get(i - 1));
        if boundary {
            score += 3.0;
        }
        from = i + 1;
    }
    if name.contains(query) {
        score += 20.0;
    }
    if name.rsplit_once('.').map_or(name, |(stem, _)| stem) == query {
        score += 10.0;
    }
    score - lower.len() as f64 * 0.01
}

#[must_use]
pub fn fuzzy_filter(query: &str, paths: &[String], limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let query = finder_query(query);
    if query.is_empty() {
        return paths.iter().take(limit).cloned().collect();
    }

    let mut hits: Vec<(usize, f64, &String)> = paths
        .iter()
        .enumerate()
        .filter_map(|(idx, p)| {
            let s = score_normalized_query(&query, p);
            (s >= 0.0).then_some((idx, s, p))
        })
        .collect();

    hits.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    hits.into_iter()
        .take(limit)
        .map(|(_, _, p)| p.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// 2. Bounded Pure-Rust Project File Walk & `FileFinderState` (`fileOpen.js`, `FileFinder.vue`)
// ---------------------------------------------------------------------------

pub const MAX_LISTED_FILES: usize = 20_000;
pub const DEFAULT_MAX_VISITED_ENTRIES: usize = 100_000;
pub const DEFAULT_MAX_WALKED_DIRS: usize = 10_000;
pub const DEFAULT_MAX_ENTRIES_PER_DIR: usize = 25_000;
pub const DEFAULT_MAX_WALK_DEPTH: usize = 32;
pub const DEFAULT_MAX_TOTAL_PATH_BYTES: usize = 8 * 1024 * 1024;
pub const DEFAULT_MAX_SINGLE_PATH_BYTES: usize = 1_024;
pub const DEFAULT_WALK_TIMEOUT: Duration = Duration::from_secs(5);
pub const WIN_FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

pub const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    "dist",
    "out",
    "build",
    ".next",
    ".cache",
    "target",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkStopReason {
    MaxFiles,
    MaxVisitedEntries,
    MaxDirs,
    MaxDirEntries,
    MaxDepth,
    MaxTotalPathBytes,
    MaxSinglePathBytes,
    Timeout,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkLimits {
    pub max_files: usize,
    pub max_visited_entries: usize,
    pub max_dirs: usize,
    pub max_dir_entries: usize,
    pub max_depth: usize,
    pub max_total_path_bytes: usize,
    pub max_single_path_bytes: usize,
    /// Cooperative wall-clock budget checked between directory/entry I/O steps
    /// (does not preempt an OS-blocked filesystem syscall).
    pub timeout: Duration,
}

impl Default for WalkLimits {
    fn default() -> Self {
        Self {
            max_files: MAX_LISTED_FILES,
            max_visited_entries: DEFAULT_MAX_VISITED_ENTRIES,
            max_dirs: DEFAULT_MAX_WALKED_DIRS,
            max_dir_entries: DEFAULT_MAX_ENTRIES_PER_DIR,
            max_depth: DEFAULT_MAX_WALK_DEPTH,
            max_total_path_bytes: DEFAULT_MAX_TOTAL_PATH_BYTES,
            max_single_path_bytes: DEFAULT_MAX_SINGLE_PATH_BYTES,
            timeout: DEFAULT_WALK_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFileList {
    pub files: Vec<String>,
    pub total_path_bytes: usize,
    pub truncated: bool,
    pub stop_reason: Option<WalkStopReason>,
}

#[must_use]
pub const fn has_reparse_point_attr(file_attributes: u32) -> bool {
    (file_attributes & WIN_FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

/// Enumeration-side canonical containment check (`candidate_canon` inside `canon_root`).
/// Note: this guards directory/file listing only; final file open/read access remains
/// independently validated by `files.rs`.
#[must_use]
pub fn is_within_canonical_scope(canon_root: &Path, candidate_canon: &Path) -> bool {
    candidate_canon.starts_with(canon_root)
}

fn is_path_symlink_or_reparse(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return true;
    };
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if has_reparse_point_attr(meta.file_attributes()) {
            return true;
        }
    }
    false
}

fn is_symlink_or_reparse(path: &Path, ft: &std::fs::FileType) -> bool {
    ft.is_symlink() || is_path_symlink_or_reparse(path)
}

/// Pure-Rust bounded directory walk matching `fileOpen.js` (`SKIP_DIRS`, symlink/junction skip,
/// canonical root containment, forward-slash relative paths, lexicographic sort).
/// Never spawns an external process, clamps `max_files <= MAX_LISTED_FILES`, and bounds
/// visited entries, directories, depth, and cooperative wall-clock duration.
pub fn walk_project_files(root: &Path, max_files: usize) -> Result<ProjectFileList, String> {
    let limits = WalkLimits {
        max_files: max_files.min(MAX_LISTED_FILES),
        ..WalkLimits::default()
    };
    walk_project_files_with_limits(root, &limits, None)
}

/// Bounded directory enumeration with explicit `WalkLimits` and cooperative `cancel`/`timeout`
/// checks between I/O calls (cannot interrupt a kernel-stalled filesystem call).
/// Revalidates symlink/reparse status and canonical `canon_root` scope both when queueing
/// subdirectories and immediately after popping each directory before `read_dir`.
/// Final file open/read operations remain validated by `files.rs`.
pub fn walk_project_files_with_limits(
    root: &Path,
    limits: &WalkLimits,
    cancel: Option<&AtomicBool>,
) -> Result<ProjectFileList, String> {
    let meta = std::fs::symlink_metadata(root)
        .map_err(|_| "The project folder is missing.".to_string())?;
    if !meta.is_dir() || is_path_symlink_or_reparse(root) {
        return Err("The project folder is missing.".to_string());
    }
    let canon_root =
        std::fs::canonicalize(root).map_err(|_| "The project folder is missing.".to_string())?;

    let effective_max_files = limits.max_files.min(MAX_LISTED_FILES);
    let started = Instant::now();

    let mut files: Vec<String> = Vec::new();
    let mut total_path_bytes = 0usize;
    let mut truncated = false;
    let mut stop_reason: Option<WalkStopReason> = None;
    let mut visited_entries = 0usize;
    let mut queued_dirs = 1usize; // root counts as the first directory
    let mut visited_canon_dirs: HashSet<PathBuf> = HashSet::new();
    visited_canon_dirs.insert(canon_root.clone());

    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    'walk: while let Some((dir, depth)) = stack.pop() {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            truncated = true;
            stop_reason = Some(WalkStopReason::Cancelled);
            break 'walk;
        }
        if started.elapsed() >= limits.timeout {
            truncated = true;
            stop_reason = Some(WalkStopReason::Timeout);
            break 'walk;
        }

        // Revalidate popped directory right before `read_dir` against TOCTOU junction/symlink swaps.
        if is_path_symlink_or_reparse(&dir) {
            continue;
        }
        let Ok(canon_dir) = std::fs::canonicalize(&dir) else {
            continue;
        };
        if !is_within_canonical_scope(&canon_root, &canon_dir) {
            continue;
        }

        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };

        let mut dir_entries: Vec<std::fs::DirEntry> = Vec::new();
        for res in read_dir {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                truncated = true;
                stop_reason = Some(WalkStopReason::Cancelled);
                break 'walk;
            }
            if started.elapsed() >= limits.timeout {
                truncated = true;
                stop_reason = Some(WalkStopReason::Timeout);
                break 'walk;
            }
            let Ok(entry) = res else {
                continue;
            };
            if visited_entries >= limits.max_visited_entries {
                truncated = true;
                stop_reason = Some(WalkStopReason::MaxVisitedEntries);
                break 'walk;
            }
            if dir_entries.len() >= limits.max_dir_entries {
                truncated = true;
                stop_reason.get_or_insert(WalkStopReason::MaxDirEntries);
                break;
            }
            visited_entries += 1;
            dir_entries.push(entry);
        }

        dir_entries.sort_by_key(std::fs::DirEntry::file_name);

        for entry in dir_entries {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                truncated = true;
                stop_reason = Some(WalkStopReason::Cancelled);
                break 'walk;
            }
            if started.elapsed() >= limits.timeout {
                truncated = true;
                stop_reason = Some(WalkStopReason::Timeout);
                break 'walk;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            let full = entry.path();
            if is_symlink_or_reparse(&full, &ft) {
                continue;
            }
            if ft.is_dir() {
                let name = entry.file_name();
                let skip = name.to_str().is_some_and(|n| SKIP_DIRS.contains(&n));
                if skip {
                    continue;
                }
                let Ok(canon_sub) = std::fs::canonicalize(&full) else {
                    continue;
                };
                if !is_within_canonical_scope(&canon_root, &canon_sub) {
                    continue;
                }
                if depth + 1 > limits.max_depth {
                    truncated = true;
                    stop_reason.get_or_insert(WalkStopReason::MaxDepth);
                    continue;
                }
                if queued_dirs >= limits.max_dirs {
                    truncated = true;
                    stop_reason.get_or_insert(WalkStopReason::MaxDirs);
                    continue;
                }
                if !visited_canon_dirs.insert(canon_sub) {
                    continue;
                }
                queued_dirs += 1;
                stack.push((full, depth + 1));
            } else if ft.is_file() {
                let Ok(canon_file) = std::fs::canonicalize(&full) else {
                    continue;
                };
                if !is_within_canonical_scope(&canon_root, &canon_file) {
                    continue;
                }
                if files.len() >= effective_max_files {
                    truncated = true;
                    stop_reason = Some(WalkStopReason::MaxFiles);
                    break 'walk;
                }
                if let Ok(rel) = full.strip_prefix(root) {
                    let normalized = rel.to_string_lossy().replace('\\', "/");
                    if !normalized.is_empty() {
                        if normalized.len() > limits.max_single_path_bytes {
                            truncated = true;
                            stop_reason.get_or_insert(WalkStopReason::MaxSinglePathBytes);
                            continue;
                        }
                        if total_path_bytes.saturating_add(normalized.len())
                            > limits.max_total_path_bytes
                        {
                            truncated = true;
                            stop_reason = Some(WalkStopReason::MaxTotalPathBytes);
                            break 'walk;
                        }
                        total_path_bytes += normalized.len();
                        files.push(normalized);
                    }
                }
            }
        }
    }

    files.sort();
    Ok(ProjectFileList {
        files,
        total_path_bytes,
        truncated,
        stop_reason,
    })
}

/// Enforces at most one active background project-file scan at a time with cooperative
/// cancellation (prevents unbounded worker spawning if the user switches projects rapidly).
#[derive(Debug, Default)]
pub struct FinderScanCoordinator {
    seq: u64,
    active: Option<(u64, Arc<AtomicBool>)>,
    pending_restart: bool,
}

impl FinderScanCoordinator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// Requests a scan. Returns `Some((scan_id, cancel_token))` only when no worker is
    /// currently running; otherwise signals cancellation on the running worker and coalesces
    /// a single follow-up scan to start once `finish_scan` is called.
    pub fn request_scan(&mut self) -> Option<(u64, Arc<AtomicBool>)> {
        if let Some((_, cancel)) = &self.active {
            cancel.store(true, Ordering::Relaxed);
            self.pending_restart = true;
            return None;
        }
        self.seq += 1;
        let token = Arc::new(AtomicBool::new(false));
        self.active = Some((self.seq, Arc::clone(&token)));
        Some((self.seq, token))
    }

    /// Completes `scan_id`. Returns `(was_latest, optional_replacement_scan)`.
    pub fn finish_scan(&mut self, scan_id: u64) -> (bool, Option<(u64, Arc<AtomicBool>)>) {
        let Some((active_id, cancel)) = &self.active else {
            return (false, None);
        };
        if *active_id != scan_id {
            return (false, None);
        }
        let was_cancelled = cancel.load(Ordering::Relaxed);
        self.active = None;
        if self.pending_restart {
            self.pending_restart = false;
            let next = self.request_scan();
            return (!was_cancelled && next.is_none(), next);
        }
        (!was_cancelled, None)
    }

    pub fn cancel_active(&mut self) {
        self.pending_restart = false;
        if let Some((_, cancel)) = &self.active {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[must_use]
pub fn name_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(idx) => &path[idx + 1..],
        None => path,
    }
}

#[must_use]
pub fn dir_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(idx) if idx > 0 => &path[..idx],
        _ => "",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileFinderAction {
    MoveSelection(usize),
    OpenSelected(String),
    InsertSelected(String),
    Close,
    Noop,
}

#[derive(Debug, Clone)]
pub struct FileFinderState {
    files: Vec<String>,
    pub truncated: bool,
    query: String,
    pub selected: usize,
    pub can_insert: bool,
    pub limit: usize,
    cached_results: Vec<String>,
    recompute_count: usize,
}

impl Default for FileFinderState {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            truncated: false,
            query: String::new(),
            selected: 0,
            can_insert: false,
            limit: FILE_FINDER_LIMIT,
            cached_results: Vec::new(),
            recompute_count: 0,
        }
    }
}

impl FileFinderState {
    #[must_use]
    pub fn new(files: Vec<String>, truncated: bool, can_insert: bool) -> Self {
        let cached_results = fuzzy_filter("", &files, FILE_FINDER_LIMIT);
        Self {
            files,
            truncated,
            query: String::new(),
            selected: 0,
            can_insert,
            limit: FILE_FINDER_LIMIT,
            cached_results,
            recompute_count: 1,
        }
    }

    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    #[must_use]
    pub fn files(&self) -> &[String] {
        &self.files
    }

    #[must_use]
    pub fn results(&self) -> &[String] {
        &self.cached_results
    }

    #[must_use]
    pub const fn recompute_count(&self) -> usize {
        self.recompute_count
    }

    pub fn set_files(&mut self, list: ProjectFileList) {
        self.files = list.files;
        self.truncated = list.truncated;
        self.selected = 0;
        self.cached_results = fuzzy_filter(&self.query, &self.files, self.limit);
        self.recompute_count += 1;
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        let bounded = clamp_finder_query(&query.into());
        if bounded == self.query {
            return;
        }
        self.query = bounded;
        self.selected = 0;
        self.cached_results = fuzzy_filter(&self.query, &self.files, self.limit);
        self.recompute_count += 1;
    }

    pub fn handle_key(&mut self, key: &str, ctrl: bool) -> FileFinderAction {
        let len = self.cached_results.len();
        match key {
            "ArrowDown" => {
                if len > 0 {
                    self.selected = (self.selected + 1) % len;
                }
                FileFinderAction::MoveSelection(self.selected)
            }
            "ArrowUp" => {
                if len > 0 {
                    self.selected = (self.selected + len - 1) % len;
                }
                FileFinderAction::MoveSelection(self.selected)
            }
            "Enter" => {
                let Some(picked) = self.cached_results.get(self.selected).cloned() else {
                    return FileFinderAction::Noop;
                };
                if ctrl && self.can_insert {
                    FileFinderAction::InsertSelected(picked)
                } else {
                    FileFinderAction::OpenSelected(picked)
                }
            }
            "Escape" => FileFinderAction::Close,
            _ => FileFinderAction::Noop,
        }
    }
}

// ---------------------------------------------------------------------------
// 3. `projectLauncher.js`, Session Filtering & `ProjectLauncherState`
// ---------------------------------------------------------------------------

pub const REMOTE_AGENT_IDS: [&str; 17] = [
    "claude",
    "openclaude",
    "codex",
    "gemini",
    "opencode",
    "copilot",
    "kimi",
    "cursor",
    "droid",
    "grok",
    "pi",
    "omp",
    "qwen",
    "devin",
    "zcode",
    "qoder",
    "muse",
];

pub const ALL_HISTORY_AGENTS: [&str; 19] = [
    "claude",
    "codex",
    "gemini",
    "qwen",
    "opencode",
    "openclaude",
    "copilot",
    "kimi",
    "cline",
    "cursor",
    "droid",
    "grok",
    "pi",
    "omp",
    "antigravity",
    "devin",
    "zcode",
    "qoder",
    "muse",
];

pub const SSH_DROPPED_EXIT_CODE: i32 = 255;
pub const DROP_RETRIES: usize = 3;
pub const DROP_RETRY_WINDOW_MS: u64 = 5 * 60 * 1000;

#[must_use]
pub fn agent_display_name(agent_id: &str) -> &str {
    match agent_id {
        "claude" => "Claude Code",
        "codex" => "Codex",
        "gemini" => "Gemini",
        "qwen" => "Qwen",
        "opencode" => "OpenCode",
        "openclaude" => "OpenClaude",
        "copilot" => "Copilot",
        "kimi" => "Kimi",
        "cline" => "Cline",
        "cursor" => "Cursor",
        "droid" => "Droid",
        "grok" => "Grok",
        "pi" => "Pi",
        "omp" => "OMP",
        "antigravity" => "Antigravity",
        "devin" => "Devin",
        "zcode" => "ZCode",
        "qoder" => "Qoder CLI",
        "muse" => "Muse Code",
        other => other,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectOpenSetting {
    Agent(String),
    Shell,
    Browser,
    Empty,
}

#[must_use]
pub fn valid_project_open(value: &str) -> ProjectOpenSetting {
    match value {
        "shell" | "" => {
            if value == "shell" {
                ProjectOpenSetting::Shell
            } else {
                ProjectOpenSetting::Empty
            }
        }
        "browser" => ProjectOpenSetting::Browser,
        "empty" => ProjectOpenSetting::Empty,
        _ => {
            if let Some(id) = value.strip_prefix("agent:")
                && !id.is_empty()
            {
                ProjectOpenSetting::Agent(id.to_string())
            } else {
                ProjectOpenSetting::Empty
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteAgentStatus {
    Found,
    Missing,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostAgentProbe {
    pub found: Option<bool>,
    pub version: Option<String>,
    pub can_install: bool,
    pub install_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCandidate {
    pub id: String,
    pub name: String,
    pub local_found: bool,
    pub version: Option<String>,
    pub can_install: bool,
    pub install_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherAgentView {
    pub id: String,
    pub name: String,
    pub status: RemoteAgentStatus,
    pub version: Option<String>,
    pub can_install: bool,
    pub install_url: Option<String>,
}

#[must_use]
pub fn remote_agent_found(
    host_agents: Option<&HashMap<String, HostAgentProbe>>,
    id: &str,
) -> RemoteAgentStatus {
    let Some(map) = host_agents else {
        return RemoteAgentStatus::Unknown;
    };
    let Some(entry) = map.get(id) else {
        return RemoteAgentStatus::Unknown;
    };
    match entry.found {
        Some(true) => RemoteAgentStatus::Found,
        Some(false) => RemoteAgentStatus::Missing,
        None => RemoteAgentStatus::Unknown,
    }
}

#[must_use]
pub fn launcher_agents(
    agents: &[AgentCandidate],
    is_remote: bool,
    host_agents: Option<&HashMap<String, HostAgentProbe>>,
) -> Vec<LauncherAgentView> {
    if !is_remote {
        return agents
            .iter()
            .filter(|a| a.local_found)
            .map(|a| LauncherAgentView {
                id: a.id.clone(),
                name: a.name.clone(),
                status: RemoteAgentStatus::Found,
                version: a.version.clone(),
                can_install: a.can_install,
                install_url: a.install_url.clone(),
            })
            .collect();
    }

    let remote_set: HashSet<&str> = REMOTE_AGENT_IDS.iter().copied().collect();
    agents
        .iter()
        .filter(|a| remote_set.contains(a.id.as_str()))
        .filter_map(|a| {
            let status = remote_agent_found(host_agents, &a.id);
            let probe = host_agents.and_then(|m| m.get(&a.id));
            if status == RemoteAgentStatus::Missing {
                let can_install = probe.is_some_and(|p| p.can_install);
                let has_url = probe.and_then(|p| p.install_url.as_ref()).is_some();
                if !can_install && !has_url {
                    return None;
                }
            }
            Some(LauncherAgentView {
                id: a.id.clone(),
                name: a.name.clone(),
                status,
                version: probe.and_then(|p| p.version.clone()),
                can_install: probe.is_some_and(|p| p.can_install),
                install_url: probe.and_then(|p| p.install_url.clone()),
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCandidate {
    pub id: String,
    pub name: String,
}

#[must_use]
pub fn launcher_shells(shells: &[ShellCandidate], is_remote: bool) -> Vec<ShellCandidate> {
    if is_remote {
        Vec::new()
    } else {
        shells.to_vec()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherChoice {
    Agent(String),
    Shell(Option<String>),
    Browser,
}

#[must_use]
pub fn project_open_choice(
    project_open: &str,
    agents: &[AgentCandidate],
    default_shell: Option<&str>,
    is_remote: bool,
    host_agents: Option<&HashMap<String, HostAgentProbe>>,
) -> Option<LauncherChoice> {
    match valid_project_open(project_open) {
        ProjectOpenSetting::Empty => None,
        ProjectOpenSetting::Browser => Some(LauncherChoice::Browser),
        ProjectOpenSetting::Shell => Some(LauncherChoice::Shell(if is_remote {
            None
        } else {
            default_shell
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
        })),
        ProjectOpenSetting::Agent(id) => {
            if is_remote {
                let remote_ok = REMOTE_AGENT_IDS.contains(&id.as_str())
                    && agents.iter().any(|a| a.id == id)
                    && remote_agent_found(host_agents, &id) != RemoteAgentStatus::Missing;
                remote_ok.then_some(LauncherChoice::Agent(id))
            } else {
                agents
                    .iter()
                    .any(|a| a.id == id && a.local_found)
                    .then_some(LauncherChoice::Agent(id))
            }
        }
    }
}

#[must_use]
pub fn remembered_value(choice: Option<&LauncherChoice>) -> String {
    match choice {
        Some(LauncherChoice::Agent(id)) if !id.is_empty() => format!("agent:{id}"),
        Some(LauncherChoice::Shell(_)) => "shell".to_string(),
        Some(LauncherChoice::Browser) => "browser".to_string(),
        _ => "empty".to_string(),
    }
}

#[must_use]
pub const fn emptied_workspace_choice() -> Option<LauncherChoice> {
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteDropCheck {
    pub is_remote: bool,
    pub exit_code: Option<i32>,
    pub closing: bool,
    pub sleeping: bool,
    pub restarting: bool,
}

#[must_use]
pub fn dropped_remote_pane(check: &RemoteDropCheck) -> bool {
    check.is_remote
        && check.exit_code == Some(SSH_DROPPED_EXIT_CODE)
        && !check.closing
        && !check.sleeping
        && !check.restarting
}

pub fn allow_drop_retry(attempts: &mut Vec<u64>, now_ms: u64) -> bool {
    attempts.retain(|&t| now_ms.saturating_sub(t) < DROP_RETRY_WINDOW_MS);
    if attempts.len() >= DROP_RETRIES {
        return false;
    }
    attempts.push(now_ms);
    true
}

// --- Session History Filtering (`sessionHistory.js:109-167`) ---

#[must_use]
pub fn folder_key(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

#[must_use]
pub fn in_folder(cwd: &str, root: &str) -> bool {
    let c = folder_key(cwd);
    let r = folder_key(root);
    if c.is_empty() || r.is_empty() {
        return false;
    }
    c == r || c.starts_with(&format!("{r}/"))
}

#[must_use]
pub fn folder_label(path: &str, lang: LauncherLang) -> String {
    if path.is_empty() {
        return match lang {
            LauncherLang::En => "Unknown location".to_string(),
            LauncherLang::Fr => "Emplacement inconnu".to_string(),
        };
    }
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    if parts.len() >= 2 {
        format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
    } else {
        parts.first().copied().unwrap_or(path).to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionScope {
    #[default]
    All,
    Workspace,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionSort {
    #[default]
    Updated,
    Created,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: String,
    pub agent: String,
    pub title: String,
    pub cwd: String,
    pub started_ms: u64,
    pub updated_ms: u64,
}

#[derive(Debug, Clone)]
pub struct SessionFilterOpts<'a> {
    pub query: &'a str,
    pub agents: Option<&'a [&'a str]>,
    pub scope: SessionScope,
    pub sort: SessionSort,
    pub cwd: Option<&'a str>,
}

impl Default for SessionFilterOpts<'_> {
    fn default() -> Self {
        Self {
            query: "",
            agents: None,
            scope: SessionScope::All,
            sort: SessionSort::Updated,
            cwd: None,
        }
    }
}

#[must_use]
pub fn filter_sessions(
    sessions: &[SessionRecord],
    opts: &SessionFilterOpts<'_>,
) -> Vec<SessionRecord> {
    let terms: Vec<String> = opts
        .query
        .to_lowercase()
        .split_whitespace()
        .map(ToString::to_string)
        .collect();
    let allowed_slice = opts.agents.unwrap_or(&ALL_HISTORY_AGENTS);
    let wanted: HashSet<&str> = allowed_slice.iter().copied().collect();

    let mut out: Vec<SessionRecord> = sessions
        .iter()
        .filter(|s| {
            if !wanted.contains(s.agent.as_str()) {
                return false;
            }
            match opts.scope {
                SessionScope::Workspace => {
                    let Some(cwd) = opts.cwd.filter(|c| !c.is_empty()) else {
                        return false;
                    };
                    if folder_key(&s.cwd) != folder_key(cwd) {
                        return false;
                    }
                }
                SessionScope::Project => {
                    let Some(cwd) = opts.cwd.filter(|c| !c.is_empty()) else {
                        return false;
                    };
                    if !in_folder(&s.cwd, cwd) {
                        return false;
                    }
                }
                SessionScope::All => {}
            }
            if !terms.is_empty() {
                let text = format!(
                    "{} {} {} {} {}",
                    s.title,
                    s.id,
                    s.agent,
                    agent_display_name(&s.agent),
                    s.cwd
                )
                .to_lowercase();
                if terms.iter().any(|w| !text.contains(w)) {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect();

    let sort_time = |s: &SessionRecord| -> u64 {
        match opts.sort {
            SessionSort::Created => {
                if s.started_ms > 0 {
                    s.started_ms
                } else {
                    s.updated_ms
                }
            }
            SessionSort::Updated => s.updated_ms,
        }
    };
    out.sort_by_key(|a| std::cmp::Reverse(sort_time(a)));
    out
}

#[must_use]
pub fn recent_project_sessions(
    sessions: &[SessionRecord],
    cwd: &str,
    limit: usize,
) -> Vec<SessionRecord> {
    if cwd.is_empty() || limit == 0 {
        return Vec::new();
    }
    let filtered = filter_sessions(
        sessions,
        &SessionFilterOpts {
            query: "",
            agents: None,
            scope: SessionScope::Project,
            sort: SessionSort::Updated,
            cwd: Some(cwd),
        },
    );
    filtered.into_iter().take(limit).collect()
}

// --- `ProjectLauncherState` (`ProjectLauncher.vue`) ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherItem {
    Agent { id: String, name: String },
    InstallAgent { id: String, name: String },
    Terminal { shell_id: Option<String> },
    Browser,
    Session(SessionRecord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectLauncherAction {
    MoveSelection(usize),
    Launch(LauncherItem),
    StayEmpty,
    Noop,
}

#[derive(Debug, Clone)]
pub struct ProjectLauncherState {
    pub items: Vec<LauncherItem>,
    pub selected: usize,
    pub remember_for_new_projects: bool,
}

impl ProjectLauncherState {
    #[must_use]
    pub fn build(
        agents: &[LauncherAgentView],
        selected_shell: Option<String>,
        recent_sessions: &[SessionRecord],
    ) -> Self {
        let mut items = Vec::with_capacity(agents.len() + 2 + recent_sessions.len());
        for a in agents {
            if a.status == RemoteAgentStatus::Missing {
                items.push(LauncherItem::InstallAgent {
                    id: a.id.clone(),
                    name: a.name.clone(),
                });
            } else {
                items.push(LauncherItem::Agent {
                    id: a.id.clone(),
                    name: a.name.clone(),
                });
            }
        }
        items.push(LauncherItem::Terminal {
            shell_id: selected_shell,
        });
        items.push(LauncherItem::Browser);
        for s in recent_sessions {
            items.push(LauncherItem::Session(s.clone()));
        }
        Self {
            items,
            selected: 0,
            remember_for_new_projects: false,
        }
    }

    /// Shortcut badge `1..=9` shown on the first 9 launcher items (`ProjectLauncher.vue`).
    #[must_use]
    pub const fn shortcut_number(index: usize) -> Option<u8> {
        if index < 9 {
            Some((index as u8) + 1)
        } else {
            None
        }
    }

    pub fn handle_key(
        &mut self,
        key: &str,
        ctrl: bool,
        alt: bool,
        meta: bool,
    ) -> ProjectLauncherAction {
        if self.items.is_empty() {
            return if key == "Escape" {
                ProjectLauncherAction::StayEmpty
            } else {
                ProjectLauncherAction::Noop
            };
        }
        if !ctrl && !alt && !meta && key.len() == 1 {
            let b = key.as_bytes()[0];
            if (b'1'..=b'9').contains(&b) {
                let idx = usize::from(b - b'1');
                if let Some(item) = self.items.get(idx).cloned() {
                    self.selected = idx;
                    return ProjectLauncherAction::Launch(item);
                }
                return ProjectLauncherAction::Noop;
            }
        }
        match key {
            "ArrowDown" => {
                self.selected = (self.selected + 1) % self.items.len();
                ProjectLauncherAction::MoveSelection(self.selected)
            }
            "ArrowUp" => {
                self.selected = (self.selected + self.items.len() - 1) % self.items.len();
                ProjectLauncherAction::MoveSelection(self.selected)
            }
            "Enter" => self
                .items
                .get(self.selected)
                .cloned()
                .map_or(ProjectLauncherAction::Noop, ProjectLauncherAction::Launch),
            "Escape" => ProjectLauncherAction::StayEmpty,
            _ => ProjectLauncherAction::Noop,
        }
    }
}

// ---------------------------------------------------------------------------
// 4. `LaunchMenuState` (`C:\Tessel\src\renderer\src\components\LaunchMenu.vue`)
// ---------------------------------------------------------------------------

pub const LAUNCH_MENU_VIEWPORT_MARGIN: f32 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchPlacement {
    Auto,
    Left,
    Right,
    Down,
}

impl LaunchPlacement {
    #[must_use]
    pub const fn label(self, lang: LauncherLang) -> &'static str {
        match (self, lang) {
            (Self::Auto, LauncherLang::En) => "Auto",
            (Self::Auto, LauncherLang::Fr) => "Auto",
            (Self::Left, LauncherLang::En) => "Left",
            (Self::Left, LauncherLang::Fr) => "Gauche",
            (Self::Right, LauncherLang::En) => "Right",
            (Self::Right, LauncherLang::Fr) => "Droite",
            (Self::Down, LauncherLang::En) => "Below",
            (Self::Down, LauncherLang::Fr) => "Dessous",
        }
    }
}

#[must_use]
pub fn available_placements(has_panes: bool, auto_tile: bool) -> Vec<LaunchPlacement> {
    if !has_panes {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(4);
    if auto_tile {
        out.push(LaunchPlacement::Auto);
    }
    out.extend([
        LaunchPlacement::Left,
        LaunchPlacement::Right,
        LaunchPlacement::Down,
    ]);
    out
}

#[must_use]
pub fn where_text(
    placement: LaunchPlacement,
    has_panes: bool,
    pane_grid_is_grid: bool,
    target_name: Option<&str>,
    lang: LauncherLang,
) -> String {
    if !has_panes {
        return match lang {
            LauncherLang::En => "Opens in this workspace".to_string(),
            LauncherLang::Fr => "S'ouvre dans cet espace de travail".to_string(),
        };
    }
    match placement {
        LaunchPlacement::Auto => {
            if pane_grid_is_grid {
                match lang {
                    LauncherLang::En => "Joins the grid: panes re-tile".to_string(),
                    LauncherLang::Fr => {
                        "Rejoint la grille : les panneaux sont réorganisés".to_string()
                    }
                }
            } else {
                match lang {
                    LauncherLang::En => "Splits the largest pane (manual layout)".to_string(),
                    LauncherLang::Fr => {
                        "Divise le plus grand panneau (disposition manuelle)".to_string()
                    }
                }
            }
        }
        LaunchPlacement::Left | LaunchPlacement::Right | LaunchPlacement::Down => {
            let name = target_name
                .filter(|s| !s.is_empty())
                .unwrap_or("active pane");
            match (placement, lang) {
                (LaunchPlacement::Left, LauncherLang::En) => format!("Opens left of {name}"),
                (LaunchPlacement::Left, LauncherLang::Fr) => format!("S'ouvre à gauche de {name}"),
                (LaunchPlacement::Down, LauncherLang::En) => format!("Opens below {name}"),
                (LaunchPlacement::Down, LauncherLang::Fr) => format!("S'ouvre sous {name}"),
                (LaunchPlacement::Right, LauncherLang::En) => format!("Opens right of {name}"),
                (LaunchPlacement::Right, LauncherLang::Fr) => format!("S'ouvre à droite de {name}"),
                (LaunchPlacement::Auto, _) => unreachable!(),
            }
        }
    }
}

#[must_use]
pub fn clamp_launch_menu_pos(
    anchor_x: f32,
    anchor_y: f32,
    menu_w: f32,
    menu_h: f32,
    win_w: f32,
    win_h: f32,
) -> (f32, f32) {
    let max_x = (win_w - menu_w - LAUNCH_MENU_VIEWPORT_MARGIN).max(LAUNCH_MENU_VIEWPORT_MARGIN);
    let max_y = (win_h - menu_h - LAUNCH_MENU_VIEWPORT_MARGIN).max(LAUNCH_MENU_VIEWPORT_MARGIN);
    (
        anchor_x.clamp(LAUNCH_MENU_VIEWPORT_MARGIN, max_x),
        anchor_y.clamp(LAUNCH_MENU_VIEWPORT_MARGIN, max_y),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMenuItem {
    Shell { id: String, name: String },
    Agent { id: String, name: String },
    InstallAgent { id: String, name: String },
    ChatAgent { id: String, name: String },
    Browser,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMenuAction {
    MoveSelection(usize),
    Launch {
        item: LaunchMenuItem,
        placement: LaunchPlacement,
        in_worktree: bool,
    },
    OpenAgentSettings,
    Close,
    Noop,
}

#[derive(Debug, Clone)]
pub struct LaunchMenuState {
    pub items: Vec<LaunchMenuItem>,
    pub selected: usize,
    pub placement: LaunchPlacement,
    pub in_worktree: bool,
    pub host_menu: bool,
}

impl LaunchMenuState {
    #[must_use]
    pub fn new(
        items: Vec<LaunchMenuItem>,
        has_panes: bool,
        auto_tile: bool,
        host_menu: bool,
    ) -> Self {
        let placement = if has_panes && auto_tile {
            LaunchPlacement::Auto
        } else {
            LaunchPlacement::Right
        };
        Self {
            items,
            selected: 0,
            placement,
            in_worktree: false,
            host_menu,
        }
    }

    pub fn handle_key(&mut self, key: &str) -> LaunchMenuAction {
        match key {
            "ArrowDown" => {
                if !self.items.is_empty() {
                    self.selected = (self.selected + 1) % self.items.len();
                }
                LaunchMenuAction::MoveSelection(self.selected)
            }
            "ArrowUp" => {
                if !self.items.is_empty() {
                    self.selected = (self.selected + self.items.len() - 1) % self.items.len();
                }
                LaunchMenuAction::MoveSelection(self.selected)
            }
            "Enter" => {
                self.items
                    .get(self.selected)
                    .cloned()
                    .map_or(LaunchMenuAction::Noop, |item| LaunchMenuAction::Launch {
                        item,
                        placement: self.placement,
                        in_worktree: self.in_worktree,
                    })
            }
            "Escape" => LaunchMenuAction::Close,
            _ => LaunchMenuAction::Noop,
        }
    }
}
