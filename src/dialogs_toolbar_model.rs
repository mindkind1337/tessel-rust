//! UI-agnostic models and bounded helpers for:
//! - `AddProjectDialog` (`AddProjectDialog.vue` + nested Git repo BFS + safe `git clone` / `git init` + SSH `RemoteFolderBrowser`)
//! - `GitHubDialog` (`GitHubDialog.vue` — `gh` Issues, PRs, CI check rollups & `Fix failing checks` prompt)
//! - Toolbar `UsageMenu` (`UsageMenu.vue` — provider token/cost bars & `<60%` / `>=60%` / `>=80%` threshold levels)
//! - Toolbar `NotificationsMenu` (`NotificationsMenu.vue` — coalesced notification center & unread counter)

use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::side_panels_model::TaskHistoryRecord;

pub const MAX_NESTED_GIT_DEPTH: usize = 3;
pub const MAX_DISCOVERED_REPOS: usize = 64;
pub const MAX_NOTIFICATIONS: usize = 100;

const SKIP_SCAN_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    "vendor",
    ".venv",
    "venv",
];

#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ============================================================================
// 1. AddProjectDialog: Nested Git Repo Discovery, Safe Clone, Init & SSH Browse
// ============================================================================

/// BFS discovery of nested Git repositories (`depth <= max_depth`) inside a parent folder.
#[must_use]
pub fn discover_nested_git_repos(root: &Path, max_depth: usize, max_repos: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if !root.is_dir() || max_repos == 0 {
        return found;
    }
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    queue.push_back((root.to_path_buf(), 0));

    while let Some((dir, depth)) = queue.pop_front() {
        if found.len() >= max_repos {
            break;
        }
        if depth > 0 && dir.join(".git").exists() {
            found.push(dir);
            continue;
        }
        if depth >= max_depth {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut subdirs: Vec<PathBuf> = entries
            .flatten()
            .filter_map(|e| {
                let ft = e.file_type().ok()?;
                if ft.is_symlink() || !ft.is_dir() {
                    return None;
                }
                let name = e.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with('.') || SKIP_SCAN_DIRS.contains(&name_str.as_ref()) {
                    return None;
                }
                Some(e.path())
            })
            .collect();
        subdirs.sort();
        for sub in subdirs {
            queue.push_back((sub, depth + 1));
        }
    }
    found
}

/// Validates a Git clone URL against option injection (`-upload-pack`) and unsafe transports (`ext::`, `fd::`).
pub fn validate_safe_clone_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    if url.is_empty() {
        return Err("Repository URL is required.".into());
    }
    if url.starts_with('-') {
        return Err("Repository URL cannot start with '-'.".into());
    }
    if url.chars().any(char::is_control) {
        return Err("Repository URL contains control characters.".into());
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("ext::") || lower.starts_with("fd::") {
        return Err("Unsafe Git transport helper is not permitted.".into());
    }
    let allowed = lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("ssh://")
        || lower.starts_with("git@")
        || Path::new(url).exists();
    if !allowed {
        return Err(
            "Use an https://, ssh://, git@host:owner/repo URL, or an existing local repository path."
                .into(),
        );
    }
    Ok(url.to_owned())
}

#[must_use]
pub fn default_repo_folder_name(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches(['/', '\\']);
    let tail = trimmed
        .rsplit(['/', ':', '\\'])
        .next()
        .unwrap_or("repo")
        .trim_end_matches(".git");
    let clean: String = tail
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    if clean.is_empty() || clean == "." || clean == ".." {
        "repository".into()
    } else {
        clean
    }
}

pub fn validate_new_folder_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() || name == "." || name == ".." {
        return Err("Enter a valid folder name.".into());
    }
    if name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        || name.chars().any(char::is_control)
    {
        return Err("Folder name contains unsupported path characters.".into());
    }
    Ok(name.to_owned())
}

pub fn clone_git_repository(
    raw_url: &str,
    parent_dir: &Path,
    folder_name: &str,
) -> Result<PathBuf, String> {
    let url = validate_safe_clone_url(raw_url)?;
    let clean_name = validate_new_folder_name(folder_name)?;
    if !parent_dir.is_dir() {
        return Err("Destination parent folder does not exist.".into());
    }
    let dest = parent_dir.join(&clean_name);
    if dest.exists() {
        return Err(format!("Destination `{}` already exists.", dest.display()));
    }
    let mut cmd = Command::new("git");
    crate::agent_env::clean_command(&mut cmd);
    cmd.args([
        "-c",
        "protocol.ext.allow=never",
        "-c",
        "protocol.fd.allow=never",
        "-c",
        "core.longpaths=true",
        "clone",
        "--",
        &url,
    ])
    .arg(&dest)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("Failed to start git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(dest)
}

pub fn init_new_git_project(parent_dir: &Path, folder_name: &str) -> Result<PathBuf, String> {
    let clean_name = validate_new_folder_name(folder_name)?;
    if !parent_dir.is_dir() {
        return Err("Parent folder does not exist.".into());
    }
    let dest = parent_dir.join(&clean_name);
    if dest.exists() {
        return Err(format!("Folder `{}` already exists.", dest.display()));
    }
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;

    let run_init = |args: &[&str]| -> Result<(), String> {
        let mut cmd = Command::new("git");
        crate::agent_env::clean_command(&mut cmd);
        cmd.current_dir(&dest)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let out = cmd.output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
        }
    };

    if run_init(&["init", "-b", "main"]).is_err() {
        run_init(&["init"])?;
    }
    Ok(dest)
}

/// Lists subdirectories on a configured SSH host (`RemoteFolderBrowser` parity).
pub fn list_remote_directories(host_alias: &str, remote_dir: &str) -> Result<Vec<String>, String> {
    let alias = host_alias.trim();
    if alias.is_empty() || alias.starts_with('-') || alias.chars().any(char::is_whitespace) {
        return Err("Invalid SSH host alias.".into());
    }
    let dir = if remote_dir.trim().is_empty() {
        "."
    } else {
        remote_dir.trim()
    };
    if dir.chars().any(char::is_control) {
        return Err("Invalid remote path.".into());
    }
    let quoted_dir = format!("'{}'", dir.replace('\'', "'\\''"));
    let mut cmd = Command::new("ssh");
    crate::agent_env::clean_command(&mut cmd);
    cmd.args([
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=8",
        "--",
        alias,
        &format!("ls -1Ap -- {quoted_dir}"),
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("Failed to start ssh: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut folders: Vec<String> = stdout
        .lines()
        .filter_map(|l| l.trim().strip_suffix('/'))
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .take(200)
        .map(str::to_owned)
        .collect();
    folders.sort();
    Ok(folders)
}

// ============================================================================
// 2. GitHubDialog (`gh` CLI Issues, Pull Requests & CI Check Rollup)
// ============================================================================

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GhCheckRollup {
    #[default]
    None,
    Passing,
    Pending,
    Failing,
}

impl GhCheckRollup {
    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::None, false) => "No checks",
            (Self::None, true) => "Aucun check",
            (Self::Passing, false) => "Checks passing",
            (Self::Passing, true) => "Checks réussis",
            (Self::Pending, false) => "Checks pending",
            (Self::Pending, true) => "Checks en cours",
            (Self::Failing, false) => "Checks failing",
            (Self::Failing, true) => "Checks en échec",
        }
    }

    #[must_use]
    pub const fn rgb(self) -> (u8, u8, u8) {
        match self {
            Self::None => (0x8b, 0x93, 0xa1),
            Self::Passing => (0x3d, 0xdc, 0x84),
            Self::Pending => (0xf5, 0xa5, 0x24),
            Self::Failing => (0xf8, 0x71, 0x71),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GhIssue {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub author: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GhCheckItem {
    pub name: String,
    pub state: String,
    pub link: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GhPullRequest {
    pub number: u64,
    pub title: String,
    pub head_ref: String,
    pub base_ref: String,
    pub state: String,
    pub is_draft: bool,
    pub rollup: GhCheckRollup,
    pub failing_checks: Vec<GhCheckItem>,
    pub url: String,
}

fn run_gh_json(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("gh");
    crate::agent_env::clean_command(&mut cmd);
    cmd.current_dir(cwd)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("GitHub CLI (`gh`) not available: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[must_use]
pub fn parse_gh_issues_json(raw: &str) -> Vec<GhIssue> {
    let Ok(serde_json::Value::Array(arr)) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(number) = item.get("number").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        let title = item
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned();
        let state = item
            .get("state")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("OPEN")
            .to_owned();
        let author = item
            .get("author")
            .and_then(|a| a.get("login"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned();
        let url = item
            .get("url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned();
        out.push(GhIssue {
            number,
            title,
            state,
            author,
            url,
        });
    }
    out
}

#[must_use]
pub fn parse_gh_prs_json(raw: &str) -> Vec<GhPullRequest> {
    let Ok(serde_json::Value::Array(arr)) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(number) = item.get("number").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        let title = item
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned();
        let head_ref = item
            .get("headRefName")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned();
        let base_ref = item
            .get("baseRefName")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("main")
            .to_owned();
        let state = item
            .get("state")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("OPEN")
            .to_owned();
        let is_draft = item
            .get("isDraft")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let url = item
            .get("url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned();

        let mut rollup = GhCheckRollup::None;
        let mut failing_checks = Vec::new();
        if let Some(serde_json::Value::Array(checks)) = item.get("statusCheckRollup")
            && !checks.is_empty()
        {
            let mut any_fail = false;
            let mut any_pending = false;
            for c in checks {
                let name = c
                    .get("name")
                    .or_else(|| c.get("context"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("check")
                    .to_owned();
                let conclusion = c
                    .get("conclusion")
                    .or_else(|| c.get("state"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_ascii_uppercase();
                let link = c
                    .get("detailsUrl")
                    .or_else(|| c.get("targetUrl"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                if matches!(
                    conclusion.as_str(),
                    "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED"
                ) {
                    any_fail = true;
                    failing_checks.push(GhCheckItem {
                        name,
                        state: conclusion,
                        link,
                    });
                } else if matches!(
                    conclusion.as_str(),
                    "" | "PENDING" | "IN_PROGRESS" | "QUEUED"
                ) {
                    any_pending = true;
                }
            }
            rollup = if any_fail {
                GhCheckRollup::Failing
            } else if any_pending {
                GhCheckRollup::Pending
            } else {
                GhCheckRollup::Passing
            };
        }

        out.push(GhPullRequest {
            number,
            title,
            head_ref,
            base_ref,
            state,
            is_draft,
            rollup,
            failing_checks,
            url,
        });
    }
    out
}

pub fn load_gh_issues(cwd: &Path, limit: usize) -> Result<Vec<GhIssue>, String> {
    let lim = limit.clamp(1, 100).to_string();
    let json = run_gh_json(
        cwd,
        &[
            "issue",
            "list",
            "--limit",
            &lim,
            "--json",
            "number,title,state,author,url",
        ],
    )?;
    Ok(parse_gh_issues_json(&json))
}

pub fn load_gh_prs(cwd: &Path, limit: usize) -> Result<Vec<GhPullRequest>, String> {
    let lim = limit.clamp(1, 100).to_string();
    let json = run_gh_json(
        cwd,
        &[
            "pr",
            "list",
            "--limit",
            &lim,
            "--json",
            "number,title,headRefName,baseRefName,state,isDraft,statusCheckRollup,url",
        ],
    )?;
    Ok(parse_gh_prs_json(&json))
}

#[must_use]
pub fn format_fix_failing_checks_prompt(pr: &GhPullRequest) -> String {
    let mut out = format!(
        "Please investigate and fix the failing CI checks on PR #{} (`{}` — `{}`):\n\n```text\n",
        pr.number, pr.head_ref, pr.title
    );
    for c in &pr.failing_checks {
        if c.link.is_empty() {
            out.push_str(&format!("- {} ({})\n", c.name, c.state));
        } else {
            out.push_str(&format!("- {} ({}) — {}\n", c.name, c.state, c.link));
        }
    }
    out.push_str("```\n");
    out
}

// ============================================================================
// 3. Toolbar UsageMenu (`<60%` dim, `>=60%` warn, `>=80%` danger)
// ============================================================================

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UsageAlertLevel {
    #[default]
    Normal,
    Warn,
    Danger,
}

impl UsageAlertLevel {
    #[must_use]
    pub fn from_ratio(ratio: f32) -> Self {
        if ratio >= 0.80_f32 {
            Self::Danger
        } else if ratio >= 0.60_f32 {
            Self::Warn
        } else {
            Self::Normal
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderUsageBar {
    pub provider: String,
    pub tokens_today: u64,
    pub tokens_30d: u64,
    pub cost_today_usd: f64,
    pub cost_30d_usd: f64,
    pub ratio: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageSummary {
    pub today_cost_usd: f64,
    pub month_cost_usd: f64,
    pub daily_budget_usd: f64,
    pub overall_ratio: f32,
    pub level: UsageAlertLevel,
    pub providers: Vec<ProviderUsageBar>,
}

#[must_use]
pub fn compute_usage_summary(
    records: &[TaskHistoryRecord],
    daily_budget_usd: f64,
    now_unix_ms: u64,
) -> UsageSummary {
    let day_cutoff = now_unix_ms.saturating_sub(24 * 3_600 * 1_000);
    let month_cutoff = now_unix_ms.saturating_sub(30 * 24 * 3_600 * 1_000);
    let budget = if daily_budget_usd > 0.0 {
        daily_budget_usd
    } else {
        10.0
    };

    let mut by_provider: BTreeMap<String, (u64, u64, f64, f64)> = BTreeMap::new();
    let mut today_cost_usd = 0.0;
    let mut month_cost_usd = 0.0;

    for rec in records {
        if rec.done_at_ms < month_cutoff {
            continue;
        }
        let prov = if rec.agent_kind.trim().is_empty() {
            "other".to_owned()
        } else {
            rec.agent_kind.clone()
        };
        let entry = by_provider.entry(prov).or_insert((0, 0, 0.0, 0.0));
        entry.1 += rec.tokens;
        entry.3 += rec.cost_usd;
        month_cost_usd += rec.cost_usd;
        if rec.done_at_ms >= day_cutoff {
            entry.0 += rec.tokens;
            entry.2 += rec.cost_usd;
            today_cost_usd += rec.cost_usd;
        }
    }

    let overall_ratio = ((today_cost_usd / budget) as f32).clamp(0.0_f32, 1.0_f32);
    let providers = by_provider
        .into_iter()
        .map(
            |(provider, (tok_d, tok_m, cost_d, cost_m))| ProviderUsageBar {
                provider,
                tokens_today: tok_d,
                tokens_30d: tok_m,
                cost_today_usd: cost_d,
                cost_30d_usd: cost_m,
                ratio: ((cost_d / budget) as f32).clamp(0.0_f32, 1.0_f32),
            },
        )
        .collect();

    UsageSummary {
        today_cost_usd,
        month_cost_usd,
        daily_budget_usd: budget,
        overall_ratio,
        level: UsageAlertLevel::from_ratio(overall_ratio),
        providers,
    }
}

// ============================================================================
// 4. Toolbar NotificationsMenu (Coalesced Notification Center)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    ApprovalNeeded,
    TaskCompleted,
    AutomationRan,
    SystemNotice,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationItem {
    pub id: u64,
    pub kind: NotificationKind,
    pub title: String,
    pub detail: String,
    pub pane_id: Option<u64>,
    pub timestamp_ms: u64,
    pub unread: bool,
    pub count: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationCenter {
    pub next_id: u64,
    pub items: Vec<NotificationItem>,
}

impl NotificationCenter {
    #[must_use]
    pub fn unread_count(&self) -> usize {
        self.items.iter().filter(|n| n.unread).count()
    }

    pub fn push_coalesced(
        &mut self,
        kind: NotificationKind,
        title: impl Into<String>,
        detail: impl Into<String>,
        pane_id: Option<u64>,
    ) {
        let title = title.into();
        let detail = detail.into();
        let ts = now_ms();
        if let Some(last) = self.items.last_mut()
            && last.kind == kind
            && last.pane_id == pane_id
            && last.title == title
        {
            last.detail = detail;
            last.timestamp_ms = ts;
            last.unread = true;
            last.count = last.count.saturating_add(1);
            return;
        }
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.items.push(NotificationItem {
            id: self.next_id,
            kind,
            title,
            detail,
            pane_id,
            timestamp_ms: ts,
            unread: true,
            count: 1,
        });
        if self.items.len() > MAX_NOTIFICATIONS {
            let drop_n = self.items.len() - MAX_NOTIFICATIONS;
            self.items.drain(0..drop_n);
        }
    }

    pub fn mark_all_read(&mut self) {
        for item in &mut self.items {
            item.unread = false;
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}
