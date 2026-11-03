//! Pure data models and exact behavioral parity logic for:
//! - Left workspace sidebar (`C:\Tessel\src\renderer\src\sidebarModel.js`, `WorkspaceSidebar.vue`)
//! - Bottom status bar & resource sparklines (`C:\Tessel\src\renderer\src\statusBarModel.js`, `StatusBar.vue`)
//! - Terminal/agent pane header priority badge (`C:\Tessel\src\renderer\src\components\TerminalPane.vue`)

use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarLang {
    #[default]
    En,
    Fr,
}

// ---------------------------------------------------------------------------
// 1. Agent Dot States, Labels & Summaries (`sidebarModel.js:9-147`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneRuntimeKind {
    Shell,
    Agent,
    Chat,
    Editor,
    Browser,
}

impl PaneRuntimeKind {
    #[must_use]
    pub const fn is_agent_pane(self) -> bool {
        matches!(self, Self::Agent | Self::Chat)
    }

    #[must_use]
    pub const fn is_sidebar_pane(self) -> bool {
        matches!(self, Self::Shell | Self::Agent | Self::Chat)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentDotState {
    Waiting,
    Blocked,
    Working,
    Monitoring,
    Interrupted,
    Done,
    Unverifiable,
    Idle,
}

pub const SUMMARY_STATE_ORDER: [AgentDotState; 8] = [
    AgentDotState::Waiting,
    AgentDotState::Blocked,
    AgentDotState::Working,
    AgentDotState::Monitoring,
    AgentDotState::Interrupted,
    AgentDotState::Done,
    AgentDotState::Unverifiable,
    AgentDotState::Idle,
];

#[must_use]
pub fn pane_dot_state(kind: PaneRuntimeKind, sleeping: bool, state: Option<&str>) -> AgentDotState {
    if !kind.is_agent_pane() || sleeping {
        return AgentDotState::Idle;
    }
    match state.unwrap_or("") {
        "working" => AgentDotState::Working,
        "monitoring" => AgentDotState::Monitoring,
        "approval" => AgentDotState::Waiting,
        "limited" => AgentDotState::Blocked,
        "waiting" => AgentDotState::Done,
        "unknown" => AgentDotState::Unverifiable,
        "stopped" | "interrupted" => AgentDotState::Interrupted,
        _ => AgentDotState::Idle,
    }
}

#[must_use]
pub const fn agent_state_label(state: AgentDotState, lang: SidebarLang) -> &'static str {
    match (state, lang) {
        (AgentDotState::Working, SidebarLang::En) => "Working",
        (AgentDotState::Working, SidebarLang::Fr) => "Au travail",
        (AgentDotState::Monitoring, SidebarLang::En) => "Monitoring background tasks",
        (AgentDotState::Monitoring, SidebarLang::Fr) => "Surveille des tâches en arrière-plan",
        (AgentDotState::Blocked, SidebarLang::En) => "Blocked",
        (AgentDotState::Blocked, SidebarLang::Fr) => "Bloqué",
        (AgentDotState::Waiting, SidebarLang::En) => "Waiting for your reply",
        (AgentDotState::Waiting, SidebarLang::Fr) => "En attente de votre réponse",
        (AgentDotState::Interrupted, SidebarLang::En) => "Interrupted",
        (AgentDotState::Interrupted, SidebarLang::Fr) => "Interrompu",
        (AgentDotState::Done, SidebarLang::En) => "Done",
        (AgentDotState::Done, SidebarLang::Fr) => "Terminé",
        (AgentDotState::Idle, SidebarLang::En) => "Idle",
        (AgentDotState::Idle, SidebarLang::Fr) => "Inactif",
        (AgentDotState::Unverifiable, SidebarLang::En) => "No recent updates",
        (AgentDotState::Unverifiable, SidebarLang::Fr) => "Aucune mise à jour récente",
    }
}

#[must_use]
pub const fn summary_state_word(state: AgentDotState, lang: SidebarLang) -> &'static str {
    match (state, lang) {
        (AgentDotState::Unverifiable, SidebarLang::En) => "not heard from",
        (AgentDotState::Unverifiable, SidebarLang::Fr) => "sans nouvelles",
        (AgentDotState::Waiting, SidebarLang::En) => "waiting",
        (AgentDotState::Waiting, SidebarLang::Fr) => "en attente",
        (AgentDotState::Blocked, SidebarLang::En) => "blocked",
        (AgentDotState::Blocked, SidebarLang::Fr) => "bloqué",
        (AgentDotState::Working, SidebarLang::En) => "working",
        (AgentDotState::Working, SidebarLang::Fr) => "au travail",
        (AgentDotState::Monitoring, SidebarLang::En) => "monitoring",
        (AgentDotState::Monitoring, SidebarLang::Fr) => "en surveillance",
        (AgentDotState::Interrupted, SidebarLang::En) => "interrupted",
        (AgentDotState::Interrupted, SidebarLang::Fr) => "interrompu",
        (AgentDotState::Done, SidebarLang::En) => "done",
        (AgentDotState::Done, SidebarLang::Fr) => "terminé",
        (AgentDotState::Idle, SidebarLang::En) => "idle",
        (AgentDotState::Idle, SidebarLang::Fr) => "inactif",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSummaryInput {
    pub agent_id: Option<String>,
    pub name: String,
    pub state: AgentDotState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSummaryGroup {
    pub state: AgentDotState,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSummary {
    pub total: usize,
    pub lead_state: AgentDotState,
    pub groups: Vec<AgentSummaryGroup>,
    pub summary_label: String,
    pub identities: String,
}

#[must_use]
pub fn summarize_agents(
    agents: &[AgentSummaryInput],
    subject: &str,
    lang: SidebarLang,
) -> Option<AgentSummary> {
    if agents.is_empty() {
        return None;
    }
    let subject = if subject.is_empty() {
        match lang {
            SidebarLang::En => "Agents",
            SidebarLang::Fr => "Agents",
        }
    } else {
        subject
    };

    let mut counts: HashMap<AgentDotState, usize> = HashMap::new();
    for a in agents {
        *counts.entry(a.state).or_insert(0) += 1;
    }

    let groups: Vec<AgentSummaryGroup> = SUMMARY_STATE_ORDER
        .iter()
        .copied()
        .filter_map(|state| {
            let count = counts.get(&state).copied().unwrap_or(0);
            (count > 0).then_some(AgentSummaryGroup { state, count })
        })
        .collect();

    let lead_state = groups.first().map_or(AgentDotState::Idle, |g| g.state);

    let summary_label = if agents.len() == 1 {
        format!("{subject} {}", summary_state_word(lead_state, lang))
    } else if groups.len() == 1 {
        match lang {
            SidebarLang::En => format!("{subject}: all {}", summary_state_word(lead_state, lang)),
            SidebarLang::Fr => format!("{subject} : tous {}", summary_state_word(lead_state, lang)),
        }
    } else {
        let parts: Vec<String> = groups
            .iter()
            .map(|g| format!("{} {}", g.count, summary_state_word(g.state, lang)))
            .collect();
        match lang {
            SidebarLang::En => format!("{subject}: {}", parts.join(", ")),
            SidebarLang::Fr => format!("{subject} : {}", parts.join(", ")),
        }
    };

    let identities = agents
        .iter()
        .map(|a| {
            let name = if a.name.is_empty() {
                "Agent"
            } else {
                a.name.as_str()
            };
            format!("{name} {}", summary_state_word(a.state, lang))
        })
        .collect::<Vec<_>>()
        .join(", ");

    Some(AgentSummary {
        total: agents.len(),
        lead_state,
        groups,
        summary_label,
        identities,
    })
}

#[must_use]
pub fn select_summary_group_icon_agents(agents: &[AgentSummaryInput]) -> Vec<&AgentSummaryInput> {
    let Some(summary) = summarize_agents(agents, "Agents", SidebarLang::En) else {
        return Vec::new();
    };
    let Some(first_group) = summary.groups.first() else {
        return Vec::new();
    };
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    for a in agents {
        if a.state != first_group.state {
            continue;
        }
        let Some(id) = a.agent_id.as_deref().filter(|s| !s.is_empty()) else {
            continue;
        };
        if seen.insert(id) {
            out.push(a);
            if out.len() >= 3 {
                break;
            }
        }
    }
    out
}

#[must_use]
pub fn format_short_time_ago(ts_ms: u64, now_ms: u64, lang: SidebarLang) -> String {
    if ts_ms == 0 {
        return String::new();
    }
    let diff = now_ms.saturating_sub(ts_ms);
    if diff < 60_000 {
        return match lang {
            SidebarLang::En => "< 1m".to_string(),
            SidebarLang::Fr => "< 1 min".to_string(),
        };
    }
    let minutes = diff / 60_000;
    if minutes < 60 {
        return match lang {
            SidebarLang::En => format!("{minutes}m"),
            SidebarLang::Fr => format!("{minutes} min"),
        };
    }
    let hours = minutes / 60;
    if hours < 24 {
        return match lang {
            SidebarLang::En => format!("{hours}h"),
            SidebarLang::Fr => format!("{hours} h"),
        };
    }
    let days = hours / 24;
    match lang {
        SidebarLang::En => format!("{days}d"),
        SidebarLang::Fr => format!("{days} j"),
    }
}

// ---------------------------------------------------------------------------
// 2. Card Status, Attention, Project Cards & Worktrees (`sidebarModel.js:151-432`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeCardStatus {
    Permission,
    Working,
    Monitoring,
    Interrupted,
    Done,
    Active,
    Inactive,
    Sleeping,
}

impl WorktreeCardStatus {
    #[must_use]
    pub const fn label(self, lang: SidebarLang) -> &'static str {
        match (self, lang) {
            (Self::Sleeping, SidebarLang::En) => "Sleeping",
            (Self::Sleeping, SidebarLang::Fr) => "En veille",
            (Self::Active, SidebarLang::En) => "Active",
            (Self::Active, SidebarLang::Fr) => "Actif",
            (Self::Working, SidebarLang::En) => "Working",
            (Self::Working, SidebarLang::Fr) => "Au travail",
            (Self::Monitoring, SidebarLang::En) => "Monitoring background tasks",
            (Self::Monitoring, SidebarLang::Fr) => "Surveille des tâches en arrière-plan",
            (Self::Permission, SidebarLang::En) => "Needs permission",
            (Self::Permission, SidebarLang::Fr) => "Autorisation requise",
            (Self::Interrupted, SidebarLang::En) => "Interrupted",
            (Self::Interrupted, SidebarLang::Fr) => "Interrompu",
            (Self::Done, SidebarLang::En) => "Done",
            (Self::Done, SidebarLang::Fr) => "Terminé",
            (Self::Inactive, SidebarLang::En) => "Inactive",
            (Self::Inactive, SidebarLang::Fr) => "Inactif",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarPaneInput {
    pub id: String,
    pub kind: PaneRuntimeKind,
    pub agent_id: Option<String>,
    pub name: String,
    pub title: String,
    pub sleeping: bool,
    pub state: Option<String>,
    pub state_since: u64,
    pub last_output_at: u64,
    pub started_at: u64,
    pub worktree_path: Option<String>,
    pub worktree_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarPaneRow {
    pub id: String,
    pub num: usize,
    pub is_agent: bool,
    pub is_chat: bool,
    pub agent_id: Option<String>,
    pub name: String,
    pub title: String,
    pub state: AgentDotState,
    pub sleeping: bool,
    pub focused: bool,
    pub since: u64,
    pub activity_at: u64,
}

#[must_use]
pub fn pane_row(
    pane: &SidebarPaneInput,
    index: usize,
    focused_id: Option<&str>,
    lang: SidebarLang,
) -> SidebarPaneRow {
    let state = pane_dot_state(pane.kind, pane.sleeping, pane.state.as_deref());
    let since = if pane.state_since > 0 {
        pane.state_since
    } else if pane.last_output_at > 0 {
        pane.last_output_at
    } else {
        pane.started_at
    };
    let activity_at = pane.last_output_at.max(pane.state_since);
    let fallback_name = if pane.kind.is_agent_pane() {
        "Agent"
    } else {
        match lang {
            SidebarLang::En => "Terminal",
            SidebarLang::Fr => "Terminal",
        }
    };
    let name = if !pane.name.is_empty() {
        pane.name.clone()
    } else if !pane.title.is_empty() {
        pane.title.clone()
    } else {
        fallback_name.to_string()
    };
    let title = if pane.title.is_empty() {
        name.clone()
    } else {
        pane.title.clone()
    };

    SidebarPaneRow {
        id: pane.id.clone(),
        num: index + 1,
        is_agent: pane.kind.is_agent_pane(),
        is_chat: pane.kind == PaneRuntimeKind::Chat,
        agent_id: pane.agent_id.clone(),
        name,
        title,
        state,
        sleeping: pane.sleeping,
        focused: focused_id == Some(pane.id.as_str()),
        since,
        activity_at,
    }
}

#[must_use]
pub fn card_status(rows: &[SidebarPaneRow]) -> WorktreeCardStatus {
    let live: Vec<&SidebarPaneRow> = rows.iter().filter(|r| !r.sleeping).collect();
    if live.iter().any(|r| r.state == AgentDotState::Waiting) {
        return WorktreeCardStatus::Permission;
    }
    if live.iter().any(|r| r.state == AgentDotState::Working) {
        return WorktreeCardStatus::Working;
    }
    if live.iter().any(|r| r.state == AgentDotState::Monitoring) {
        return WorktreeCardStatus::Monitoring;
    }
    if live.iter().any(|r| r.state == AgentDotState::Blocked) {
        return WorktreeCardStatus::Interrupted;
    }
    if live.iter().any(|r| r.state == AgentDotState::Done) {
        return WorktreeCardStatus::Done;
    }
    if !live.is_empty() {
        return WorktreeCardStatus::Active;
    }
    WorktreeCardStatus::Inactive
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttentionRank {
    pub cls: u8,
    pub timestamp: u64,
}

#[must_use]
pub fn attention_of(rows: &[SidebarPaneRow]) -> AttentionRank {
    let mut best_cls = 5u8;
    let mut best_since = 0u64;
    for r in rows {
        if !r.is_agent || r.sleeping {
            continue;
        }
        let cls = match r.state {
            AgentDotState::Waiting | AgentDotState::Blocked => 1,
            AgentDotState::Done => 2,
            AgentDotState::Working | AgentDotState::Monitoring => 3,
            AgentDotState::Unverifiable => 4,
            AgentDotState::Interrupted | AgentDotState::Idle => 5,
        };
        if cls < best_cls || (cls == best_cls && r.since > best_since) {
            best_cls = cls;
            best_since = r.since;
        }
    }
    AttentionRank {
        cls: best_cls,
        timestamp: if best_cls == 5 { 0 } else { best_since },
    }
}

#[must_use]
pub fn norm_sidebar_path(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    trimmed.replace('\\', "/").to_lowercase()
}

#[must_use]
pub fn same_path(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && norm_sidebar_path(a) == norm_sidebar_path(b)
}

#[must_use]
pub fn folder_name(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let clean = path.trim_end_matches(['/', '\\']);
    match clean.rfind(['/', '\\']) {
        Some(idx) => clean[idx + 1..].to_string(),
        None => clean.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskCopyInput {
    pub path: String,
    pub branch: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiscoveredWorktree {
    pub path: String,
    pub branch: String,
    pub head: String,
    pub is_primary: bool,
    pub locked: bool,
    pub detached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SidebarProjectInput {
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub branch: String,
    pub default_branch: String,
    pub remote_host_id: Option<String>,
    pub remote_host_label: Option<String>,
    pub is_active_workspace: bool,
    pub active_pane_id: Option<String>,
    pub active_view_key: String,
    pub created_at: u64,
    pub panes: Vec<SidebarPaneInput>,
    pub task_copies: Vec<TaskCopyInput>,
    pub unread_keys: HashSet<String>,
    pub shown_worktree_paths: Vec<String>,
    pub worktrees_inbox_dismissed: bool,
    pub discovered_worktrees: Vec<DiscoveredWorktree>,
    pub collapsed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectCard {
    pub key: String,
    pub project_id: String,
    pub project_name: String,
    pub name: String,
    pub path: String,
    pub branch: String,
    pub is_main: bool,
    pub is_default: bool,
    pub is_external: bool,
    pub locked: bool,
    pub remote_host_id: Option<String>,
    pub remote_host_label: Option<String>,
    pub panes: Vec<SidebarPaneRow>,
    pub agent_count: usize,
    pub status: WorktreeCardStatus,
    pub sleeping: bool,
    pub is_active: bool,
    pub is_unread: bool,
    pub attention: AttentionRank,
    pub last_activity_at: u64,
    pub created_at: u64,
}

struct CopySlot {
    path: String,
    branch: String,
    created_at: u64,
    is_external: bool,
    locked: bool,
}

#[must_use]
pub fn project_other_branches(project: &SidebarProjectInput) -> Vec<&DiscoveredWorktree> {
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for wt in &project.discovered_worktrees {
        if wt.path.is_empty() || wt.is_primary {
            continue;
        }
        if !project.cwd.is_empty() && same_path(&wt.path, &project.cwd) {
            continue;
        }
        if project
            .task_copies
            .iter()
            .any(|c| same_path(&c.path, &wt.path))
        {
            continue;
        }
        if seen.iter().any(|p| same_path(p, &wt.path)) {
            continue;
        }
        seen.push(&wt.path);
        out.push(wt);
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeVisibility<'a> {
    pub shown: Vec<&'a DiscoveredWorktree>,
    pub hidden: Vec<&'a DiscoveredWorktree>,
    pub inbox: Vec<&'a DiscoveredWorktree>,
    pub show_banner: bool,
}

#[must_use]
pub fn project_worktree_visibility<'a>(project: &'a SidebarProjectInput) -> WorktreeVisibility<'a> {
    let others = project_other_branches(project);
    let mut shown = Vec::new();
    let mut hidden = Vec::new();
    for wt in others {
        let is_shown = project
            .shown_worktree_paths
            .iter()
            .any(|p| same_path(p, &wt.path));
        if is_shown {
            shown.push(wt);
        } else {
            hidden.push(wt);
        }
    }
    let inbox: Vec<&'a DiscoveredWorktree> =
        hidden.iter().copied().filter(|w| !w.detached).collect();
    let show_banner = !project.worktrees_inbox_dismissed && !inbox.is_empty();
    WorktreeVisibility {
        shown,
        hidden,
        inbox,
        show_banner,
    }
}

#[must_use]
pub fn build_project_cards(
    project: &SidebarProjectInput,
    now_ms: u64,
    lang: SidebarLang,
) -> Vec<ProjectCard> {
    let mut copies: Vec<CopySlot> = Vec::new();
    for c in &project.task_copies {
        if c.path.is_empty() || copies.iter().any(|x| same_path(&x.path, &c.path)) {
            continue;
        }
        copies.push(CopySlot {
            path: c.path.clone(),
            branch: c.branch.clone(),
            created_at: c.created_at,
            is_external: false,
            locked: false,
        });
    }
    for wt in project_worktree_visibility(project).shown {
        if copies.iter().any(|x| same_path(&x.path, &wt.path)) {
            continue;
        }
        copies.push(CopySlot {
            path: wt.path.clone(),
            branch: wt.branch.clone(),
            created_at: 0,
            is_external: true,
            locked: wt.locked,
        });
    }

    let focused_id = if project.is_active_workspace {
        project.active_pane_id.as_deref()
    } else {
        None
    };

    let mut main_rows: Vec<SidebarPaneRow> = Vec::new();
    let mut copy_rows: Vec<Vec<SidebarPaneRow>> = (0..copies.len()).map(|_| Vec::new()).collect();

    let mut visible_index = 0usize;
    for pane in &project.panes {
        if !pane.kind.is_sidebar_pane() {
            continue;
        }
        let row = pane_row(pane, visible_index, focused_id, lang);
        visible_index += 1;

        let copy_path = pane.worktree_path.as_deref().unwrap_or("");
        if !copy_path.is_empty() && !same_path(copy_path, &project.cwd) {
            if let Some(pos) = copies.iter().position(|c| same_path(&c.path, copy_path)) {
                copy_rows[pos].push(row);
            } else {
                copies.push(CopySlot {
                    path: copy_path.to_string(),
                    branch: pane.worktree_branch.clone().unwrap_or_default(),
                    created_at: 0,
                    is_external: false,
                    locked: false,
                });
                copy_rows.push(vec![row]);
            }
        } else {
            main_rows.push(row);
        }
    }

    let _ = now_ms; // kept in signature for parity with `buildProjectCards(project, now)`
    let default_branch = &project.default_branch;

    let make_card = |slot: Option<&CopySlot>, rows: Vec<SidebarPaneRow>| -> ProjectCard {
        let is_main = slot.is_none();
        let path = slot.map_or_else(|| project.cwd.clone(), |s| s.path.clone());
        let branch = slot.map_or_else(|| project.branch.clone(), |s| s.branch.clone());
        let key = if is_main {
            format!("{}::", project.id)
        } else {
            format!("{}::{path}", project.id)
        };
        let status = card_status(&rows);
        // Matches JS `[].every((r) => r.sleeping) === true` when `rows` is empty.
        let sleeping = rows.iter().all(|r| r.sleeping);
        let attention = attention_of(&rows);
        let last_activity_at = rows.iter().map(|r| r.activity_at).max().unwrap_or(0);
        let is_active = if project.is_active_workspace {
            if project.active_view_key.is_empty() {
                is_main
            } else {
                !is_main && same_path(&path, &project.active_view_key)
            }
        } else {
            false
        };
        let is_unread = !is_active && project.unread_keys.contains(&key);
        let is_default = if !default_branch.is_empty() && !branch.is_empty() {
            branch == *default_branch
        } else {
            is_main
        };
        let name = if is_main {
            project.name.clone()
        } else {
            let folder = folder_name(&path);
            if !folder.is_empty() {
                folder
            } else if !branch.is_empty() {
                branch.clone()
            } else {
                project.name.clone()
            }
        };
        let agent_count = rows.iter().filter(|r| r.is_agent).count();
        let created_at = slot.map_or(project.created_at, |s| s.created_at);

        ProjectCard {
            key,
            project_id: project.id.clone(),
            project_name: project.name.clone(),
            name,
            path,
            branch,
            is_main,
            is_default,
            is_external: slot.is_some_and(|s| s.is_external),
            locked: slot.is_some_and(|s| s.locked),
            remote_host_id: project.remote_host_id.clone(),
            remote_host_label: project.remote_host_label.clone(),
            panes: rows,
            agent_count,
            status,
            sleeping,
            is_active,
            is_unread,
            attention,
            last_activity_at,
            created_at,
        }
    };

    let mut out = Vec::with_capacity(1 + copies.len());
    out.push(make_card(None, main_rows));
    for (slot, rows) in copies.iter().zip(copy_rows) {
        out.push(make_card(Some(slot), rows));
    }
    out
}

#[must_use]
pub fn worktree_parent_path(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let clean = path.trim_end_matches(['/', '\\']);
    let Some(idx) = clean.rfind(['/', '\\']) else {
        return String::new();
    };
    if idx == 0 {
        return "/".to_string();
    }
    let bytes = clean.as_bytes();
    if idx == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return clean[..3].to_string();
    }
    clean[..idx].to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeGroup<'a> {
    pub parent_path: String,
    pub worktrees: Vec<&'a DiscoveredWorktree>,
}

#[must_use]
pub fn group_worktrees_by_parent_path<'a>(
    worktrees: &[&'a DiscoveredWorktree],
) -> Vec<WorktreeGroup<'a>> {
    let mut groups: Vec<WorktreeGroup<'a>> = Vec::new();
    for &wt in worktrees {
        let parent = worktree_parent_path(&wt.path);
        if let Some(existing) = groups.iter_mut().find(|g| {
            same_path(&g.parent_path, &parent) || (g.parent_path.is_empty() && parent.is_empty())
        }) {
            existing.worktrees.push(wt);
        } else {
            groups.push(WorktreeGroup {
                parent_path: parent,
                worktrees: vec![wt],
            });
        }
    }
    for group in &mut groups {
        group.worktrees.sort_by(|a, b| {
            folder_name(&a.path)
                .to_lowercase()
                .cmp(&folder_name(&b.path).to_lowercase())
        });
    }
    groups.sort_by(|a, b| {
        b.worktrees.len().cmp(&a.worktrees.len()).then_with(|| {
            a.parent_path
                .to_lowercase()
                .cmp(&b.parent_path.to_lowercase())
        })
    });
    groups
}

pub const HIDDEN_GROUP_LIMIT: usize = 5;
pub const HIDDEN_PREVIEW_LIMIT: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenWorktreeGroupPreview<'a> {
    pub parent_path: String,
    pub shown: Vec<&'a DiscoveredWorktree>,
    pub total: usize,
    pub remaining: usize,
    pub expanded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenWorktreesPreview<'a> {
    pub groups: Vec<HiddenWorktreeGroupPreview<'a>>,
    pub overflow_groups: usize,
}

#[must_use]
pub fn hidden_worktrees_preview<'a>(
    worktrees: &[&'a DiscoveredWorktree],
    expanded_parents: &HashSet<String>,
) -> HiddenWorktreesPreview<'a> {
    let all = group_worktrees_by_parent_path(worktrees);
    let groups = all
        .iter()
        .take(HIDDEN_GROUP_LIMIT)
        .map(|g| {
            let expanded = expanded_parents.iter().any(|p| {
                same_path(p, &g.parent_path) || (p.is_empty() && g.parent_path.is_empty())
            });
            let shown = if expanded {
                g.worktrees.clone()
            } else {
                g.worktrees
                    .iter()
                    .copied()
                    .take(HIDDEN_PREVIEW_LIMIT)
                    .collect()
            };
            let total = g.worktrees.len();
            let remaining = total.saturating_sub(shown.len());
            HiddenWorktreeGroupPreview {
                parent_path: g.parent_path.clone(),
                shown,
                total,
                remaining,
                expanded,
            }
        })
        .collect();
    HiddenWorktreesPreview {
        groups,
        overflow_groups: all.len().saturating_sub(HIDDEN_GROUP_LIMIT),
    }
}

// ---------------------------------------------------------------------------
// 3. Card Sorting, Visibility, Rows & Reordering (`sidebarModel.js:436-722`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardSortBy {
    Name,
    #[default]
    Smart,
    Recent,
    Repo,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarGroupBy {
    None,
    #[default]
    Repo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectOrderBy {
    #[default]
    Manual,
    Recent,
}

#[must_use]
pub fn compare_cards(a: &ProjectCard, b: &ProjectCard, sort_by: CardSortBy) -> std::cmp::Ordering {
    match sort_by {
        CardSortBy::Name => a
            .name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| {
                a.project_name
                    .to_lowercase()
                    .cmp(&b.project_name.to_lowercase())
            }),
        CardSortBy::Recent => b
            .last_activity_at
            .cmp(&a.last_activity_at)
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        CardSortBy::Repo => a
            .project_name
            .to_lowercase()
            .cmp(&b.project_name.to_lowercase())
            .then_with(|| b.is_main.cmp(&a.is_main))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        CardSortBy::Manual => b
            .is_main
            .cmp(&a.is_main)
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        CardSortBy::Smart => a
            .attention
            .cls
            .cmp(&b.attention.cls)
            .then_with(|| b.attention.timestamp.cmp(&a.attention.timestamp))
            .then_with(|| b.last_activity_at.cmp(&a.last_activity_at))
            .then_with(|| b.is_main.cmp(&a.is_main))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
    }
}

#[derive(Debug, Clone, Default)]
pub struct CardVisibilityOpts {
    pub project_filter: Option<HashSet<String>>,
    pub search_query: String,
    pub hide_sleeping: bool,
    pub except_default_branch: bool,
    pub hide_default_branch: bool,
}

#[must_use]
pub fn is_card_visible(card: &ProjectCard, opts: &CardVisibilityOpts) -> bool {
    if let Some(allowed) = &opts.project_filter
        && !allowed.contains(&card.project_id)
    {
        return false;
    }
    let q = opts.search_query.trim().to_lowercase();
    if !q.is_empty() {
        let mut hay = format!(
            "{} {} {} {}",
            card.name, card.project_name, card.branch, card.path
        )
        .to_lowercase();
        for p in &card.panes {
            hay.push(' ');
            hay.push_str(&p.name.to_lowercase());
            hay.push(' ');
            hay.push_str(&p.title.to_lowercase());
        }
        if !hay.contains(&q) {
            return false;
        }
    }
    // The active workspace card is always kept visible even when sleeping/default filters apply.
    if card.is_active {
        return true;
    }
    if opts.hide_default_branch && card.is_default {
        return false;
    }
    if opts.hide_sleeping && card.sleeping {
        if opts.except_default_branch && card.is_default {
            return true;
        }
        return false;
    }
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFolder {
    pub id: String,
    pub name: String,
    pub project_ids: Vec<String>,
    pub collapsed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SidebarBuildOpts {
    pub group_by: SidebarGroupBy,
    pub sort_by: CardSortBy,
    pub project_order_by: ProjectOrderBy,
    pub project_order: Vec<String>,
    pub folders: Vec<ProjectFolder>,
    pub visibility: CardVisibilityOpts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarRow {
    Folder {
        key: String,
        folder_id: String,
        name: String,
        count: usize,
        collapsed: bool,
    },
    Project {
        key: String,
        project_id: String,
        project_name: String,
        folder_id: Option<String>,
        total_cards: usize,
        visible_cards: usize,
        hidden_by_filter: usize,
        collapsed: bool,
        attention: AttentionRank,
        any_unread: bool,
        any_active: bool,
    },
    Card {
        key: String,
        folder_id: Option<String>,
        card: Box<ProjectCard>,
    },
    HiddenSummary {
        key: String,
        project_id: String,
        folder_id: Option<String>,
        count: usize,
    },
}

struct ProjectEntry<'a> {
    project: &'a SidebarProjectInput,
    all: Vec<ProjectCard>,
    visible: Vec<ProjectCard>,
    last_activity: u64,
    attention: AttentionRank,
    any_unread: bool,
    any_active: bool,
}

#[must_use]
pub fn build_sidebar_rows(
    projects: &[SidebarProjectInput],
    opts: &SidebarBuildOpts,
    now_ms: u64,
    lang: SidebarLang,
) -> Vec<SidebarRow> {
    let mut per_project: Vec<ProjectEntry<'_>> = projects
        .iter()
        .map(|p| {
            let all = build_project_cards(p, now_ms, lang);
            let visible: Vec<ProjectCard> = all
                .iter()
                .filter(|c| is_card_visible(c, &opts.visibility))
                .cloned()
                .collect();
            let last_activity = all.iter().map(|c| c.last_activity_at).max().unwrap_or(0);
            let all_panes: Vec<SidebarPaneRow> =
                all.iter().flat_map(|c| c.panes.iter().cloned()).collect();
            let attention = attention_of(&all_panes);
            let any_unread = all.iter().any(|c| c.is_unread);
            let any_active = all.iter().any(|c| c.is_active);
            ProjectEntry {
                project: p,
                all,
                visible,
                last_activity,
                attention,
                any_unread,
                any_active,
            }
        })
        .collect();

    if opts.group_by == SidebarGroupBy::None {
        let mut flat: Vec<ProjectCard> = per_project.into_iter().flat_map(|e| e.visible).collect();
        flat.sort_by(|a, b| compare_cards(a, b, opts.sort_by));
        return flat
            .into_iter()
            .map(|c| SidebarRow::Card {
                key: c.key.clone(),
                folder_id: None,
                card: Box::new(c),
            })
            .collect();
    }

    let order_index: HashMap<&str, usize> = opts
        .project_order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();

    match opts.project_order_by {
        ProjectOrderBy::Recent => {
            per_project.sort_by(|a, b| {
                b.last_activity.cmp(&a.last_activity).then_with(|| {
                    a.project
                        .name
                        .to_lowercase()
                        .cmp(&b.project.name.to_lowercase())
                })
            });
        }
        ProjectOrderBy::Manual => {
            if !order_index.is_empty() {
                per_project.sort_by_key(|e| {
                    order_index
                        .get(e.project.id.as_str())
                        .copied()
                        .unwrap_or(usize::MAX)
                });
            }
        }
    }

    let has_query = !opts.visibility.search_query.trim().is_empty();
    let mut rows: Vec<SidebarRow> = Vec::new();

    let push_project =
        |rows: &mut Vec<SidebarRow>, entry: &ProjectEntry<'_>, folder_id: Option<String>| {
            if let Some(allowed) = &opts.visibility.project_filter
                && !allowed.contains(&entry.project.id)
            {
                return;
            }
            if has_query && entry.visible.is_empty() {
                return;
            }
            let mut sorted = entry.visible.clone();
            sorted.sort_by(|a, b| compare_cards(a, b, opts.sort_by));
            let hidden_by_filter = entry.all.len().saturating_sub(sorted.len());

            rows.push(SidebarRow::Project {
                key: format!("project:{}", entry.project.id),
                project_id: entry.project.id.clone(),
                project_name: entry.project.name.clone(),
                folder_id: folder_id.clone(),
                total_cards: entry.all.len(),
                visible_cards: sorted.len(),
                hidden_by_filter,
                collapsed: entry.project.collapsed,
                attention: entry.attention,
                any_unread: entry.any_unread,
                any_active: entry.any_active,
            });

            if !entry.project.collapsed {
                for card in sorted {
                    rows.push(SidebarRow::Card {
                        key: card.key.clone(),
                        folder_id: folder_id.clone(),
                        card: Box::new(card),
                    });
                }
                if entry.visible.is_empty() && hidden_by_filter > 0 {
                    rows.push(SidebarRow::HiddenSummary {
                        key: format!("project:{}:hidden", entry.project.id),
                        project_id: entry.project.id.clone(),
                        folder_id,
                        count: hidden_by_filter,
                    });
                }
            }
        };

    let mut folder_of: HashMap<&str, &ProjectFolder> = HashMap::new();
    for f in &opts.folders {
        for pid in &f.project_ids {
            if !pid.is_empty() && !folder_of.contains_key(pid.as_str()) {
                folder_of.insert(pid.as_str(), f);
            }
        }
    }

    let mut folder_members: HashMap<&str, Vec<&ProjectEntry<'_>>> = HashMap::new();
    for entry in &per_project {
        if let Some(f) = folder_of.get(entry.project.id.as_str()) {
            folder_members.entry(f.id.as_str()).or_default().push(entry);
        }
    }

    let mut placed_folders: HashSet<&str> = HashSet::new();
    let push_folder =
        |rows: &mut Vec<SidebarRow>, f: &ProjectFolder, members: &[&ProjectEntry<'_>]| {
            if has_query && !members.iter().any(|m| !m.visible.is_empty()) {
                return;
            }
            if let Some(allowed) = &opts.visibility.project_filter
                && !members.is_empty()
                && !members.iter().any(|m| allowed.contains(&m.project.id))
            {
                return;
            }
            rows.push(SidebarRow::Folder {
                key: format!("folder:{}", f.id),
                folder_id: f.id.clone(),
                name: f.name.clone(),
                count: members.len(),
                collapsed: f.collapsed,
            });
            if !f.collapsed || has_query {
                for m in members {
                    push_project(rows, m, Some(f.id.clone()));
                }
            }
        };

    for entry in &per_project {
        if let Some(&f) = folder_of.get(entry.project.id.as_str()) {
            if placed_folders.insert(f.id.as_str()) {
                let members = folder_members
                    .get(f.id.as_str())
                    .map_or(&[][..], Vec::as_slice);
                push_folder(&mut rows, f, members);
            }
        } else {
            push_project(&mut rows, entry, None);
        }
    }

    // Empty user folders appear at the end so newly created folders are immediately visible.
    for f in &opts.folders {
        if placed_folders.insert(f.id.as_str()) {
            push_folder(&mut rows, f, &[]);
        }
    }

    rows
}

#[must_use]
pub fn project_drop_index(ordered_ids: &[String], target_id: &str, after: bool) -> Option<usize> {
    if target_id.is_empty() {
        return None;
    }
    let idx = ordered_ids.iter().position(|id| id == target_id)?;
    Some(idx + usize::from(after))
}

#[must_use]
pub fn move_to_index<T: Clone + PartialEq>(list: &[T], item: &T, insert_before: usize) -> Vec<T> {
    let Some(from) = list.iter().position(|x| x == item) else {
        return list.to_vec();
    };
    let mut next = list.to_vec();
    let removed = next.remove(from);
    let mut to = if from < insert_before {
        insert_before - 1
    } else {
        insert_before
    };
    to = to.min(next.len());
    next.insert(to, removed);
    next
}

#[must_use]
pub fn merge_project_order(saved_order: &[String], current_ids: &[String]) -> Vec<String> {
    let current_set: HashSet<&str> = current_ids.iter().map(String::as_str).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::with_capacity(current_ids.len());
    for id in saved_order {
        if current_set.contains(id.as_str()) && seen.insert(id.as_str()) {
            out.push(id.clone());
        }
    }
    for id in current_ids {
        if seen.insert(id.as_str()) {
            out.push(id.clone());
        }
    }
    out
}

#[must_use]
pub fn order_workspaces<'a>(
    workspaces: &'a [SidebarProjectInput],
    project_order_by: ProjectOrderBy,
    project_order: &[String],
) -> Vec<&'a SidebarProjectInput> {
    if workspaces.len() < 2 {
        return workspaces.iter().collect();
    }
    match project_order_by {
        ProjectOrderBy::Recent => {
            let mut copy: Vec<&SidebarProjectInput> = workspaces.iter().collect();
            copy.sort_by(|a, b| {
                let last_of = |ws: &SidebarProjectInput| -> u64 {
                    ws.panes
                        .iter()
                        .filter(|p| p.kind.is_sidebar_pane())
                        .map(|p| p.last_output_at.max(p.state_since))
                        .max()
                        .unwrap_or(0)
                };
                last_of(b)
                    .cmp(&last_of(a))
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            copy
        }
        ProjectOrderBy::Manual => {
            if project_order.is_empty() {
                return workspaces.iter().collect();
            }
            let ids: Vec<String> = workspaces.iter().map(|w| w.id.clone()).collect();
            let merged = merge_project_order(project_order, &ids);
            let by_id: HashMap<&str, &SidebarProjectInput> =
                workspaces.iter().map(|w| (w.id.as_str(), w)).collect();
            merged
                .iter()
                .filter_map(|id| by_id.get(id.as_str()).copied())
                .collect()
        }
    }
}

#[must_use]
pub fn neighbor_card<'a>(
    rows: &'a [SidebarRow],
    active_key: &str,
    delta: isize,
) -> Option<&'a ProjectCard> {
    let cards: Vec<&ProjectCard> = rows
        .iter()
        .filter_map(|r| match r {
            SidebarRow::Card { card, .. } => Some(card.as_ref()),
            _ => None,
        })
        .collect();
    if cards.is_empty() {
        return None;
    }
    let len = cards.len().cast_signed();
    let idx = cards
        .iter()
        .position(|c| c.key == active_key)
        .map_or(0isize, |i| i.cast_signed());
    let next = (idx + delta).rem_euclid(len).cast_unsigned();
    cards.get(next).copied()
}

#[must_use]
pub fn card_target_pane(card: &ProjectCard) -> Option<&str> {
    if card.panes.is_empty() {
        return None;
    }
    if let Some(focused) = card.panes.iter().find(|r| r.focused) {
        return Some(focused.id.as_str());
    }
    if let Some(recent) = card
        .panes
        .iter()
        .filter(|r| r.activity_at > 0)
        .max_by_key(|r| r.activity_at)
    {
        return Some(recent.id.as_str());
    }
    if let Some(agent) = card.panes.iter().find(|r| r.is_agent) {
        return Some(agent.id.as_str());
    }
    card.panes.first().map(|r| r.id.as_str())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortProbe {
    pub key: String,
    pub path: Option<String>,
    pub pids: Vec<u32>,
}

#[must_use]
pub fn port_probes(
    projects: &[SidebarProjectInput],
    pids_by_pane: &HashMap<String, u32>,
    now_ms: u64,
) -> Vec<PortProbe> {
    let mut probes = Vec::new();
    for p in projects {
        if p.remote_host_id.is_some() {
            continue;
        }
        for card in build_project_cards(p, now_ms, SidebarLang::En) {
            let pids: Vec<u32> = card
                .panes
                .iter()
                .filter(|r| !r.sleeping)
                .filter_map(|r| pids_by_pane.get(&r.id).copied())
                .filter(|&pid| pid > 0)
                .collect();
            let path = if !card.path.is_empty() && (!card.is_main || !card.branch.is_empty()) {
                Some(card.path.clone())
            } else {
                None
            };
            if path.is_some() || !pids.is_empty() {
                probes.push(PortProbe {
                    key: card.key,
                    path,
                    pids,
                });
            }
        }
    }
    probes
}

// ---------------------------------------------------------------------------
// 4. Bottom Status Bar & Resource Sparklines (`statusBarModel.js` + `StatusBar.vue`)
// ---------------------------------------------------------------------------

pub const STATUS_BAR_COMPACT_BELOW: f32 = 900.0;
pub const STATUS_BAR_ICON_ONLY_BELOW: f32 = 500.0;
pub const RESOURCE_OPEN_POLL_MS: u64 = 2_000;
pub const RESOURCE_BACKGROUND_POLL_MS: u64 = 30_000;

pub const HISTORY_CAPACITY: usize = 60;
pub const HISTORY_STALE_MS: u64 = 10 * 60 * 1000;
pub const APP_HISTORY_KEY: &str = "__app__";
pub const SPARK_SLOTS: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusBarDensity {
    Full,
    Compact,
    IconOnly,
}

#[must_use]
pub fn status_bar_density(width_px: f32) -> StatusBarDensity {
    if width_px < STATUS_BAR_ICON_ONLY_BELOW {
        StatusBarDensity::IconOnly
    } else if width_px < STATUS_BAR_COMPACT_BELOW {
        StatusBarDensity::Compact
    } else {
        StatusBarDensity::Full
    }
}

fn localized_decimal(formatted: String, lang: SidebarLang) -> String {
    match lang {
        SidebarLang::En => formatted,
        SidebarLang::Fr => formatted.replace('.', ","),
    }
}

#[must_use]
pub fn format_memory(bytes: Option<f64>, lang: SidebarLang) -> String {
    let Some(bytes) = bytes.filter(|b| b.is_finite() && *b >= 0.0) else {
        return "—".to_string();
    };
    let mb = bytes / (1024.0 * 1024.0);
    if mb < 1.0 {
        let kb = (bytes / 1024.0).round() as u64;
        return match lang {
            SidebarLang::En => format!("{kb} KB"),
            SidebarLang::Fr => format!("{kb} Ko"),
        };
    }
    if mb < 1024.0 {
        let val = localized_decimal(format!("{mb:.1}"), lang);
        return match lang {
            SidebarLang::En => format!("{val} MB"),
            SidebarLang::Fr => format!("{val} Mo"),
        };
    }
    let gb = mb / 1024.0;
    let val = localized_decimal(format!("{gb:.2}"), lang);
    match lang {
        SidebarLang::En => format!("{val} GB"),
        SidebarLang::Fr => format!("{val} Go"),
    }
}

#[must_use]
pub fn format_cpu(percent: Option<f64>, lang: SidebarLang) -> String {
    let Some(percent) = percent.filter(|p| p.is_finite() && *p >= 0.0) else {
        return "—".to_string();
    };
    let val = localized_decimal(format!("{percent:.1}"), lang);
    match lang {
        SidebarLang::En => format!("{val}%"),
        SidebarLang::Fr => format!("{val} %"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeepAwakeMode {
    On,
    #[default]
    Agents,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwakeCopy {
    pub label: &'static str,
    pub hint: &'static str,
    pub state: &'static str,
    pub status: String,
    pub active: bool,
}

#[must_use]
pub fn awake_copy(mode: KeepAwakeMode, active: bool, lang: SidebarLang) -> AwakeCopy {
    let (label, hint) = match (mode, lang) {
        (KeepAwakeMode::On, SidebarLang::En) => ("On", "Keep this computer awake at all times"),
        (KeepAwakeMode::On, SidebarLang::Fr) => {
            ("Activé", "Maintenir cet ordinateur éveillé en permanence")
        }
        (KeepAwakeMode::Agents, SidebarLang::En) => {
            ("Agent", "Stay awake while an agent is working")
        }
        (KeepAwakeMode::Agents, SidebarLang::Fr) => {
            ("Agent", "Rester éveillé pendant qu'un agent travaille")
        }
        (KeepAwakeMode::Off, SidebarLang::En) => ("Off", "Allow normal system sleep behavior"),
        (KeepAwakeMode::Off, SidebarLang::Fr) => (
            "Désactivé",
            "Autoriser le comportement de veille normal du système",
        ),
    };
    let state = match (active, lang) {
        (true, SidebarLang::En) => "Active",
        (true, SidebarLang::Fr) => "Actif",
        (false, SidebarLang::En) => "Inactive",
        (false, SidebarLang::Fr) => "Inactif",
    };
    AwakeCopy {
        label,
        hint,
        state,
        status: format!("{label} · {state}"),
        active,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResourceSortBy {
    #[default]
    Memory,
    Cpu,
    Name,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppResourceEntry {
    pub kind: String,
    pub memory_bytes: f64,
    pub cpu_percent: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerminalResourceSnapshot {
    pub id: String,
    pub memory_bytes: f64,
    pub private_bytes: f64,
    pub cpu_percent: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResourceSnapshot {
    pub timestamp_ms: u64,
    pub total_memory_bytes: f64,
    pub total_private_bytes: f64,
    pub total_cpu_percent: f64,
    pub app: Vec<AppResourceEntry>,
    pub terminals: HashMap<String, TerminalResourceSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalMeta {
    pub id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub name: String,
    pub is_agent: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppSubRow {
    pub kind: String,
    pub label: String,
    pub memory_bytes: f64,
    pub cpu_percent: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerminalResourceRow {
    pub id: String,
    pub name: String,
    pub is_agent: bool,
    pub memory_bytes: f64,
    pub private_bytes: f64,
    pub cpu_percent: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceResourceGroup {
    pub id: String,
    pub name: String,
    pub memory_bytes: f64,
    pub private_bytes: f64,
    pub cpu_percent: f64,
    pub rows: Vec<TerminalResourceRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceTree {
    pub app_memory_bytes: f64,
    pub app_cpu_percent: f64,
    pub app_rows: Vec<AppSubRow>,
    pub groups: Vec<WorkspaceResourceGroup>,
    pub total_memory_bytes: f64,
    pub total_private_bytes: f64,
    pub total_cpu_percent: f64,
    pub terminal_count: usize,
}

fn compare_resource(
    a_mem: f64,
    a_cpu: f64,
    a_name: &str,
    b_mem: f64,
    b_cpu: f64,
    b_name: &str,
    sort_by: ResourceSortBy,
) -> std::cmp::Ordering {
    match sort_by {
        ResourceSortBy::Cpu => b_cpu
            .total_cmp(&a_cpu)
            .then_with(|| b_mem.total_cmp(&a_mem))
            .then_with(|| a_name.to_lowercase().cmp(&b_name.to_lowercase())),
        ResourceSortBy::Name => a_name
            .to_lowercase()
            .cmp(&b_name.to_lowercase())
            .then_with(|| b_mem.total_cmp(&a_mem)),
        ResourceSortBy::Memory => b_mem
            .total_cmp(&a_mem)
            .then_with(|| b_cpu.total_cmp(&a_cpu))
            .then_with(|| a_name.to_lowercase().cmp(&b_name.to_lowercase())),
    }
}

#[must_use]
pub fn resource_tree(
    snapshot: Option<&ResourceSnapshot>,
    terminals: &[TerminalMeta],
    sort_by: ResourceSortBy,
    lang: SidebarLang,
) -> Option<ResourceTree> {
    let snap = snapshot?;
    let mut app_rows: Vec<AppSubRow> = snap
        .app
        .iter()
        .map(|a| {
            let label = match (a.kind.as_str(), lang) {
                ("main", SidebarLang::En) => "Main".to_string(),
                ("main", SidebarLang::Fr) => "Principal".to_string(),
                ("renderer", SidebarLang::En) => "Renderer".to_string(),
                ("renderer", SidebarLang::Fr) => "Rendu".to_string(),
                ("other", SidebarLang::En) => "Other".to_string(),
                ("other", SidebarLang::Fr) => "Autre".to_string(),
                (other, _) => other.to_string(),
            };
            AppSubRow {
                kind: a.kind.clone(),
                label,
                memory_bytes: a.memory_bytes,
                cpu_percent: a.cpu_percent,
            }
        })
        .collect();
    app_rows.sort_by(|a, b| {
        compare_resource(
            a.memory_bytes,
            a.cpu_percent,
            &a.label,
            b.memory_bytes,
            b.cpu_percent,
            &b.label,
            sort_by,
        )
    });

    let app_memory_bytes: f64 = app_rows.iter().map(|r| r.memory_bytes).sum();
    let app_cpu_percent: f64 = app_rows.iter().map(|r| r.cpu_percent).sum();

    let mut groups_by_ws: BTreeMap<String, WorkspaceResourceGroup> = BTreeMap::new();
    let mut ws_order: Vec<String> = Vec::new();
    let mut terminal_count = 0usize;

    for meta in terminals {
        let Some(usage) = snap.terminals.get(&meta.id) else {
            continue;
        };
        terminal_count += 1;
        let ws_key = if meta.workspace_id.is_empty() {
            "__other__".to_string()
        } else {
            meta.workspace_id.clone()
        };
        let ws_name = if meta.workspace_name.is_empty() {
            match lang {
                SidebarLang::En => "Terminals".to_string(),
                SidebarLang::Fr => "Terminaux".to_string(),
            }
        } else {
            meta.workspace_name.clone()
        };
        if !groups_by_ws.contains_key(&ws_key) {
            ws_order.push(ws_key.clone());
            groups_by_ws.insert(
                ws_key.clone(),
                WorkspaceResourceGroup {
                    id: ws_key.clone(),
                    name: ws_name,
                    memory_bytes: 0.0,
                    private_bytes: 0.0,
                    cpu_percent: 0.0,
                    rows: Vec::new(),
                },
            );
        }
        if let Some(g) = groups_by_ws.get_mut(&ws_key) {
            g.memory_bytes += usage.memory_bytes;
            g.private_bytes += usage.private_bytes;
            g.cpu_percent += usage.cpu_percent;
            g.rows.push(TerminalResourceRow {
                id: meta.id.clone(),
                name: if meta.name.is_empty() {
                    "Terminal".to_string()
                } else {
                    meta.name.clone()
                },
                is_agent: meta.is_agent,
                memory_bytes: usage.memory_bytes,
                private_bytes: usage.private_bytes,
                cpu_percent: usage.cpu_percent,
            });
        }
    }

    let mut groups: Vec<WorkspaceResourceGroup> = ws_order
        .into_iter()
        .filter_map(|k| groups_by_ws.remove(&k))
        .collect();
    for g in &mut groups {
        g.rows.sort_by(|a, b| {
            compare_resource(
                a.memory_bytes,
                a.cpu_percent,
                &a.name,
                b.memory_bytes,
                b.cpu_percent,
                &b.name,
                sort_by,
            )
        });
    }
    groups.sort_by(|a, b| {
        compare_resource(
            a.memory_bytes,
            a.cpu_percent,
            &a.name,
            b.memory_bytes,
            b.cpu_percent,
            &b.name,
            sort_by,
        )
    });

    Some(ResourceTree {
        app_memory_bytes,
        app_cpu_percent,
        app_rows,
        groups,
        total_memory_bytes: snap.total_memory_bytes,
        total_private_bytes: snap.total_private_bytes,
        total_cpu_percent: snap.total_cpu_percent,
        terminal_count,
    })
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResourceSeries {
    pub memory: Vec<f64>,
    pub cpu: Vec<f64>,
    pub last_seen_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResourceHistory {
    entries: HashMap<String, ResourceSeries>,
}

impl ResourceHistory {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn push_sample(&mut self, key: &str, mem: f64, cpu: f64, at_ms: u64) {
        let entry = self.entries.entry(key.to_string()).or_default();
        entry.memory.push(if mem.is_finite() { mem } else { 0.0 });
        entry.cpu.push(if cpu.is_finite() { cpu } else { 0.0 });
        if entry.memory.len() > HISTORY_CAPACITY {
            let overflow = entry.memory.len() - HISTORY_CAPACITY;
            entry.memory.drain(0..overflow);
        }
        if entry.cpu.len() > HISTORY_CAPACITY {
            let overflow = entry.cpu.len() - HISTORY_CAPACITY;
            entry.cpu.drain(0..overflow);
        }
        entry.last_seen_ms = at_ms;
    }

    pub fn record_snapshot(&mut self, snap: &ResourceSnapshot, now_ms: u64) {
        let at_ms = if snap.timestamp_ms > 0 {
            snap.timestamp_ms
        } else {
            now_ms
        };
        let app_mem: f64 = snap.app.iter().map(|a| a.memory_bytes).sum();
        let app_cpu: f64 = snap.app.iter().map(|a| a.cpu_percent).sum();
        self.push_sample(APP_HISTORY_KEY, app_mem, app_cpu, at_ms);

        for (id, usage) in &snap.terminals {
            self.push_sample(id, usage.memory_bytes, usage.cpu_percent, at_ms);
        }

        let live: HashSet<&str> = snap.terminals.keys().map(String::as_str).collect();
        self.prune(&live, at_ms);
    }

    pub fn prune(&mut self, live_ids: &HashSet<&str>, now_ms: u64) {
        self.entries.retain(|key, entry| {
            key == APP_HISTORY_KEY
                || live_ids.contains(key.as_str())
                || now_ms.saturating_sub(entry.last_seen_ms) <= HISTORY_STALE_MS
        });
    }

    #[must_use]
    pub fn series(&self, key: &str) -> Option<&ResourceSeries> {
        self.entries.get(key)
    }
}

#[must_use]
pub fn sparkline_points(values: &[f64], width: f64, height: f64) -> String {
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return String::new();
    }
    let slice = if finite.len() > SPARK_SLOTS {
        &finite[finite.len() - SPARK_SLOTS..]
    } else {
        &finite[..]
    };
    let pad = 1.5;
    let inner_h = (height - pad * 2.0).max(1.0);
    let step = width / ((SPARK_SLOTS - 1) as f64);
    let offset = SPARK_SLOTS - slice.len();

    let min = slice.iter().copied().fold(f64::INFINITY, f64::min);
    let max = slice.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mut span = (max - min).max(max.abs() * 0.1);
    if !span.is_finite() || span <= 0.0 {
        span = 1.0;
    }

    let y_of = |v: f64| -> f64 {
        if max <= 0.0 && min >= 0.0 {
            height - pad
        } else {
            pad + inner_h * (1.0 - ((v - min) / span).clamp(0.0, 1.0))
        }
    };

    if slice.len() == 1 {
        let y = format!("{:.1}", y_of(slice[0]));
        let x1 = format!("{:.1}", ((offset as f64) - 1.0).max(0.0) * step);
        let x2 = format!("{:.1}", (offset as f64) * step);
        return format!("{x1},{y} {x2},{y}");
    }

    slice
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = ((offset + i) as f64) * step;
            let y = y_of(v);
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[must_use]
pub fn sparkline_lead(points: &str) -> Option<(f64, f64)> {
    let last = points.split_whitespace().last()?;
    let (xs, ys) = last.split_once(',')?;
    let x = xs.parse::<f64>().ok()?;
    let y = ys.parse::<f64>().ok()?;
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListeningPortEntry {
    pub port: u16,
    pub workspace_key: Option<String>,
    pub process_name: String,
    pub pid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortsSummary {
    pub workspace_count: usize,
    pub external_count: usize,
    pub label: String,
}

#[must_use]
pub fn ports_summary(ports: &[ListeningPortEntry], lang: SidebarLang) -> PortsSummary {
    let workspace_count = ports.iter().filter(|p| p.workspace_key.is_some()).count();
    let external_count = ports.len().saturating_sub(workspace_count);
    let label = match (workspace_count, external_count, lang) {
        (1, 0, SidebarLang::En) => "1 workspace port".to_string(),
        (n, 0, SidebarLang::En) => format!("{n} workspace ports"),
        (1, e, SidebarLang::En) => format!("1 workspace port · {e} external"),
        (n, e, SidebarLang::En) => format!("{n} workspace ports · {e} external"),
        (1, 0, SidebarLang::Fr) => "1 port d'espace de travail".to_string(),
        (n, 0, SidebarLang::Fr) => format!("{n} ports d'espace de travail"),
        (1, 1, SidebarLang::Fr) => "1 port d'espace de travail · 1 externe".to_string(),
        (1, e, SidebarLang::Fr) => format!("1 port d'espace de travail · {e} externes"),
        (n, 1, SidebarLang::Fr) => format!("{n} ports d'espace de travail · 1 externe"),
        (n, e, SidebarLang::Fr) => format!("{n} ports d'espace de travail · {e} externes"),
    };
    PortsSummary {
        workspace_count,
        external_count,
        label,
    }
}

// ---------------------------------------------------------------------------
// 5. Terminal/Agent Header Priority Badge (`TerminalPane.vue:1680-1708`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneHeaderBadge {
    Asleep,
    Exited,
    Question,
    Approval,
    Limit,
    Unsent,
    Stuck,
    Apply,
    Working,
    Monitoring,
    Unknown,
    Cache,
    Needs,
}

impl PaneHeaderBadge {
    #[must_use]
    pub const fn label(self, lang: SidebarLang) -> &'static str {
        match (self, lang) {
            (Self::Asleep, SidebarLang::En) => "asleep",
            (Self::Asleep, SidebarLang::Fr) => "en veille",
            (Self::Exited, SidebarLang::En) => "exited",
            (Self::Exited, SidebarLang::Fr) => "terminé",
            (Self::Question, SidebarLang::En) => "question?",
            (Self::Question, SidebarLang::Fr) => "question ?",
            (Self::Approval, SidebarLang::En) => "approve?",
            (Self::Approval, SidebarLang::Fr) => "approuver ?",
            (Self::Limit, SidebarLang::En) => "limit",
            (Self::Limit, SidebarLang::Fr) => "limite",
            (Self::Unsent, SidebarLang::En) => "unconfirmed",
            (Self::Unsent, SidebarLang::Fr) => "non confirmé",
            (Self::Stuck, SidebarLang::En) => "quiet",
            (Self::Stuck, SidebarLang::Fr) => "silencieux",
            (Self::Apply, SidebarLang::En) => "Restart to apply",
            (Self::Apply, SidebarLang::Fr) => "Redémarrer pour appliquer",
            (Self::Working, SidebarLang::En) => "working",
            (Self::Working, SidebarLang::Fr) => "au travail",
            (Self::Monitoring, SidebarLang::En) => "monitoring",
            (Self::Monitoring, SidebarLang::Fr) => "surveille",
            (Self::Unknown, SidebarLang::En) => "unknown",
            (Self::Unknown, SidebarLang::Fr) => "inconnu",
            (Self::Cache, SidebarLang::En) => "cache",
            (Self::Cache, SidebarLang::Fr) => "cache",
            (Self::Needs, SidebarLang::En) => "needs you",
            (Self::Needs, SidebarLang::Fr) => "vous attend",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PaneBadgeInput {
    pub sleeping: bool,
    pub exited: bool,
    pub guest_runs: bool,
    pub guest_sleeping: bool,
    pub guest_state: Option<String>,
    pub is_agent: bool,
    pub asks_approval: bool,
    pub asks_question_input: bool,
    pub limit_reached: bool,
    pub unsent_message: bool,
    pub stuck_quiet: bool,
    pub launch_stale: bool,
    pub agent_busy: bool,
    pub background_running: bool,
    pub sub_running: bool,
    pub agent_unknown: bool,
    pub cache_countdown_active: bool,
    pub needs_you: bool,
}

#[must_use]
pub fn pane_header_badge(input: &PaneBadgeInput) -> Option<PaneHeaderBadge> {
    if input.sleeping {
        return Some(PaneHeaderBadge::Asleep);
    }
    if input.exited {
        return Some(PaneHeaderBadge::Exited);
    }
    if input.guest_runs {
        if input.guest_sleeping {
            return Some(PaneHeaderBadge::Asleep);
        }
        return match input.guest_state.as_deref().unwrap_or("") {
            "approval" => Some(PaneHeaderBadge::Approval),
            "limited" => Some(PaneHeaderBadge::Limit),
            "working" => Some(PaneHeaderBadge::Working),
            "monitoring" => Some(PaneHeaderBadge::Monitoring),
            "unknown" => Some(PaneHeaderBadge::Unknown),
            "waiting" => Some(PaneHeaderBadge::Needs),
            _ => None,
        };
    }
    if !input.is_agent {
        return None;
    }
    if input.asks_approval {
        return Some(if input.asks_question_input {
            PaneHeaderBadge::Question
        } else {
            PaneHeaderBadge::Approval
        });
    }
    if input.limit_reached {
        return Some(PaneHeaderBadge::Limit);
    }
    if input.unsent_message {
        return Some(PaneHeaderBadge::Unsent);
    }
    if input.stuck_quiet {
        return Some(PaneHeaderBadge::Stuck);
    }
    if input.launch_stale {
        return Some(PaneHeaderBadge::Apply);
    }
    if input.agent_busy {
        return Some(PaneHeaderBadge::Working);
    }
    if input.background_running {
        return Some(PaneHeaderBadge::Monitoring);
    }
    if input.sub_running {
        return Some(PaneHeaderBadge::Working);
    }
    if input.agent_unknown {
        return Some(PaneHeaderBadge::Unknown);
    }
    if input.cache_countdown_active {
        return Some(PaneHeaderBadge::Cache);
    }
    if input.needs_you {
        return Some(PaneHeaderBadge::Needs);
    }
    None
}
