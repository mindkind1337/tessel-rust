//! Bounded Git worktree discovery, creation, `.env*` secret copy, Windows NTFS junction-safe
//! removal, and cleanup classification (`Safe`, `Risk`, `Blocker`).

use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    thread,
    time::Duration,
};

use crate::{
    git,
    sidebar_model::{DiscoveredWorktree, norm_sidebar_path, same_path},
    workspace::{BranchPrefixMode, GeneralPreferences, GitPreferences},
};

const MAX_DISCOVERED_WORKTREES: usize = 200;
const MAX_ENV_FILE_BYTES: u64 = 1_048_576;
const MAX_ENV_TOTAL_BYTES: u64 = 4_194_304;
const MAX_ENV_WALK_DEPTH: usize = 5;
const MAX_ENV_VISITED: usize = 5_000;
const MAX_UNLINK_DEPTH: usize = 32;
const MAX_UNLINK_VISITED: usize = 50_000;

const ENV_SKIP_DIRS: &[&str] = &[
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

#[cfg(windows)]
const WIN_FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0010;
#[cfg(windows)]
const WIN_FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitRepoSummary {
    pub toplevel: PathBuf,
    pub branch: String,
    pub default_branch: String,
    pub has_commits: bool,
    pub git_username: Option<String>,
    pub worktrees: Vec<DiscoveredWorktree>,
}

impl GitRepoSummary {
    /// Returns `(eligible, disabled_reason)` for launching a pane inside a separate worktree.
    #[must_use]
    pub fn launch_availability(&self, is_en: bool) -> (bool, Option<String>) {
        if !self.has_commits {
            let msg = if is_en {
                "Make an initial commit before creating a worktree."
            } else {
                "Créez un premier commit avant d'ouvrir un worktree."
            };
            return (false, Some(msg.to_owned()));
        }
        (true, None)
    }

    #[must_use]
    pub fn non_primary_count(&self) -> usize {
        self.worktrees.iter().filter(|w| !w.is_primary).count()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatedWorktree {
    pub root: PathBuf,
    pub path: PathBuf,
    pub branch: String,
    pub copied_env_files: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupBlocker {
    Detached,
    DefaultBranch,
    Locked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupRisk {
    Dirty(usize),
    Unmerged(usize),
    OpenPanes(usize),
    MissingFolder,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupItem {
    pub path: PathBuf,
    pub branch: String,
    pub head: String,
    pub last_subject: String,
    pub ahead: usize,
    pub behind: usize,
    pub merged: bool,
    pub blocker: Option<CleanupBlocker>,
    pub risks: Vec<CleanupRisk>,
}

impl CleanupItem {
    #[must_use]
    pub fn is_safe(&self) -> bool {
        self.blocker.is_none() && self.risks.is_empty()
    }

    #[must_use]
    pub fn is_selectable(&self) -> bool {
        self.blocker.is_none()
    }
}

fn run_git_readonly(cwd: &Path, args: &[&str]) -> Result<String, String> {
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
pub fn normalize_git_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    #[cfg(windows)]
    {
        let mut s = trimmed.replace('/', "\\");
        while s.len() > 3 && s.ends_with('\\') {
            s.pop();
        }
        PathBuf::from(s)
    }
    #[cfg(not(windows))]
    {
        let mut s = trimmed.to_owned();
        while s.len() > 1 && s.ends_with('/') {
            s.pop();
        }
        PathBuf::from(s)
    }
}

#[must_use]
pub fn parse_porcelain_worktrees(text: &str, max: usize) -> Vec<DiscoveredWorktree> {
    let normalized = text.replace("\r\n", "\n");
    let mut out = Vec::new();
    let mut seen_non_bare = false;

    for raw_block in normalized.split("\n\n") {
        if out.len() >= max {
            break;
        }
        let block = raw_block.trim();
        if block.is_empty() {
            continue;
        }
        let mut path = String::new();
        let mut head = String::new();
        let mut branch = String::new();
        let mut locked = false;
        let mut detached = false;
        let mut bare = false;

        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("worktree ") {
                path = normalize_git_path(rest).to_string_lossy().into_owned();
            } else if let Some(rest) = line.strip_prefix("HEAD ") {
                head = rest.trim().to_owned();
            } else if let Some(rest) = line.strip_prefix("branch ") {
                let b = rest.trim();
                branch = b.strip_prefix("refs/heads/").unwrap_or(b).to_owned();
            } else if line == "detached" {
                detached = true;
            } else if line == "locked" || line.starts_with("locked ") {
                locked = true;
            } else if line == "bare" {
                bare = true;
            }
        }

        if bare || path.is_empty() {
            continue;
        }
        if branch.is_empty() {
            detached = true;
        }
        let is_primary = !seen_non_bare;
        seen_non_bare = true;
        out.push(DiscoveredWorktree {
            path,
            branch,
            head,
            is_primary,
            locked,
            detached,
        });
    }

    out
}

#[must_use]
pub fn slugify_worktree_name(label: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in label.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    let cleaned = out.trim_matches('-');
    if cleaned.is_empty() {
        "agent".to_owned()
    } else {
        cleaned.to_owned()
    }
}

fn validate_prefix_segment(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() || trimmed.chars().count() > 60 {
        return None;
    }
    if trimmed.starts_with('-')
        || trimmed.starts_with('.')
        || trimmed.ends_with('.')
        || trimmed.contains("..")
        || trimmed.contains("//")
        || trimmed.contains("@{")
    {
        return None;
    }
    for part in trimmed.split('/') {
        if part.is_empty()
            || part.starts_with('.')
            || part.ends_with('.')
            || part.ends_with(".lock")
        {
            return None;
        }
    }
    if trimmed.chars().any(|c| {
        c.is_control()
            || c.is_whitespace()
            || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\' | '\x7f')
    }) {
        return None;
    }
    Some(trimmed.to_owned())
}

fn sanitize_username_prefix(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut prev_sep = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_sep = false;
        } else if matches!(ch, '-' | '_' | ' ' | '.') && !out.is_empty() && !prev_sep {
            out.push('-');
            prev_sep = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    validate_prefix_segment(out.trim_matches('-'))
}

#[must_use]
pub fn format_worktree_branch(
    mode: BranchPrefixMode,
    custom: &str,
    git_username: Option<&str>,
    slug: &str,
) -> String {
    let prefix = match mode {
        BranchPrefixMode::None => None,
        BranchPrefixMode::Custom => {
            validate_prefix_segment(custom).or_else(|| Some("agent".to_owned()))
        }
        BranchPrefixMode::GitUsername => git_username
            .and_then(sanitize_username_prefix)
            .or_else(|| Some("agent".to_owned())),
    };
    match prefix {
        Some(p) => format!("{p}/{slug}"),
        None => slug.to_owned(),
    }
}

fn is_meta_reparse_or_symlink(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if (meta.file_attributes() & WIN_FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return true;
        }
    }
    false
}

#[cfg_attr(windows, allow(unused_variables))]
fn is_meta_directory_link(path: &Path, meta: &fs::Metadata) -> bool {
    if !is_meta_reparse_or_symlink(meta) {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        (meta.file_attributes() & WIN_FILE_ATTRIBUTE_DIRECTORY) != 0
    }
    #[cfg(not(windows))]
    {
        fs::metadata(path).is_ok_and(|target| target.is_dir())
    }
}

pub fn is_reparse_or_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| is_meta_reparse_or_symlink(&meta))
}

fn repo_folder_slug(toplevel: &Path) -> String {
    let raw = toplevel
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");
    let stripped = raw.strip_suffix(".git").unwrap_or(raw).trim();
    let safe: String = stripped
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        .take(80)
        .collect();
    if safe.is_empty() || safe == "." || safe == ".." {
        "project".to_owned()
    } else {
        safe
    }
}

fn expand_home_prefix(trimmed: &str) -> Result<Option<PathBuf>, String> {
    if trimmed == "~" {
        let base = directories::BaseDirs::new().ok_or("Could not resolve home directory.")?;
        return Ok(Some(base.home_dir().to_path_buf()));
    }
    if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        let rel = Path::new(rest);
        if rel.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err("Worktrees directory override cannot contain '..'.".into());
        }
        let base = directories::BaseDirs::new().ok_or("Could not resolve home directory.")?;
        return Ok(Some(base.home_dir().join(rel)));
    }
    Ok(None)
}

/// Resolves the base directory under which worktree copies for `toplevel` are created.
pub fn worktree_base_dir(toplevel: &Path, workspace_dir: &str) -> Result<PathBuf, String> {
    if !toplevel.is_absolute() {
        return Err("Repository root must be an absolute path.".into());
    }
    let repo_name = repo_folder_slug(toplevel);
    let trimmed = workspace_dir.trim();

    let base_dir = if trimmed.is_empty() {
        let parent = toplevel
            .parent()
            .ok_or("Repository root has no parent directory.")?;
        parent.join(format!("{repo_name}.worktrees"))
    } else if let Some(expanded) = expand_home_prefix(trimmed)? {
        expanded.join(repo_name)
    } else {
        let candidate = Path::new(trimmed);
        if candidate
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            return Err("Worktrees directory override cannot contain '..'.".into());
        }
        if candidate.is_absolute() {
            candidate.join(repo_name)
        } else {
            toplevel.join(candidate).join(repo_name)
        }
    };

    if base_dir.exists() && is_reparse_or_symlink(&base_dir) {
        return Err("Worktrees directory must not be a symbolic link or junction.".into());
    }
    Ok(base_dir)
}

fn detect_default_branch(
    toplevel: &Path,
    current_branch: &str,
    worktrees: &[DiscoveredWorktree],
) -> String {
    if let Ok(origin_head) = run_git_readonly(
        toplevel,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    ) {
        let trimmed = origin_head.trim();
        let short = trimmed.strip_prefix("origin/").unwrap_or(trimmed);
        if !short.is_empty()
            && run_git_readonly(
                toplevel,
                &[
                    "show-ref",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{short}"),
                ],
            )
            .is_ok()
        {
            return short.to_owned();
        }
    }
    for candidate in ["main", "master"] {
        if run_git_readonly(
            toplevel,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{candidate}"),
            ],
        )
        .is_ok()
        {
            return candidate.to_owned();
        }
    }
    if let Some(primary) = worktrees
        .iter()
        .find(|w| w.is_primary && !w.branch.is_empty())
    {
        return primary.branch.clone();
    }
    if !current_branch.is_empty() {
        return current_branch.to_owned();
    }
    "main".to_owned()
}

/// Inspects a project folder with read-only Git commands (never acquires index locks).
pub fn inspect_project(cwd: &Path) -> Result<GitRepoSummary, String> {
    if !cwd.is_dir() {
        return Err("Project folder does not exist.".into());
    }
    let toplevel_raw = run_git_readonly(cwd, &["rev-parse", "--show-toplevel"])?;
    let toplevel = normalize_git_path(&toplevel_raw);
    if !toplevel.is_dir() {
        return Err("Git repository root is unavailable.".into());
    }

    let has_commits =
        run_git_readonly(&toplevel, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok();
    let branch = run_git_readonly(&toplevel, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map(|s| s.trim().to_owned())
        .unwrap_or_default();

    let git_username = run_git_readonly(&toplevel, &["config", "--get", "user.name"])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    let porcelain = run_git_readonly(&toplevel, &["worktree", "list", "--porcelain"])?;
    let worktrees = parse_porcelain_worktrees(&porcelain, MAX_DISCOVERED_WORKTREES);
    let default_branch = detect_default_branch(&toplevel, &branch, &worktrees);

    Ok(GitRepoSummary {
        toplevel,
        branch,
        default_branch,
        has_commits,
        git_username,
        worktrees,
    })
}

fn is_env_secret_filename(name: &str) -> bool {
    if name == ".env" {
        return true;
    }
    let Some(suffix) = name.strip_prefix(".env.") else {
        return false;
    };
    if suffix.is_empty() {
        return false;
    }
    let lower = suffix.to_ascii_lowercase();
    const TEMPLATES: &[&str] = &["example", "sample", "template", "dist", "schema"];
    if TEMPLATES.contains(&lower.as_str()) {
        return false;
    }
    !TEMPLATES
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")))
}

fn ensure_safe_rel_parent(target_root: &Path, rel: &Path) -> Result<PathBuf, String> {
    let mut cur = target_root.to_path_buf();
    if let Some(parent) = rel.parent() {
        for comp in parent.components() {
            let Component::Normal(part) = comp else {
                return Err("Unsafe relative path component.".into());
            };
            cur.push(part);
            if cur.exists() {
                if is_reparse_or_symlink(&cur) {
                    return Err("Refusing to traverse symbolic link inside target worktree.".into());
                }
            } else {
                fs::create_dir(&cur).map_err(|e| e.to_string())?;
            }
        }
    }
    let file_name = rel.file_name().ok_or("Missing target file name.")?;
    Ok(cur.join(file_name))
}

fn copy_ignored_env_files(source_root: &Path, target_root: &Path) -> usize {
    if is_reparse_or_symlink(source_root) || is_reparse_or_symlink(target_root) {
        return 0;
    }
    let mut stack = vec![(source_root.to_path_buf(), 0usize)];
    let mut visited = 0usize;
    let mut total_bytes = 0u64;
    let mut copied = 0usize;

    while let Some((dir, depth)) = stack.pop() {
        if is_reparse_or_symlink(&dir) {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if visited >= MAX_ENV_VISITED {
                return copied;
            }
            visited += 1;

            let full = entry.path();
            let Ok(meta) = fs::symlink_metadata(&full) else {
                continue;
            };
            if is_meta_reparse_or_symlink(&meta) {
                continue;
            }
            let Some(name) = full.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if meta.is_dir() {
                if depth < MAX_ENV_WALK_DEPTH && !ENV_SKIP_DIRS.contains(&name) {
                    stack.push((full, depth + 1));
                }
                continue;
            }
            if !meta.is_file() || !is_env_secret_filename(name) {
                continue;
            }
            let len = meta.len();
            if len > MAX_ENV_FILE_BYTES || total_bytes.saturating_add(len) > MAX_ENV_TOTAL_BYTES {
                continue;
            }
            let Ok(rel) = full.strip_prefix(source_root) else {
                continue;
            };
            let rel_slash = rel.to_string_lossy().replace('\\', "/");
            if git::run(source_root, &["check-ignore", "-q", "--", &rel_slash], None).is_err() {
                continue;
            }
            let Ok(bytes) = fs::read(&full) else {
                continue;
            };
            let Ok(dest) = ensure_safe_rel_parent(target_root, rel) else {
                continue;
            };
            if dest.exists() {
                continue;
            }
            if let Ok(mut file) = OpenOptions::new().write(true).create_new(true).open(&dest)
                && file.write_all(&bytes).is_ok()
            {
                total_bytes = total_bytes.saturating_add(bytes.len() as u64);
                copied += 1;
            }
        }
    }

    copied
}

fn resolve_base_commit(toplevel: &Path, base_branch: Option<&str>) -> Result<String, String> {
    let Some(raw) = base_branch.map(str::trim).filter(|s| !s.is_empty()) else {
        let sha = run_git_readonly(toplevel, &["rev-parse", "--verify", "HEAD^{commit}"])?;
        return Ok(sha.trim().to_owned());
    };
    if raw.starts_with('-') || raw.contains("..") || raw.chars().any(char::is_control) {
        return Err("Invalid base branch name.".into());
    }
    for candidate in [
        format!("refs/heads/{raw}"),
        format!("refs/remotes/{raw}"),
        raw.to_owned(),
    ] {
        if let Ok(sha) = run_git_readonly(
            toplevel,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                "--end-of-options",
                &format!("{candidate}^{{commit}}"),
            ],
        ) {
            return Ok(sha.trim().to_owned());
        }
    }
    Err(format!("Base branch '{raw}' could not be resolved."))
}

/// Creates a new Git worktree on a fresh branch honoring `general_prefs.workspace_dir`,
/// `git_prefs` branch prefix settings, and optional `.env*` secret copying.
pub fn create_worktree(
    cwd: &Path,
    label: &str,
    base_branch: Option<&str>,
    copy_env: bool,
    general_prefs: &GeneralPreferences,
    git_prefs: &GitPreferences,
    git_username_override: Option<&str>,
) -> Result<CreatedWorktree, String> {
    let summary = inspect_project(cwd)?;
    if !summary.has_commits {
        return Err("Make an initial commit before creating a worktree.".into());
    }
    let base_commit = resolve_base_commit(&summary.toplevel, base_branch)?;
    let base_dir = worktree_base_dir(&summary.toplevel, &general_prefs.workspace_dir)?;
    fs::create_dir_all(&base_dir).map_err(|e| {
        format!(
            "Could not create worktrees directory {}: {e}",
            base_dir.display()
        )
    })?;
    if is_reparse_or_symlink(&base_dir) {
        return Err("Worktrees directory must not be a symbolic link or junction.".into());
    }

    let base_slug = slugify_worktree_name(label);
    let effective_user = git_username_override.or(summary.git_username.as_deref());

    let mut chosen: Option<(String, PathBuf)> = None;
    for attempt in 1..=50u32 {
        let slug = if attempt == 1 {
            base_slug.clone()
        } else {
            format!("{base_slug}-{attempt}")
        };
        let branch = format_worktree_branch(
            git_prefs.branch_prefix_mode,
            &git_prefs.branch_prefix_custom,
            effective_user,
            &slug,
        );
        let wt_path = base_dir.join(&slug);
        if wt_path.exists() || is_reparse_or_symlink(&wt_path) {
            continue;
        }
        let branch_exists = run_git_readonly(
            &summary.toplevel,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )
        .is_ok();
        if !branch_exists {
            chosen = Some((branch, wt_path));
            break;
        }
    }

    let (branch, wt_path) = chosen
        .ok_or("Could not find an unused worktree folder and branch name after 50 attempts.")?;
    let wt_str = wt_path.to_string_lossy();

    git::run(
        &summary.toplevel,
        &[
            "-c",
            "core.longpaths=true",
            "worktree",
            "add",
            "-b",
            &branch,
            &wt_str,
            &base_commit,
        ],
        None,
    )?;

    let copied_env_files = if copy_env {
        copy_ignored_env_files(&summary.toplevel, &wt_path)
    } else {
        0
    };

    Ok(CreatedWorktree {
        root: summary.toplevel,
        path: normalize_git_path(&wt_str),
        branch,
        copied_env_files,
    })
}

/// Walks `root` without descending into symlinks or Windows NTFS reparse points (junctions /
/// directory symlinks) and unlinks any directory link via `fs::remove_dir`, leaving external
/// junction targets completely untouched before `git worktree remove` runs.
pub fn unlink_directory_links(root: &Path) -> Result<usize, String> {
    let root_meta = fs::symlink_metadata(root)
        .map_err(|e| format!("Cannot inspect {}: {e}", root.display()))?;
    if is_meta_reparse_or_symlink(&root_meta) {
        return Err("Refusing to traverse a worktree root that is itself a link.".into());
    }
    if !root_meta.is_dir() {
        return Ok(0);
    }

    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut visited = 0usize;
    let mut unlinked = 0usize;

    while let Some((dir, depth)) = stack.pop() {
        let Ok(dir_meta) = fs::symlink_metadata(&dir) else {
            continue;
        };
        if is_meta_reparse_or_symlink(&dir_meta) {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > MAX_UNLINK_VISITED {
                return Err("Worktree contains too many entries to safely scan links.".into());
            }
            let full = entry.path();
            let Ok(meta) = fs::symlink_metadata(&full) else {
                continue;
            };
            if is_meta_directory_link(&full, &meta) {
                fs::remove_dir(&full).map_err(|e| {
                    format!(
                        "Could not unlink directory junction/symlink {}: {e}",
                        full.display()
                    )
                })?;
                unlinked += 1;
                continue;
            }
            if is_meta_reparse_or_symlink(&meta) {
                continue;
            }
            if meta.is_dir() {
                if depth >= MAX_UNLINK_DEPTH {
                    return Err("Worktree directory nesting exceeds safe unlink depth.".into());
                }
                stack.push((full, depth + 1));
            }
        }
    }

    Ok(unlinked)
}

fn is_protected_path(target: &Path, toplevel: &Path) -> bool {
    if target.parent().is_none() {
        return true;
    }
    let t_str = target.to_string_lossy();
    let top_str = toplevel.to_string_lossy();
    if same_path(&t_str, &top_str) {
        return true;
    }
    // Refuse if `target` is an ancestor of `toplevel`.
    let t_norm = format!("{}/", norm_sidebar_path(&t_str));
    let top_norm = format!("{}/", norm_sidebar_path(&top_str));
    if top_norm.starts_with(&t_norm) {
        return true;
    }
    if let Some(base) = directories::BaseDirs::new() {
        let home_str = base.home_dir().to_string_lossy();
        if same_path(&t_str, &home_str) {
            return true;
        }
    }
    false
}

fn remove_leftover_tree_safe(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    unlink_directory_links(root)?;
    // Clear read-only attributes on leftover files if needed (common on Windows Git pack/index files).
    let mut dirs = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(meta) = fs::symlink_metadata(&dir) else {
            continue;
        };
        if is_meta_reparse_or_symlink(&meta) {
            continue;
        }
        dirs.push(dir.clone());
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let full = entry.path();
            let Ok(m) = fs::symlink_metadata(&full) else {
                continue;
            };
            if is_meta_directory_link(&full, &m) {
                let _ = fs::remove_dir(&full);
                continue;
            }
            if is_meta_reparse_or_symlink(&m) || m.is_file() {
                if m.permissions().readonly() {
                    let mut perms = m.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    perms.set_readonly(false);
                    let _ = fs::set_permissions(&full, perms);
                }
                let _ = fs::remove_file(&full);
            } else if m.is_dir() {
                stack.push(full);
            }
        }
    }
    for dir in dirs.into_iter().rev() {
        let _ = fs::remove_dir(&dir);
    }
    Ok(())
}

/// Safely removes a secondary Git worktree (never the primary checkout) and optionally deletes
/// its local branch when safe (or forced).
pub fn remove_worktree(
    project_cwd: &Path,
    wt_path: &Path,
    branch: Option<&str>,
    force: bool,
) -> Result<(), String> {
    if !wt_path.is_absolute() {
        return Err("Worktree path must be absolute.".into());
    }
    let summary = inspect_project(project_cwd)?;
    if is_protected_path(wt_path, &summary.toplevel) {
        return Err("Refusing to remove primary repository or protected directory.".into());
    }

    let wt_str = wt_path.to_string_lossy();
    let registered = summary
        .worktrees
        .iter()
        .find(|w| !w.is_primary && same_path(&w.path, &wt_str))
        .cloned();

    let Some(entry) = registered else {
        return Err("This folder is not a secondary Git worktree of the project.".into());
    };
    if entry.locked && !force {
        return Err("Worktree is locked in Git.".into());
    }

    if wt_path.exists() {
        if !force {
            let status = run_git_readonly(
                wt_path,
                &["status", "--porcelain", "--ignore-submodules=none"],
            )?;
            if !status.trim().is_empty() {
                return Err(
                    "The copy has modified or untracked files — nothing was removed.".into(),
                );
            }
        }
        unlink_directory_links(wt_path)?;

        let mut remove_args = vec!["-c", "core.longpaths=true", "worktree", "remove"];
        if force {
            remove_args.push("--force");
            if entry.locked {
                remove_args.push("--force");
            }
        }
        remove_args.push(&wt_str);

        let mut last_err = None;
        for attempt in 0..4 {
            match git::run(&summary.toplevel, &remove_args, None) {
                Ok(_) => {
                    last_err = None;
                    break;
                }
                Err(err) => {
                    last_err = Some(err);
                    if attempt < 3 {
                        thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        }
        if let Some(err) = last_err {
            return Err(err);
        }
        if wt_path.exists() {
            remove_leftover_tree_safe(wt_path)?;
        }
    } else {
        let _ = git::run(&summary.toplevel, &["worktree", "prune"], None);
    }

    let _ = git::run(&summary.toplevel, &["worktree", "prune"], None);

    let effective_branch = branch
        .filter(|b| !b.is_empty())
        .or_else(|| (!entry.branch.is_empty()).then_some(entry.branch.as_str()));

    if let Some(b) = effective_branch
        && b != summary.default_branch
        && run_git_readonly(
            &summary.toplevel,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{b}"),
            ],
        )
        .is_ok()
    {
        let merged_into_default = !force
            && !summary.default_branch.is_empty()
            && run_git_readonly(
                &summary.toplevel,
                &[
                    "merge-base",
                    "--is-ancestor",
                    &format!("refs/heads/{b}"),
                    &format!("refs/heads/{}", summary.default_branch),
                ],
            )
            .is_ok();
        let flag = if force || merged_into_default {
            "-D"
        } else {
            "-d"
        };
        let _ = git::run(&summary.toplevel, &["branch", flag, "--", b], None);
    }

    Ok(())
}

fn parse_ahead_behind(raw: &str) -> (usize, usize) {
    let mut parts = raw.split_whitespace();
    let behind = parts
        .next()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let ahead = parts
        .next()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    (ahead, behind)
}

/// Scans all non-primary worktrees of `project_cwd` and classifies each as `Safe`, `Risk`, or
/// `Blocker` compared to `default_branch`.
pub fn scan_cleanup(
    project_cwd: &Path,
    open_pane_counts: &HashMap<String, usize>,
) -> Result<(String, Vec<CleanupItem>), String> {
    let summary = inspect_project(project_cwd)?;
    let default_branch = summary.default_branch.clone();
    let has_default_ref = run_git_readonly(
        &summary.toplevel,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{default_branch}"),
        ],
    )
    .is_ok();

    let mut items = Vec::new();
    for wt in summary.worktrees.iter().filter(|w| !w.is_primary) {
        let wt_path = PathBuf::from(&wt.path);
        let folder_exists = wt_path.is_dir();

        let blocker = if wt.locked {
            Some(CleanupBlocker::Locked)
        } else if wt.detached || wt.branch.is_empty() {
            Some(CleanupBlocker::Detached)
        } else if wt.branch == default_branch {
            Some(CleanupBlocker::DefaultBranch)
        } else {
            None
        };

        let last_subject = if !wt.head.is_empty() {
            run_git_readonly(&summary.toplevel, &["log", "-1", "--format=%s", &wt.head])
                .map(|s| s.trim().to_owned())
                .unwrap_or_default()
        } else {
            String::new()
        };

        let (ahead, behind, merged) = if !wt.branch.is_empty() && has_default_ref {
            let counts = run_git_readonly(
                &summary.toplevel,
                &[
                    "rev-list",
                    "--left-right",
                    "--count",
                    &format!("{default_branch}...{}", wt.branch),
                ],
            )
            .map(|s| parse_ahead_behind(&s))
            .unwrap_or((0, 0));

            let is_merged = counts.0 == 0
                || run_git_readonly(
                    &summary.toplevel,
                    &["merge-base", "--is-ancestor", &wt.branch, &default_branch],
                )
                .is_ok();
            (counts.0, counts.1, is_merged)
        } else {
            (0, 0, false)
        };

        let mut risks = Vec::new();
        if blocker.is_none() {
            if !folder_exists {
                risks.push(CleanupRisk::MissingFolder);
            } else if let Ok(porcelain) = run_git_readonly(
                &wt_path,
                &["status", "--porcelain", "--ignore-submodules=none"],
            ) {
                let dirty_count = porcelain.lines().filter(|l| !l.trim().is_empty()).count();
                if dirty_count > 0 {
                    risks.push(CleanupRisk::Dirty(dirty_count));
                }
            }
            if !merged && ahead > 0 {
                risks.push(CleanupRisk::Unmerged(ahead));
            }
            let norm = norm_sidebar_path(&wt.path);
            if let Some(&panes) = open_pane_counts.get(&norm)
                && panes > 0
            {
                risks.push(CleanupRisk::OpenPanes(panes));
            }
        }

        items.push(CleanupItem {
            path: wt_path,
            branch: wt.branch.clone(),
            head: wt.head.clone(),
            last_subject,
            ahead,
            behind,
            merged,
            blocker,
            risks,
        });
    }

    Ok((default_branch, items))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_parser_extracts_primary_copies_locked_and_detached() {
        let sample = "\
worktree C:/repo/main
HEAD 1111111111111111111111111111111111111111
branch refs/heads/main

worktree C:/repo/main.worktrees/claude
HEAD 2222222222222222222222222222222222222222
branch refs/heads/agent/claude
locked reason here

worktree C:/repo/main.worktrees/detached-one
HEAD 3333333333333333333333333333333333333333
detached
";
        let list = parse_porcelain_worktrees(sample, 50);
        assert_eq!(list.len(), 3);
        assert!(list[0].is_primary);
        assert_eq!(list[0].branch, "main");
        assert!(!list[0].locked);
        assert!(!list[0].detached);

        assert!(!list[1].is_primary);
        assert_eq!(list[1].branch, "agent/claude");
        assert!(list[1].locked);
        assert!(!list[1].detached);

        assert!(!list[2].is_primary);
        assert!(list[2].branch.is_empty());
        assert!(list[2].detached);
    }

    #[test]
    fn slug_and_branch_formatting_follow_prefix_modes() {
        assert_eq!(slugify_worktree_name("Claude Code!!"), "claude-code");
        assert_eq!(slugify_worktree_name("   ---   "), "agent");
        assert_eq!(
            format_worktree_branch(BranchPrefixMode::Custom, "team/ai", None, "codex"),
            "team/ai/codex"
        );
        assert_eq!(
            format_worktree_branch(
                BranchPrefixMode::GitUsername,
                "agent",
                Some("Jean C."),
                "codex"
            ),
            "jean-c/codex"
        );
        assert_eq!(
            format_worktree_branch(BranchPrefixMode::None, "agent", Some("Jean"), "codex"),
            "codex"
        );
    }

    #[test]
    fn env_secret_matcher_skips_templates() {
        assert!(is_env_secret_filename(".env"));
        assert!(is_env_secret_filename(".env.local"));
        assert!(is_env_secret_filename(".env.development.local"));
        assert!(!is_env_secret_filename(".env.example"));
        assert!(!is_env_secret_filename(".env.sample"));
        assert!(!is_env_secret_filename(".env.local.example"));
        assert!(!is_env_secret_filename("README.md"));
    }
}
