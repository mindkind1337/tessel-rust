//! Pure data models and exact behavioral parity logic for:
//! - Fuzzy file scoring & filtering (`C:\Tessel\src\shared\fuzzy.js`)
//! - Bounded pure-Rust project file walk & `FileFinder` (`C:\Tessel\src\main\fileOpen.js`, `FileFinder.vue`)
//! - Empty-grid `ProjectLauncher`, remote drop retry & recent sessions (`projectLauncher.js`, `ProjectLauncher.vue`, `sessionHistory.js`)
//! - Toolbar/pane `LaunchMenu` placement, positioning & keyboard navigation (`LaunchMenu.vue`)

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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

#[must_use]
pub fn fuzzy_score(query: &str, path: &str) -> f64 {
    let q_chars: Vec<char> = query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if q_chars.is_empty() {
        return 0.0;
    }

    let p_chars: Vec<char> = path.chars().collect();
    let lower_chars: Vec<char> = p_chars
        .iter()
        .map(|&c| c.to_lowercase().next().unwrap_or(c))
        .collect();

    let name_start = lower_chars
        .iter()
        .rposition(|&c| c == '/' || c == '\\')
        .map_or(0, |idx| idx + 1);

    let mut qi = 0usize;
    let mut score = 0.0f64;
    let mut run = 0usize;
    let mut last: Option<usize> = None;

    for i in 0..lower_chars.len() {
        if qi >= q_chars.len() {
            break;
        }
        if lower_chars[i] != q_chars[qi] {
            continue;
        }
        run = if qi > 0 && last == Some(i - 1) {
            run + 1
        } else {
            0
        };
        score += (1 + run * 3) as f64;
        if i >= name_start {
            score += 2.0;
        }
        let prev = i.checked_sub(1).map(|idx| lower_chars[idx]);
        let camel_boundary = i
            .checked_sub(1)
            .is_some_and(|idx| p_chars[i] != lower_chars[i] && p_chars[idx] == lower_chars[idx]);
        let boundary = i == 0
            || prev
                .is_some_and(|ch| matches!(ch, '/' | '\\' | '.' | '_' | '-') || ch.is_whitespace())
            || camel_boundary;
        if boundary {
            score += 3.0;
        }
        last = Some(i);
        qi += 1;
    }

    if qi < q_chars.len() {
        return -1.0;
    }

    let q_str: String = q_chars.iter().collect();
    let name: String = lower_chars[name_start..].iter().collect();
    if name.contains(&q_str) {
        score += 20.0;
    }
    let stem = match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name.as_str(),
    };
    if stem == q_str {
        score += 10.0;
    }

    score - (lower_chars.len() as f64) * 0.01
}

#[must_use]
pub fn fuzzy_filter(query: &str, paths: &[String], limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let has_non_ws = query.chars().any(|c| !c.is_whitespace());
    if !has_non_ws {
        return paths.iter().take(limit).cloned().collect();
    }

    let mut hits: Vec<(usize, f64, &String)> = paths
        .iter()
        .enumerate()
        .filter_map(|(idx, p)| {
            let s = fuzzy_score(query, p);
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFileList {
    pub files: Vec<String>,
    pub truncated: bool,
}

/// Pure-Rust bounded directory walk matching `fileOpen.js` (`SKIP_DIRS`, symlink skip,
/// forward-slash relative paths, lexicographic sort). Never spawns an external process.
pub fn walk_project_files(root: &Path, max_files: usize) -> Result<ProjectFileList, String> {
    let meta = std::fs::metadata(root).map_err(|_| "The project folder is missing.".to_string())?;
    if !meta.is_dir() {
        return Err("The project folder is missing.".to_string());
    }

    let mut files: Vec<String> = Vec::new();
    let mut truncated = false;
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];

    'walk: while let Some(dir) = stack.pop() {
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<std::fs::DirEntry> = read_dir.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);

        for entry in entries {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            let full = entry.path();
            if ft.is_dir() {
                let name = entry.file_name();
                let skip = name.to_str().is_some_and(|n| SKIP_DIRS.contains(&n));
                if !skip {
                    stack.push(full);
                }
            } else if ft.is_file() {
                if files.len() >= max_files {
                    truncated = true;
                    break 'walk;
                }
                if let Ok(rel) = full.strip_prefix(root) {
                    let normalized = rel.to_string_lossy().replace('\\', "/");
                    if !normalized.is_empty() {
                        files.push(normalized);
                    }
                }
            }
        }
    }

    files.sort();
    Ok(ProjectFileList { files, truncated })
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
    pub files: Vec<String>,
    pub truncated: bool,
    pub query: String,
    pub selected: usize,
    pub can_insert: bool,
    pub limit: usize,
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
        }
    }
}

impl FileFinderState {
    #[must_use]
    pub fn new(files: Vec<String>, truncated: bool, can_insert: bool) -> Self {
        Self {
            files,
            truncated,
            query: String::new(),
            selected: 0,
            can_insert,
            limit: FILE_FINDER_LIMIT,
        }
    }

    #[must_use]
    pub fn results(&self) -> Vec<String> {
        fuzzy_filter(&self.query, &self.files, self.limit)
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.selected = 0;
    }

    pub fn handle_key(&mut self, key: &str, ctrl: bool) -> FileFinderAction {
        let res = self.results();
        match key {
            "ArrowDown" => {
                if !res.is_empty() {
                    self.selected = (self.selected + 1) % res.len();
                }
                FileFinderAction::MoveSelection(self.selected)
            }
            "ArrowUp" => {
                if !res.is_empty() {
                    self.selected = (self.selected + res.len() - 1) % res.len();
                }
                FileFinderAction::MoveSelection(self.selected)
            }
            "Enter" => {
                let Some(picked) = res.get(self.selected).cloned() else {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMenuAction {
    MoveSelection(usize),
    Launch {
        item: LaunchMenuItem,
        placement: LaunchPlacement,
        in_worktree: bool,
    },
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
