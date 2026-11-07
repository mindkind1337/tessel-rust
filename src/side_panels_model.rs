//! UI-agnostic models, scanners, persistence, and bounded Git helpers for:
//! - Agent Dashboard (`AgentDashboard.vue` + `agentDashboard.js`)
//! - Task History & formula-safe CSV export (`TaskHistoryPanel.vue` + `historyCsv.js`)
//! - Session History / Past Conversations (`SessionHistoryPanel.vue` + `sessionHistory.js`)
//! - Conversations & Activity threads (`ConversationsPanel.vue` + `conversations.js`)
//! - Project Notes `.tessel/notes.md` with `mtime` conflict guard (`NotesPanel.vue`)
//! - SCM Commit History & 5-color swimlane graph (`ScmHistoryPanel.vue` + `gitHistoryGraph.js`)
//! - Task Branch Review (`ReviewPanel.vue` + `main/review.js`)

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::git;

pub const MAX_TASK_HISTORY: usize = 2_000;
pub const MAX_ACTIVITY_ENTRIES: usize = 1_000;
pub const MAX_NOTES_BYTES: usize = 512 * 1024;
pub const MAX_TRANSCRIPT_SCAN_BYTES: usize = 256 * 1024;
pub const DEFAULT_COMMIT_LIMIT: usize = 80;
pub const SESSION_LIMITS: [usize; 3] = [50, 100, 200];

#[must_use]
pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn file_mtime_ms(path: &Path) -> Option<u64> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    Some(
        modified
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    )
}

// ============================================================================
// 1. Agent Dashboard (`agentDashboard.js` parity)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DashboardState {
    Waiting,
    Working,
    Done,
    Idle,
    Sleeping,
}

impl DashboardState {
    pub const ALL: [Self; 5] = [
        Self::Waiting,
        Self::Working,
        Self::Done,
        Self::Idle,
        Self::Sleeping,
    ];

    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Waiting, false) => "Waiting",
            (Self::Waiting, true) => "En attente",
            (Self::Working, false) => "Working",
            (Self::Working, true) => "En cours",
            (Self::Done, false) => "Done",
            (Self::Done, true) => "Terminé",
            (Self::Idle, false) => "Idle",
            (Self::Idle, true) => "Inactif",
            (Self::Sleeping, false) => "Sleeping",
            (Self::Sleeping, true) => "En veille",
        }
    }

    #[must_use]
    pub const fn rgb(self) -> (u8, u8, u8) {
        match self {
            Self::Waiting => (0xf5, 0xa5, 0x24),
            Self::Working => (0x6c, 0x9c, 0xff),
            Self::Done => (0x3d, 0xdc, 0x84),
            Self::Idle => (0x8b, 0x93, 0xa1),
            Self::Sleeping => (0x63, 0x6b, 0x78),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DashboardGroupBy {
    #[default]
    Status,
    Project,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DashboardItem {
    pub pane_id: u64,
    pub project_id: Option<u64>,
    pub project_name: String,
    pub title: String,
    pub agent_kind: String,
    pub branch: Option<String>,
    pub task_line: Option<String>,
    pub state: DashboardState,
    pub is_chat: bool,
    pub is_focused: bool,
}

impl DashboardItem {
    #[must_use]
    pub const fn is_sleepable(&self) -> bool {
        matches!(self.state, DashboardState::Idle | DashboardState::Done)
            && !self.is_chat
            && !self.is_focused
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DashboardGroup {
    pub heading: String,
    pub state_hint: Option<DashboardState>,
    pub items: Vec<DashboardItem>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DashboardViewModel {
    pub counts: BTreeMap<DashboardState, usize>,
    pub total: usize,
    pub sleepable_pane_ids: Vec<u64>,
    pub groups: Vec<DashboardGroup>,
}

#[must_use]
pub fn build_dashboard(
    items: &[DashboardItem],
    group_by: DashboardGroupBy,
    filter_state: Option<DashboardState>,
    query: &str,
    french: bool,
) -> DashboardViewModel {
    let mut counts = BTreeMap::new();
    for st in DashboardState::ALL {
        counts.insert(st, 0);
    }
    let mut sleepable_pane_ids = Vec::new();
    for item in items {
        *counts.entry(item.state).or_insert(0) += 1;
        if item.is_sleepable() {
            sleepable_pane_ids.push(item.pane_id);
        }
    }

    let q = query.trim().to_ascii_lowercase();
    let filtered: Vec<DashboardItem> = items
        .iter()
        .filter(|it| filter_state.is_none_or(|st| it.state == st))
        .filter(|it| {
            if q.is_empty() {
                return true;
            }
            it.title.to_ascii_lowercase().contains(&q)
                || it.project_name.to_ascii_lowercase().contains(&q)
                || it.agent_kind.to_ascii_lowercase().contains(&q)
                || it
                    .branch
                    .as_deref()
                    .is_some_and(|b| b.to_ascii_lowercase().contains(&q))
                || it
                    .task_line
                    .as_deref()
                    .is_some_and(|t| t.to_ascii_lowercase().contains(&q))
        })
        .cloned()
        .collect();

    let mut groups = Vec::new();
    match group_by {
        DashboardGroupBy::Status => {
            for st in DashboardState::ALL {
                let bucket: Vec<DashboardItem> =
                    filtered.iter().filter(|i| i.state == st).cloned().collect();
                if !bucket.is_empty() {
                    groups.push(DashboardGroup {
                        heading: st.label(french).to_owned(),
                        state_hint: Some(st),
                        items: bucket,
                    });
                }
            }
        }
        DashboardGroupBy::Project => {
            let mut by_proj: BTreeMap<String, Vec<DashboardItem>> = BTreeMap::new();
            for item in filtered {
                let key = if item.project_name.trim().is_empty() {
                    if french {
                        "Sans projet".to_owned()
                    } else {
                        "No project".to_owned()
                    }
                } else {
                    item.project_name.clone()
                };
                by_proj.entry(key).or_default().push(item);
            }
            for (heading, mut bucket) in by_proj {
                bucket.sort_by_key(|it| (it.state, it.pane_id));
                groups.push(DashboardGroup {
                    heading,
                    state_hint: None,
                    items: bucket,
                });
            }
        }
    }

    DashboardViewModel {
        counts,
        total: items.len(),
        sleepable_pane_ids,
        groups,
    }
}

// ============================================================================
// 2. Task History & Formula-Safe CSV (`TaskHistoryPanel.vue` + `historyCsv.js`)
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TaskHistoryRecord {
    pub id: u64,
    pub task_id: u64,
    pub title: String,
    pub project_id: u64,
    pub project_name: String,
    pub agent_name: String,
    pub agent_kind: String,
    pub created_at_ms: u64,
    pub done_at_ms: u64,
    pub duration_ms: u64,
    pub tokens: u64,
    pub cost_usd: f64,
}

impl Default for TaskHistoryRecord {
    fn default() -> Self {
        Self {
            id: 0,
            task_id: 0,
            title: String::new(),
            project_id: 0,
            project_name: String::new(),
            agent_name: String::new(),
            agent_kind: String::new(),
            created_at_ms: 0,
            done_at_ms: 0,
            duration_ms: 0,
            tokens: 0,
            cost_usd: 0.0,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TaskHistoryStore {
    pub next_id: u64,
    pub records: Vec<TaskHistoryRecord>,
}

impl TaskHistoryStore {
    #[must_use]
    pub fn file_path(data_dir: &Path) -> PathBuf {
        data_dir.join("task-history.json")
    }

    #[must_use]
    pub fn load(data_dir: &Path) -> Self {
        let path = Self::file_path(data_dir);
        let Ok(raw) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str::<Self>(&raw).unwrap_or_default()
    }

    pub fn save(&self, data_dir: &Path) -> Result<(), String> {
        fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let path = Self::file_path(data_dir);
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(path, json).map_err(|e| e.to_string())
    }

    pub fn record_completed(&mut self, mut rec: TaskHistoryRecord) {
        self.next_id = self.next_id.saturating_add(1).max(1);
        rec.id = self.next_id;
        if rec.done_at_ms == 0 {
            rec.done_at_ms = now_unix_ms();
        }
        if rec.created_at_ms > 0 && rec.done_at_ms >= rec.created_at_ms && rec.duration_ms == 0 {
            rec.duration_ms = rec.done_at_ms - rec.created_at_ms;
        }
        self.records.push(rec);
        if self.records.len() > MAX_TASK_HISTORY {
            let drop_n = self.records.len() - MAX_TASK_HISTORY;
            self.records.drain(0..drop_n);
        }
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaskHistoryFilter {
    Today,
    Days7,
    Days30,
    #[default]
    All,
}

impl TaskHistoryFilter {
    pub const ALL: [Self; 4] = [Self::Today, Self::Days7, Self::Days30, Self::All];

    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Today, false) => "Today",
            (Self::Today, true) => "Aujourd'hui",
            (Self::Days7, false) => "7d",
            (Self::Days7, true) => "7 j",
            (Self::Days30, false) => "30d",
            (Self::Days30, true) => "30 j",
            (Self::All, false) => "All",
            (Self::All, true) => "Tout",
        }
    }

    #[must_use]
    pub const fn window_ms(self) -> Option<u64> {
        match self {
            Self::Today => Some(24 * 3_600 * 1_000),
            Self::Days7 => Some(7 * 24 * 3_600 * 1_000),
            Self::Days30 => Some(30 * 24 * 3_600 * 1_000),
            Self::All => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaskHistorySort {
    #[default]
    Date,
    Cost,
    Time,
}

impl TaskHistorySort {
    pub const ALL: [Self; 3] = [Self::Date, Self::Cost, Self::Time];

    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Date, false) => "Date",
            (Self::Date, true) => "Date",
            (Self::Cost, false) => "Cost",
            (Self::Cost, true) => "Coût",
            (Self::Time, false) => "Duration",
            (Self::Time, true) => "Durée",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TaskHistoryTotals {
    pub count: usize,
    pub duration_ms: u64,
    pub tokens: u64,
    pub cost_usd: f64,
}

#[must_use]
pub fn filter_and_sort_task_history(
    records: &[TaskHistoryRecord],
    project_filter: Option<u64>,
    window: TaskHistoryFilter,
    sort: TaskHistorySort,
    query: &str,
    now_ms: u64,
) -> (Vec<TaskHistoryRecord>, TaskHistoryTotals) {
    let q = query.trim().to_ascii_lowercase();
    let cutoff = window.window_ms().map(|w| now_ms.saturating_sub(w));
    let mut out: Vec<TaskHistoryRecord> = records
        .iter()
        .filter(|r| project_filter.is_none_or(|pid| r.project_id == pid))
        .filter(|r| cutoff.is_none_or(|min_ms| r.done_at_ms >= min_ms))
        .filter(|r| {
            if q.is_empty() {
                return true;
            }
            r.title.to_ascii_lowercase().contains(&q)
                || r.project_name.to_ascii_lowercase().contains(&q)
                || r.agent_name.to_ascii_lowercase().contains(&q)
                || r.agent_kind.to_ascii_lowercase().contains(&q)
        })
        .cloned()
        .collect();

    match sort {
        TaskHistorySort::Date => out.sort_by_key(|a| std::cmp::Reverse(a.done_at_ms)),
        TaskHistorySort::Cost => out.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.done_at_ms.cmp(&a.done_at_ms))
        }),
        TaskHistorySort::Time => out.sort_by(|a, b| {
            b.duration_ms
                .cmp(&a.duration_ms)
                .then_with(|| b.done_at_ms.cmp(&a.done_at_ms))
        }),
    }

    let totals = TaskHistoryTotals {
        count: out.len(),
        duration_ms: out.iter().map(|r| r.duration_ms).sum(),
        tokens: out.iter().map(|r| r.tokens).sum(),
        cost_usd: out.iter().map(|r| r.cost_usd).sum(),
    };
    (out, totals)
}

fn csv_escape_cell(raw: &str) -> String {
    let trimmed = raw.trim_start_matches(' ');
    let needs_formula_guard = trimmed
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@' | '\t' | '\r'));
    let guarded = if needs_formula_guard {
        format!("'{raw}")
    } else {
        raw.to_owned()
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

/// Formula-safe UTF-8 BOM CSV export (`historyCsv.js` parity).
#[must_use]
pub fn history_csv(records: &[TaskHistoryRecord]) -> String {
    let mut out =
        String::from("\u{feff}Title,Project,Agent,Provider,DoneAtMs,DurationMs,Tokens,CostUSD\r\n");
    for rec in records {
        let row = [
            csv_escape_cell(&rec.title),
            csv_escape_cell(&rec.project_name),
            csv_escape_cell(&rec.agent_name),
            csv_escape_cell(&rec.agent_kind),
            rec.done_at_ms.to_string(),
            rec.duration_ms.to_string(),
            rec.tokens.to_string(),
            format!("{:.4}", rec.cost_usd),
        ]
        .join(",");
        out.push_str(&row);
        out.push_str("\r\n");
    }
    out
}

#[must_use]
pub fn format_duration_compact(ms: u64) -> String {
    let secs = ms / 1_000;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {:02}m", secs / 3_600, (secs % 3_600) / 60)
    }
}

// ============================================================================
// 3. Past Conversations / Session History (`sessionHistory.js` parity)
// ============================================================================

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionHistoryScope {
    #[default]
    Project,
    Workspace,
    All,
}

impl SessionHistoryScope {
    pub const ALL: [Self; 3] = [Self::Project, Self::Workspace, Self::All];

    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Project, false) => "Active project",
            (Self::Project, true) => "Projet actif",
            (Self::Workspace, false) => "Workspace",
            (Self::Workspace, true) => "Espace",
            (Self::All, false) => "All folders",
            (Self::All, true) => "Tous les dossiers",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionHistoryGroupBy {
    #[default]
    Folder,
    Agent,
}

impl SessionHistoryGroupBy {
    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Folder, false) => "By folder",
            (Self::Folder, true) => "Par dossier",
            (Self::Agent, false) => "By agent",
            (Self::Agent, true) => "Par agent",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionHistoryEntry {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub folder: PathBuf,
    pub updated_at_ms: u64,
    pub file_path: Option<PathBuf>,
    pub open_pane_id: Option<u64>,
    pub transcript_snippet: Option<String>,
}

#[must_use]
pub fn mask_transcript_secrets(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for token in input.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        let lower = token.to_ascii_lowercase();
        let looks_secret = (lower.starts_with("sk-") && token.len() >= 16)
            || (lower.starts_with("ghp_") && token.len() >= 16)
            || (lower.starts_with("github_pat_") && token.len() >= 20)
            || (lower.starts_with("xoxb-") && token.len() >= 16)
            || (lower.starts_with("aiZa") && token.len() >= 20);
        if looks_secret {
            let prefix: String = token.chars().take(4).collect();
            out.push_str(&prefix);
            out.push_str("••••••••");
        } else {
            out.push_str(token);
        }
    }
    out
}

fn read_bounded_utf8(path: &Path, max_bytes: usize) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(max_bytes as u64).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn summarize_session_file(path: &Path) -> (String, Option<PathBuf>) {
    let fallback_title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Session")
        .to_owned();
    let Some(head) = read_bounded_utf8(path, 16 * 1024) else {
        return (fallback_title, None);
    };
    let mut title = None;
    let mut cwd = None;
    for line in head.lines().take(25) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if cwd.is_none()
                && let Some(dir) = val
                    .get("cwd")
                    .or_else(|| val.get("projectPath"))
                    .and_then(|v| v.as_str())
                && !dir.is_empty()
            {
                cwd = Some(PathBuf::from(dir));
            }
            if title.is_none() {
                let candidate = val
                    .get("summary")
                    .or_else(|| val.get("title"))
                    .and_then(|v| v.as_str())
                    .or_else(|| {
                        val.get("message")
                            .and_then(|m| m.get("content"))
                            .and_then(|c| c.as_str())
                    });
                if let Some(text) = candidate {
                    let first_line = text.lines().next().unwrap_or("").trim();
                    if !first_line.is_empty() {
                        title = Some(first_line.chars().take(100).collect::<String>());
                    }
                }
            }
        }
        if title.is_some() && cwd.is_some() {
            break;
        }
    }
    (title.unwrap_or(fallback_title), cwd)
}

fn collect_session_files(
    dir: &Path,
    agent_id: &str,
    depth: usize,
    max_depth: usize,
    limit: usize,
    out: &mut Vec<SessionHistoryEntry>,
) {
    if depth > max_depth || out.len() >= limit || !dir.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= limit {
            break;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            collect_session_files(&path, agent_id, depth + 1, max_depth, limit, out);
        } else if ft.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("jsonl") || ext.eq_ignore_ascii_case("json")
                })
        {
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("session")
                .to_owned();
            let updated_at_ms = file_mtime_ms(&path).unwrap_or(0);
            let (title, detected_cwd) = summarize_session_file(&path);
            let folder = detected_cwd.unwrap_or_else(|| dir.to_path_buf());
            out.push(SessionHistoryEntry {
                id,
                agent_id: agent_id.to_owned(),
                title: mask_transcript_secrets(&title),
                folder,
                updated_at_ms,
                file_path: Some(path),
                open_pane_id: None,
                transcript_snippet: None,
            });
        }
    }
}

#[must_use]
pub fn scan_past_sessions(
    home_dir: Option<&Path>,
    live_entries: &[SessionHistoryEntry],
    limit: usize,
) -> Vec<SessionHistoryEntry> {
    let cap = limit.clamp(1, 500);
    let mut out: Vec<SessionHistoryEntry> = live_entries.to_vec();
    let mut seen_ids: BTreeSet<(String, String)> = out
        .iter()
        .map(|e| (e.agent_id.clone(), e.id.clone()))
        .collect();

    if let Some(home) = home_dir {
        let mut disk = Vec::new();
        collect_session_files(
            &home.join(".claude").join("projects"),
            "claude",
            0,
            3,
            cap * 2,
            &mut disk,
        );
        collect_session_files(
            &home.join(".codex").join("sessions"),
            "codex",
            0,
            4,
            cap * 2,
            &mut disk,
        );
        for item in disk {
            if seen_ids.insert((item.agent_id.clone(), item.id.clone())) {
                out.push(item);
            }
        }
    }

    out.sort_by_key(|a| std::cmp::Reverse(a.updated_at_ms));
    out.truncate(cap);
    out
}

#[must_use]
pub fn filter_sessions(
    entries: &[SessionHistoryEntry],
    scope: SessionHistoryScope,
    active_project_root: Option<&Path>,
    workspace_roots: &[PathBuf],
    agent_filter: Option<&str>,
    query: &str,
    include_transcripts: bool,
) -> Vec<SessionHistoryEntry> {
    let q = query.trim().to_ascii_lowercase();
    let mut out = Vec::new();
    for entry in entries {
        if let Some(ag) = agent_filter
            && !ag.is_empty()
            && entry.agent_id != ag
        {
            continue;
        }
        match scope {
            SessionHistoryScope::Project => {
                if let Some(root) = active_project_root
                    && !entry.folder.starts_with(root)
                    && entry.open_pane_id.is_none()
                {
                    continue;
                }
            }
            SessionHistoryScope::Workspace => {
                if !workspace_roots.is_empty()
                    && !workspace_roots.iter().any(|r| entry.folder.starts_with(r))
                    && entry.open_pane_id.is_none()
                {
                    continue;
                }
            }
            SessionHistoryScope::All => {}
        }

        if q.is_empty() {
            out.push(entry.clone());
            continue;
        }
        let meta_hit = entry.title.to_ascii_lowercase().contains(&q)
            || entry.agent_id.to_ascii_lowercase().contains(&q)
            || entry.id.to_ascii_lowercase().contains(&q)
            || entry
                .folder
                .to_string_lossy()
                .to_ascii_lowercase()
                .contains(&q);
        if meta_hit {
            out.push(entry.clone());
            continue;
        }
        if include_transcripts
            && let Some(path) = &entry.file_path
            && let Some(body) = read_bounded_utf8(path, MAX_TRANSCRIPT_SCAN_BYTES)
            && let Some(line) = body.lines().find(|l| l.to_ascii_lowercase().contains(&q))
        {
            let mut cloned = entry.clone();
            let snippet: String = mask_transcript_secrets(line.trim())
                .chars()
                .take(140)
                .collect();
            cloned.transcript_snippet = Some(snippet);
            out.push(cloned);
        }
    }
    out
}

pub fn delete_session_file(entry: &SessionHistoryEntry) -> Result<(), String> {
    let Some(path) = &entry.file_path else {
        return Err("Session has no local transcript file.".into());
    };
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ============================================================================
// 4. Conversations & Activity Threads (`conversations.js` parity)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationMediaKind {
    Links,
    Images,
    Files,
    Code,
}

impl ConversationMediaKind {
    pub const ALL: [Self; 4] = [Self::Links, Self::Images, Self::Files, Self::Code];

    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Links, false) => "Links",
            (Self::Links, true) => "Liens",
            (Self::Images, false) => "Images",
            (Self::Images, true) => "Images",
            (Self::Files, false) => "Files",
            (Self::Files, true) => "Fichiers",
            (Self::Code, false) => "Code",
            (Self::Code, true) => "Code",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConversationWindow {
    Hours24,
    #[default]
    Days7,
    Days30,
}

impl ConversationWindow {
    pub const ALL: [Self; 3] = [Self::Hours24, Self::Days7, Self::Days30];

    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Hours24, false) => "24h",
            (Self::Hours24, true) => "24 h",
            (Self::Days7, false) => "7d",
            (Self::Days7, true) => "7 j",
            (Self::Days30, false) => "30d",
            (Self::Days30, true) => "30 j",
        }
    }

    #[must_use]
    pub const fn window_ms(self) -> u64 {
        match self {
            Self::Hours24 => 24 * 3_600 * 1_000,
            Self::Days7 => 7 * 24 * 3_600 * 1_000,
            Self::Days30 => 30 * 24 * 3_600 * 1_000,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ActivityMessage {
    pub id: u64,
    pub thread_key: String,
    pub thread_title: String,
    pub pane_id: Option<u64>,
    pub sender: String,
    pub recipient: String,
    pub body: String,
    pub timestamp_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ActivityStore {
    pub next_id: u64,
    pub messages: Vec<ActivityMessage>,
}

impl ActivityStore {
    #[must_use]
    pub fn file_path(data_dir: &Path) -> PathBuf {
        data_dir.join("activity.json")
    }

    #[must_use]
    pub fn load(data_dir: &Path) -> Self {
        let path = Self::file_path(data_dir);
        let Ok(raw) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str::<Self>(&raw).unwrap_or_default()
    }

    pub fn save(&self, data_dir: &Path) -> Result<(), String> {
        fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(Self::file_path(data_dir), json).map_err(|e| e.to_string())
    }

    pub fn push(&mut self, mut msg: ActivityMessage) {
        self.next_id = self.next_id.saturating_add(1).max(1);
        msg.id = self.next_id;
        if msg.timestamp_ms == 0 {
            msg.timestamp_ms = now_unix_ms();
        }
        self.messages.push(msg);
        if self.messages.len() > MAX_ACTIVITY_ENTRIES {
            let drop_n = self.messages.len() - MAX_ACTIVITY_ENTRIES;
            self.messages.drain(0..drop_n);
        }
    }
}

#[must_use]
pub fn detect_media_kinds(body: &str) -> BTreeSet<ConversationMediaKind> {
    let mut kinds = BTreeSet::new();
    let lower = body.to_ascii_lowercase();
    if lower.contains("http://") || lower.contains("https://") {
        kinds.insert(ConversationMediaKind::Links);
    }
    if [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg"]
        .iter()
        .any(|ext| lower.contains(ext))
    {
        kinds.insert(ConversationMediaKind::Images);
    }
    if body.contains("```") {
        kinds.insert(ConversationMediaKind::Code);
    }
    if [".rs", ".ts", ".js", ".vue", ".json", ".toml", ".md", ".py"]
        .iter()
        .any(|ext| lower.contains(ext))
    {
        kinds.insert(ConversationMediaKind::Files);
    }
    kinds
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationThread {
    pub key: String,
    pub title: String,
    pub is_team: bool,
    pub pane_id: Option<u64>,
    pub updated_at_ms: u64,
    pub media: BTreeSet<ConversationMediaKind>,
    pub messages: Vec<ActivityMessage>,
}

#[must_use]
pub fn build_conversation_threads(
    messages: &[ActivityMessage],
    window: ConversationWindow,
    media_filter: Option<ConversationMediaKind>,
    query: &str,
    now_ms: u64,
) -> Vec<ConversationThread> {
    let min_ms = now_ms.saturating_sub(window.window_ms());
    let q = query.trim().to_ascii_lowercase();
    let mut grouped: BTreeMap<String, ConversationThread> = BTreeMap::new();

    for msg in messages {
        if msg.timestamp_ms < min_ms {
            continue;
        }
        let msg_media = detect_media_kinds(&msg.body);
        if let Some(req_media) = media_filter
            && !msg_media.contains(&req_media)
        {
            continue;
        }
        if !q.is_empty()
            && !msg.body.to_ascii_lowercase().contains(&q)
            && !msg.sender.to_ascii_lowercase().contains(&q)
            && !msg.thread_title.to_ascii_lowercase().contains(&q)
        {
            continue;
        }
        let entry = grouped
            .entry(msg.thread_key.clone())
            .or_insert_with(|| ConversationThread {
                key: msg.thread_key.clone(),
                title: msg.thread_title.clone(),
                is_team: msg.thread_key.starts_with("team:"),
                pane_id: msg.pane_id,
                updated_at_ms: msg.timestamp_ms,
                media: BTreeSet::new(),
                messages: Vec::new(),
            });
        if msg.timestamp_ms >= entry.updated_at_ms {
            entry.updated_at_ms = msg.timestamp_ms;
            if !msg.thread_title.is_empty() {
                entry.title = msg.thread_title.clone();
            }
        }
        if entry.pane_id.is_none() {
            entry.pane_id = msg.pane_id;
        }
        entry.media.extend(msg_media);
        entry.messages.push(msg.clone());
    }

    let mut threads: Vec<ConversationThread> = grouped.into_values().collect();
    for t in &mut threads {
        t.messages.sort_by_key(|m| m.timestamp_ms);
    }
    threads.sort_by_key(|a| std::cmp::Reverse(a.updated_at_ms));
    threads
}

#[must_use]
pub fn format_thread_as_text(thread: &ConversationThread) -> String {
    let mut out = format!("# {}\n\n", thread.title);
    for msg in &thread.messages {
        out.push_str(&format!("[{}] {}\n\n", msg.sender, msg.body.trim()));
    }
    out
}

// ============================================================================
// 5. Project Notes (`.tessel/notes.md` + `mtime` conflict guard)
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotesConflict {
    pub disk_text: String,
    pub disk_mtime_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotesModel {
    pub project_root: Option<PathBuf>,
    pub text: String,
    pub saved_text: String,
    pub loaded_mtime_ms: Option<u64>,
    pub preview_mode: bool,
    pub conflict: Option<NotesConflict>,
}

impl NotesModel {
    #[must_use]
    pub fn notes_path(project_root: &Path) -> PathBuf {
        project_root.join(".tessel").join("notes.md")
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.text != self.saved_text
    }

    pub fn load_project(&mut self, project_root: Option<&Path>) -> Result<(), String> {
        self.conflict = None;
        let Some(root) = project_root else {
            self.project_root = None;
            self.text.clear();
            self.saved_text.clear();
            self.loaded_mtime_ms = None;
            return Ok(());
        };
        self.project_root = Some(root.to_path_buf());
        let path = Self::notes_path(root);
        if !path.exists() {
            self.text.clear();
            self.saved_text.clear();
            self.loaded_mtime_ms = None;
            return Ok(());
        }
        let meta = fs::metadata(&path).map_err(|e| e.to_string())?;
        if meta.len() > MAX_NOTES_BYTES as u64 {
            return Err("Project notes exceed 512 KiB limit.".into());
        }
        let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        self.loaded_mtime_ms = file_mtime_ms(&path);
        self.saved_text = content.clone();
        self.text = content;
        Ok(())
    }

    /// Saves `.tessel/notes.md`, detecting external edits if `mtime` changed while dirty.
    pub fn save(&mut self, force_overwrite: bool) -> Result<bool, String> {
        let Some(root) = &self.project_root else {
            return Err("Select a project first.".into());
        };
        if self.text.len() > MAX_NOTES_BYTES {
            return Err("Project notes exceed 512 KiB limit.".into());
        }
        let path = Self::notes_path(root);
        let current_mtime = file_mtime_ms(&path);
        if !force_overwrite && current_mtime != self.loaded_mtime_ms && path.exists() {
            let disk_text = fs::read_to_string(&path).unwrap_or_default();
            if disk_text != self.saved_text {
                self.conflict = Some(NotesConflict {
                    disk_text,
                    disk_mtime_ms: current_mtime,
                });
                return Ok(false);
            }
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&path, &self.text).map_err(|e| e.to_string())?;
        self.loaded_mtime_ms = file_mtime_ms(&path);
        self.saved_text = self.text.clone();
        self.conflict = None;
        Ok(true)
    }

    pub fn accept_disk_version(&mut self) {
        if let Some(conflict) = self.conflict.take() {
            self.text = conflict.disk_text.clone();
            self.saved_text = conflict.disk_text;
            self.loaded_mtime_ms = conflict.disk_mtime_ms;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteBlock {
    Heading(u8, String),
    TaskItem { checked: bool, text: String },
    Bullet(String),
    Quote(String),
    CodeBlock(String),
    Paragraph(String),
}

#[must_use]
pub fn parse_note_markdown(text: &str) -> Vec<NoteBlock> {
    let mut blocks = Vec::new();
    let mut in_code = false;
    let mut code_buf = String::new();

    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim_start().starts_with("```") {
            if in_code {
                blocks.push(NoteBlock::CodeBlock(std::mem::take(&mut code_buf)));
                in_code = false;
            } else {
                in_code = true;
            }
            continue;
        }
        if in_code {
            if !code_buf.is_empty() {
                code_buf.push('\n');
            }
            code_buf.push_str(trimmed);
            continue;
        }
        let s = trimmed.trim();
        if s.is_empty() {
            continue;
        }
        if let Some(h) = s.strip_prefix("### ") {
            blocks.push(NoteBlock::Heading(3, h.to_owned()));
        } else if let Some(h) = s.strip_prefix("## ") {
            blocks.push(NoteBlock::Heading(2, h.to_owned()));
        } else if let Some(h) = s.strip_prefix("# ") {
            blocks.push(NoteBlock::Heading(1, h.to_owned()));
        } else if let Some(item) = s
            .strip_prefix("- [x] ")
            .or_else(|| s.strip_prefix("- [X] "))
        {
            blocks.push(NoteBlock::TaskItem {
                checked: true,
                text: item.to_owned(),
            });
        } else if let Some(item) = s.strip_prefix("- [ ] ") {
            blocks.push(NoteBlock::TaskItem {
                checked: false,
                text: item.to_owned(),
            });
        } else if let Some(b) = s.strip_prefix("- ").or_else(|| s.strip_prefix("* ")) {
            blocks.push(NoteBlock::Bullet(b.to_owned()));
        } else if let Some(q) = s.strip_prefix("> ") {
            blocks.push(NoteBlock::Quote(q.to_owned()));
        } else {
            blocks.push(NoteBlock::Paragraph(s.to_owned()));
        }
    }
    if in_code && !code_buf.is_empty() {
        blocks.push(NoteBlock::CodeBlock(code_buf));
    }
    blocks
}

#[must_use]
pub fn format_notes_for_agent(notes: &str) -> String {
    format!(
        "Project notes (`.tessel/notes.md`):\n```markdown\n{}\n```\n",
        notes.trim()
    )
}

// ============================================================================
// 6. SCM Commit History & 5-Color Swimlane Graph (`gitHistoryGraph.js` parity)
// ============================================================================

pub const SWIMLANE_PALETTE: [(u8, u8, u8); 5] = [
    (0x6c, 0x9c, 0xff), // accent blue
    (0x3d, 0xdc, 0x84), // green
    (0xf5, 0xa5, 0x24), // amber
    (0xc0, 0x84, 0xfc), // purple
    (0x38, 0xbd, 0xf8), // sky
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitNode {
    pub hash: String,
    pub short_hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub date_short: String,
    pub refs: Vec<String>,
    pub subject: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitHistoryRow {
    pub node: CommitNode,
    pub lane: usize,
    pub active_lanes: usize,
    pub color_index: usize,
    pub is_merge: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitChangedFile {
    pub status: String,
    pub path: String,
}

fn run_git_safe(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let mut full = Vec::with_capacity(args.len() + 6);
    full.extend_from_slice(&[
        "-c",
        "core.fsmonitor=false",
        "-c",
        "log.showSignature=false",
        "-c",
        "core.hooksPath=/nonexistent-tessel-no-hooks",
    ]);
    full.extend_from_slice(args);
    git::run(cwd, &full, None)
}

#[must_use]
pub fn parse_git_log_output(raw: &str) -> Vec<CommitNode> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.splitn(7, '\x1f');
        let Some(hash) = parts.next() else { continue };
        let Some(short_hash) = parts.next() else {
            continue;
        };
        let parents_raw = parts.next().unwrap_or("");
        let author = parts.next().unwrap_or("").to_owned();
        let date_short = parts.next().unwrap_or("").to_owned();
        let refs_raw = parts.next().unwrap_or("");
        let subject = parts.next().unwrap_or("").to_owned();

        let parents = parents_raw
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        let refs = refs_raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        out.push(CommitNode {
            hash: hash.to_owned(),
            short_hash: short_hash.to_owned(),
            parents,
            author,
            date_short,
            refs,
            subject,
        });
    }
    out
}

#[must_use]
pub fn build_git_history_lanes(nodes: &[CommitNode]) -> Vec<CommitHistoryRow> {
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::with_capacity(nodes.len());

    for node in nodes {
        let lane = if let Some(idx) = lanes
            .iter()
            .position(|slot| slot.as_deref() == Some(node.hash.as_str()))
        {
            idx
        } else if let Some(empty_idx) = lanes.iter().position(Option::is_none) {
            lanes[empty_idx] = Some(node.hash.clone());
            empty_idx
        } else {
            lanes.push(Some(node.hash.clone()));
            lanes.len() - 1
        };

        // Clear duplicate slots pointing to the same commit
        for (idx, slot) in lanes.iter_mut().enumerate() {
            if idx != lane && slot.as_deref() == Some(node.hash.as_str()) {
                *slot = None;
            }
        }

        if let Some(first_parent) = node.parents.first() {
            lanes[lane] = Some(first_parent.clone());
            for extra_parent in node.parents.iter().skip(1) {
                if !lanes
                    .iter()
                    .any(|s| s.as_deref() == Some(extra_parent.as_str()))
                {
                    if let Some(free) = lanes.iter().position(Option::is_none) {
                        lanes[free] = Some(extra_parent.clone());
                    } else if lanes.len() < 8 {
                        lanes.push(Some(extra_parent.clone()));
                    }
                }
            }
        } else {
            lanes[lane] = None;
        }

        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        let active_lanes = lanes.len().max(lane + 1).clamp(1, 8);
        rows.push(CommitHistoryRow {
            is_merge: node.parents.len() > 1,
            lane,
            active_lanes,
            color_index: lane % SWIMLANE_PALETTE.len(),
            node: node.clone(),
        });
    }
    rows
}

pub fn load_git_commit_history(cwd: &Path, limit: usize) -> Result<Vec<CommitHistoryRow>, String> {
    let cap = limit.clamp(1, 300).to_string();
    let raw = run_git_safe(
        cwd,
        &[
            "log",
            "--topo-order",
            "-n",
            &cap,
            "--date=short",
            "--format=%H%x1f%h%x1f%P%x1f%an%x1f%ad%x1f%D%x1f%s",
        ],
    )?;
    let nodes = parse_git_log_output(&raw);
    Ok(build_git_history_lanes(&nodes))
}

pub fn load_commit_files(cwd: &Path, commit_hash: &str) -> Result<Vec<CommitChangedFile>, String> {
    let raw = run_git_safe(
        cwd,
        &[
            "diff-tree",
            "--root",
            "--no-commit-id",
            "--name-status",
            "-r",
            commit_hash,
        ],
    )?;
    let mut files = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.splitn(2, '\t');
        let status = parts.next().unwrap_or("M").trim().to_owned();
        let Some(path) = parts.next() else { continue };
        files.push(CommitChangedFile {
            status,
            path: path.trim().to_owned(),
        });
    }
    Ok(files)
}

pub fn load_commit_file_diff(
    cwd: &Path,
    commit_hash: &str,
    rel_path: &str,
) -> Result<String, String> {
    run_git_safe(cwd, &["show", "--format=", commit_hash, "--", rel_path])
}

// ============================================================================
// 7. Task Branch Review (`ReviewPanel.vue` + `main/review.js` parity)
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewFileEntry {
    pub path: String,
    pub status: String,
    pub added: u32,
    pub removed: u32,
    pub blob_sig: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineReviewComment {
    pub file: String,
    pub line: usize,
    pub body: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewSnapshot {
    pub base_branch: String,
    pub head_branch: String,
    pub merge_base: String,
    pub head_oid: String,
    pub ahead: usize,
    pub behind: usize,
    pub files: Vec<ReviewFileEntry>,
    pub conflicts: Vec<String>,
    pub worktree_dirty: bool,
    pub base_dirty_overlap: Vec<String>,
}

impl ReviewSnapshot {
    #[must_use]
    pub fn can_clean_merge(&self) -> bool {
        !self.head_oid.is_empty()
            && self.ahead > 0
            && self.conflicts.is_empty()
            && !self.worktree_dirty
            && self.base_dirty_overlap.is_empty()
    }
}

pub fn load_branch_review(
    primary_root: &Path,
    base_branch: &str,
    head_branch: &str,
    worktree_root: Option<&Path>,
) -> Result<ReviewSnapshot, String> {
    let head_oid = run_git_safe(primary_root, &["rev-parse", head_branch])?
        .trim()
        .to_owned();
    let merge_base = run_git_safe(primary_root, &["merge-base", base_branch, head_branch])?
        .trim()
        .to_owned();

    let counts_raw = run_git_safe(
        primary_root,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{base_branch}...{head_branch}"),
        ],
    )
    .unwrap_or_default();
    let mut count_parts = counts_raw.split_whitespace();
    let behind = count_parts
        .next()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let ahead = count_parts
        .next()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    let range = format!("{merge_base}..{head_branch}");
    let raw_diff =
        run_git_safe(primary_root, &["diff", "--raw", "--no-abbrev", &range]).unwrap_or_default();
    let mut status_and_sig: BTreeMap<String, (String, String)> = BTreeMap::new();
    for line in raw_diff.lines() {
        // Format: :100644 100644 <old_sha> <new_sha> <status>\t<path>
        let Some((meta, path_part)) = line.split_once('\t') else {
            continue;
        };
        let cols: Vec<&str> = meta.split_whitespace().collect();
        if cols.len() >= 5 {
            let new_sha = cols[3];
            let st = cols[4];
            let target_path = path_part.split('\t').next_back().unwrap_or(path_part);
            status_and_sig.insert(
                target_path.to_owned(),
                (
                    st.chars().next().unwrap_or('M').to_string(),
                    new_sha.to_owned(),
                ),
            );
        }
    }

    let numstat = run_git_safe(primary_root, &["diff", "--numstat", &range]).unwrap_or_default();
    let mut files = Vec::new();
    for line in numstat.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() >= 3 {
            let added = cols[0].parse::<u32>().unwrap_or(0);
            let removed = cols[1].parse::<u32>().unwrap_or(0);
            let path = cols[cols.len() - 1].to_owned();
            let (status, blob_sig) = status_and_sig
                .remove(&path)
                .unwrap_or_else(|| ("M".to_owned(), format!("{added}:{removed}")));
            files.push(ReviewFileEntry {
                path,
                status,
                added,
                removed,
                blob_sig,
            });
        }
    }

    // Conflict simulation via `git merge-tree --write-tree --name-only`
    let conflicts = match run_git_safe(
        primary_root,
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            base_branch,
            head_branch,
        ],
    ) {
        Ok(_) => Vec::new(),
        Err(err_out) => err_out
            .lines()
            .skip(1)
            .map(str::trim)
            .filter(|l| {
                !l.is_empty() && !l.starts_with("CONFLICT ") && !l.starts_with("Auto-merging ")
            })
            .map(str::to_owned)
            .collect(),
    };

    let worktree_dirty = worktree_root.is_some_and(|wt| {
        run_git_safe(wt, &["status", "--porcelain"])
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    });

    let primary_status = run_git_safe(primary_root, &["status", "--porcelain"]).unwrap_or_default();
    let dirty_primary_paths: BTreeSet<String> = primary_status
        .lines()
        .filter_map(|l| l.get(3..))
        .map(|s| s.trim().to_owned())
        .collect();
    let base_dirty_overlap: Vec<String> = files
        .iter()
        .filter(|f| dirty_primary_paths.contains(&f.path))
        .map(|f| f.path.clone())
        .collect();

    Ok(ReviewSnapshot {
        base_branch: base_branch.to_owned(),
        head_branch: head_branch.to_owned(),
        merge_base,
        head_oid,
        ahead,
        behind,
        files,
        conflicts,
        worktree_dirty,
        base_dirty_overlap,
    })
}

pub fn load_review_file_diff(
    primary_root: &Path,
    merge_base: &str,
    head_branch: &str,
    rel_path: &str,
) -> Result<String, String> {
    let range = format!("{merge_base}..{head_branch}");
    run_git_safe(primary_root, &["diff", &range, "--", rel_path])
}

#[must_use]
pub fn format_review_request_changes(
    head_branch: &str,
    general_note: &str,
    comments: &[InlineReviewComment],
) -> String {
    let mut out = format!("Review feedback on branch `{head_branch}`:\n");
    if !general_note.trim().is_empty() {
        out.push('\n');
        out.push_str(general_note.trim());
        out.push('\n');
    }
    if !comments.is_empty() {
        out.push_str("\nInline comments:\n");
        for c in comments {
            out.push_str(&format!("- `{}:{}` — {}\n", c.file, c.line, c.body.trim()));
        }
    }
    out
}

pub fn merge_review_branch(
    primary_root: &Path,
    base_branch: &str,
    head_branch: &str,
    expect_head_oid: &str,
) -> Result<String, String> {
    let actual_head = run_git_safe(primary_root, &["rev-parse", head_branch])?
        .trim()
        .to_owned();
    if actual_head != expect_head_oid {
        return Err(format!(
            "Branch `{head_branch}` moved from `{}` to `{}` since review loaded; refresh before merging.",
            &expect_head_oid[..expect_head_oid.len().min(7)],
            &actual_head[..actual_head.len().min(7)]
        ));
    }
    let current_branch = run_git_safe(primary_root, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_owned();
    if current_branch != base_branch {
        return Err(format!(
            "Primary checkout is on `{current_branch}` (expected `{base_branch}`). Switch to `{base_branch}` before merging."
        ));
    }
    git::run(
        primary_root,
        &[
            "merge",
            "--no-ff",
            "-m",
            &format!("Merge branch '{head_branch}'"),
            head_branch,
        ],
        None,
    )
}
