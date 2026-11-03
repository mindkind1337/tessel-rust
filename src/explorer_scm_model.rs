//! Pure data models and exact behavioral parity logic for:
//! - Right-panel Files tree (`explorerRows.js`: compact folders, indent guides, sticky scroll)
//! - File & folder Git status propagation (`explorerStatus.js`)
//! - File-watch reload & git-status relevance filtering (`explorerChanges.js`)
//! - Source Control slow-status backoff coalescer & EN/FR labels (`scmState.js`, `scmLabels.js`)
//!
//! Pure deterministic state only: never spawns processes, timers, or background polling.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScmLang {
    #[default]
    En,
    Fr,
}

// ---------------------------------------------------------------------------
// 1. VS Code Explorer Rows, Compact Folders & Sticky Scroll (`explorerRows.js`)
// ---------------------------------------------------------------------------

pub const EXPLORER_ROW_HEIGHT: f32 = 22.0;
pub const EXPLORER_INDENT: f32 = 8.0;
pub const EXPLORER_BASE_INDENT: f32 = 8.0;
pub const EXPLORER_TWISTIE: f32 = 16.0;
pub const MAX_COMPACT_CHAIN: usize = 256;
pub const MAX_STICKY_FOLDERS: usize = 7;

#[must_use]
pub fn row_padding(depth: usize) -> f32 {
    EXPLORER_BASE_INDENT + (depth as f32) * EXPLORER_INDENT
}

#[must_use]
pub fn guide_x(depth: usize) -> f32 {
    EXPLORER_BASE_INDENT + (depth as f32) * EXPLORER_INDENT + EXPLORER_TWISTIE / 2.0
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplorerEntry {
    pub name: String,
    pub path: String,
    pub dir: bool,
    pub link: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplorerRow {
    pub path: String,
    pub name: String,
    pub dir: bool,
    pub depth: usize,
    pub entry: ExplorerEntry,
    pub chain: Vec<ExplorerEntry>,
    pub parent: String,
    pub open: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ExplorerBuildOpts<'a> {
    pub compact: bool,
    pub hidden_paths: Option<&'a HashSet<String>>,
    pub no_join_paths: Option<&'a HashSet<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExplorerTreeResult {
    pub rows: Vec<ExplorerRow>,
    pub index: HashMap<String, usize>,
}

fn visible_entries<'a>(
    nodes: &'a HashMap<String, Vec<ExplorerEntry>>,
    dir: &str,
    opts: &ExplorerBuildOpts<'_>,
) -> Option<Vec<&'a ExplorerEntry>> {
    let entries = nodes.get(dir)?;
    Some(
        entries
            .iter()
            .filter(|e| !opts.hidden_paths.is_some_and(|h| h.contains(&e.path)))
            .collect(),
    )
}

fn is_alone(path: &str, opts: &ExplorerBuildOpts<'_>) -> bool {
    opts.no_join_paths.is_some_and(|nj| nj.contains(path))
}

#[must_use]
pub fn build_explorer_rows(
    root: &str,
    nodes: &HashMap<String, Vec<ExplorerEntry>>,
    open_dirs: &HashSet<String>,
    opts: &ExplorerBuildOpts<'_>,
) -> ExplorerTreeResult {
    let mut out = ExplorerTreeResult::default();
    if root.is_empty() {
        return out;
    }

    fn walk(
        dir: &str,
        depth: usize,
        nodes: &HashMap<String, Vec<ExplorerEntry>>,
        open_dirs: &HashSet<String>,
        opts: &ExplorerBuildOpts<'_>,
        out: &mut ExplorerTreeResult,
    ) {
        let Some(entries) = visible_entries(nodes, dir, opts) else {
            return;
        };
        for e in entries {
            let mut chain: Vec<ExplorerEntry> = vec![e.clone()];
            if opts.compact && e.dir && !is_alone(&e.path, opts) {
                let mut cur = e;
                for _ in 0..MAX_COMPACT_CHAIN {
                    let Some(kids) = visible_entries(nodes, &cur.path, opts) else {
                        break;
                    };
                    if kids.len() != 1 || !kids[0].dir || is_alone(&kids[0].path, opts) {
                        break;
                    }
                    cur = kids[0];
                    chain.push(cur.clone());
                }
            }
            let last = chain.last().expect("chain starts non-empty").clone();
            let is_open = last.dir && open_dirs.contains(&last.path);
            let row_idx = out.rows.len();
            for c in &chain {
                out.index.insert(c.path.clone(), row_idx);
            }
            let last_path = last.path.clone();
            out.rows.push(ExplorerRow {
                path: last.path.clone(),
                name: last.name.clone(),
                dir: last.dir,
                depth,
                entry: last,
                chain,
                parent: dir.to_string(),
                open: is_open,
            });
            if is_open {
                walk(&last_path, depth + 1, nodes, open_dirs, opts, out);
            }
        }
    }

    walk(root, 0, nodes, open_dirs, opts, &mut out);
    out
}

#[must_use]
pub fn toggled_paths(row: &ExplorerRow) -> Vec<String> {
    if row.dir {
        row.chain.iter().map(|e| e.path.clone()).collect()
    } else {
        Vec::new()
    }
}

#[must_use]
pub fn sole_subfolder<'a>(
    nodes: &'a HashMap<String, Vec<ExplorerEntry>>,
    dir: &str,
    hidden_paths: Option<&HashSet<String>>,
) -> Option<&'a ExplorerEntry> {
    let entries = nodes.get(dir)?;
    let mut iter = entries
        .iter()
        .filter(|e| !hidden_paths.is_some_and(|h| h.contains(&e.path)));
    let first = iter.next()?;
    if iter.next().is_none() && first.dir {
        Some(first)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveIndentGuide {
    pub depth: usize,
    pub from: usize,
    pub to: usize,
}

#[must_use]
pub fn active_guide(
    rows: &[ExplorerRow],
    index: &HashMap<String, usize>,
    selected: Option<&str>,
) -> Option<ActiveIndentGuide> {
    let sel = selected.filter(|s| !s.is_empty())?;
    let &i = index.get(sel)?;
    let row = rows.get(i)?;

    let owner = if row.dir && row.open && i + 1 < rows.len() && rows[i + 1].depth > row.depth {
        Some(i)
    } else {
        (0..i).rev().find(|&j| rows[j].depth < row.depth)
    }?;

    let depth = rows[owner].depth;
    let mut to = owner + 1;
    while to < rows.len() && rows[to].depth > depth {
        to += 1;
    }
    (to > owner + 1).then_some(ActiveIndentGuide {
        depth,
        from: owner + 1,
        to,
    })
}

#[must_use]
pub fn segment_of(row: &ExplorerRow, path: &str) -> usize {
    row.chain
        .iter()
        .position(|e| e.path == path)
        .unwrap_or_else(|| row.chain.len().saturating_sub(1))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StickyFolder {
    pub at: usize,
    pub top: f32,
}

#[must_use]
pub fn sticky_folders(
    depths: &[usize],
    scroll_top: f32,
    row_height: f32,
    max_lines: usize,
) -> Vec<StickyFolder> {
    let n = depths.len();
    if n == 0 || scroll_top <= 0.0 || row_height <= 0.0 || max_lines == 0 {
        return Vec::new();
    }

    let mut parent: Vec<Option<usize>> = vec![None; n];
    let mut end: Vec<usize> = vec![0; n];
    let mut stack: Vec<usize> = Vec::new();

    for (i, &d) in depths.iter().enumerate() {
        while let Some(&top_idx) = stack.last() {
            if depths[top_idx] >= d {
                end[top_idx] = i;
                stack.pop();
            } else {
                break;
            }
        }
        parent[i] = stack.last().copied();
        stack.push(i);
    }
    while let Some(top_idx) = stack.pop() {
        end[top_idx] = n;
    }

    let mut out = Vec::new();
    for level in 0..max_lines {
        let y = scroll_top + (level as f32) * row_height;
        let under = (y / row_height).floor() as usize;
        if under >= n || depths[under] <= level {
            break;
        }
        let mut cur = Some(under);
        while let Some(idx) = cur {
            if depths[idx] > level {
                cur = parent[idx];
            } else {
                break;
            }
        }
        let Some(anc) = cur else {
            break;
        };
        if (anc as f32) * row_height >= y {
            break;
        }
        let level_top = (level as f32) * row_height;
        let push_up = (end[anc] as f32) * row_height - scroll_top - row_height;
        let top = level_top.min(push_up);
        out.push(StickyFolder { at: anc, top });
        if top < level_top {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 2. Explorer Git Status Letters & Folder Priority (`explorerStatus.js`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GitStatusLetter {
    Deleted,
    Modified,
    Added,
    Untracked,
    Renamed,
    Copied,
    Ignored,
}

impl GitStatusLetter {
    #[must_use]
    pub const fn priority(self) -> u8 {
        match self {
            Self::Deleted => 5,
            Self::Modified => 4,
            Self::Added | Self::Untracked => 3,
            Self::Renamed => 2,
            Self::Copied => 1,
            Self::Ignored => 0,
        }
    }

    #[must_use]
    pub const fn as_char(self) -> char {
        match self {
            Self::Deleted => 'D',
            Self::Modified => 'M',
            Self::Added => 'A',
            Self::Untracked => 'U',
            Self::Renamed => 'R',
            Self::Copied => 'C',
            Self::Ignored => '!',
        }
    }
}

#[must_use]
pub fn parent_of_path(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(idx) => &path[..idx],
        None => path,
    }
}

fn trim_trailing_sep(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
}

#[must_use]
pub fn in_untracked(map: &HashMap<String, GitStatusLetter>, key: &str, root_key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    let root = trim_trailing_sep(root_key);
    let mut cur = trim_trailing_sep(key);
    loop {
        let up = parent_of_path(cur);
        if up == cur || up.len() <= root.len() || (!root.is_empty() && !up.starts_with(root)) {
            return false;
        }
        cur = up;
        if let Some(&letter) = map.get(cur) {
            return letter == GitStatusLetter::Untracked;
        }
    }
}

#[must_use]
pub fn explorer_status_of(
    map: &HashMap<String, GitStatusLetter>,
    key: &str,
    root_key: &str,
) -> Option<GitStatusLetter> {
    if let Some(&letter) = map.get(key) {
        return (letter != GitStatusLetter::Ignored).then_some(letter);
    }
    in_untracked(map, key, root_key).then_some(GitStatusLetter::Untracked)
}

#[must_use]
pub fn untracked_folder(
    map: &HashMap<String, GitStatusLetter>,
    key: &str,
    root_key: &str,
) -> Option<GitStatusLetter> {
    if let Some(&letter) = map.get(key) {
        return (letter == GitStatusLetter::Untracked).then_some(GitStatusLetter::Untracked);
    }
    in_untracked(map, key, root_key).then_some(GitStatusLetter::Untracked)
}

#[must_use]
pub fn folder_status(
    map: &HashMap<String, GitStatusLetter>,
    root_key: &str,
) -> HashMap<String, GitStatusLetter> {
    let mut out: HashMap<String, GitStatusLetter> = HashMap::new();
    let root = trim_trailing_sep(root_key);
    for (p, &letter) in map {
        let prio = letter.priority();
        if prio == 0 {
            continue;
        }
        let mut dir = parent_of_path(p);
        while dir.len() > root.len() && dir.starts_with(root) {
            let replace = out.get(dir).is_none_or(|prev| prio > prev.priority());
            if replace {
                out.insert(dir.to_string(), letter);
            }
            let up = parent_of_path(dir);
            if up == dir {
                break;
            }
            dir = up;
        }
    }
    out
}

#[must_use]
pub fn ignored_set(map: &HashMap<String, GitStatusLetter>) -> HashSet<String> {
    map.iter()
        .filter(|&(_, &l)| l == GitStatusLetter::Ignored)
        .map(|(p, _)| trim_trailing_sep(p).to_string())
        .collect()
}

#[must_use]
pub fn is_ignored(ignored: &HashSet<String>, key: &str, root_key: &str) -> bool {
    if ignored.is_empty() || key.is_empty() {
        return false;
    }
    let root = trim_trailing_sep(root_key);
    let mut cur = trim_trailing_sep(key);
    while cur.len() > root.len() {
        if ignored.contains(cur) {
            return true;
        }
        let up = parent_of_path(cur);
        if up == cur {
            break;
        }
        cur = up;
    }
    false
}

// ---------------------------------------------------------------------------
// 3. Watch Change Filtering & Reload Selection (`explorerChanges.js`)
// ---------------------------------------------------------------------------

#[must_use]
pub fn in_git_dir(path: &str) -> bool {
    path.split(['/', '\\'])
        .any(|seg| seg.eq_ignore_ascii_case(".git"))
}

#[must_use]
pub fn status_matters(root: &str, paths: Option<&[String]>, ignored: &HashSet<String>) -> bool {
    let Some(list) = paths else {
        return true;
    };
    let root_key = root.to_lowercase();
    list.iter()
        .any(|p| in_git_dir(p) || !is_ignored(ignored, &p.to_lowercase(), &root_key))
}

#[must_use]
pub fn folders_to_reload(
    nodes: &HashMap<String, Vec<ExplorerEntry>>,
    paths: &[String],
    remote: bool,
    renamed: Option<&[String]>,
) -> Vec<String> {
    let by_key: HashMap<String, &String> = nodes.keys().map(|d| (d.to_lowercase(), d)).collect();
    let moved: Option<HashSet<String>> =
        renamed.map(|r| r.iter().map(|s| s.to_lowercase()).collect());

    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    let mut push_unique = |dir: &str| {
        if seen.insert(dir.to_string()) {
            out.push(dir.to_string());
        }
    };

    for p in paths {
        if in_git_dir(p) {
            continue;
        }
        let p_key = p.to_lowercase();
        if remote && let Some(&self_dir) = by_key.get(&p_key) {
            push_unique(self_dir);
        }
        let parent_key = parent_of_path(p).to_lowercase();
        let Some(&parent_dir) = by_key.get(&parent_key) else {
            continue;
        };
        let listed = nodes
            .get(parent_dir)
            .is_some_and(|entries| entries.iter().any(|e| e.path.to_lowercase() == p_key));
        let should_reload = if remote {
            !listed
        } else {
            moved.as_ref().is_none_or(|m| m.contains(&p_key)) || !listed
        };
        if should_reload {
            push_unique(parent_dir);
        }
    }
    out
}

#[must_use]
pub fn same_entries(a: &[ExplorerEntry], b: &[ExplorerEntry]) -> bool {
    a == b
}

#[must_use]
pub fn same_status_map(
    a: &HashMap<String, GitStatusLetter>,
    b: &HashMap<String, GitStatusLetter>,
) -> bool {
    a == b
}

// ---------------------------------------------------------------------------
// 4. SCM Backoff Coalescer & Translated Labels (`scmState.js`, `scmLabels.js`)
// ---------------------------------------------------------------------------

pub const SLOW_STATUS_MS: u64 = 3_000;
pub const MAX_STATUS_GAP_MS: u64 = 5 * 60 * 1000;

#[must_use]
pub fn scm_root_key(path: &str) -> String {
    let mut collapsed = String::with_capacity(path.len());
    let mut prev_sep = false;
    for ch in path.chars() {
        if ch == '/' || ch == '\\' {
            if !prev_sep {
                collapsed.push('\\');
                prev_sep = true;
            }
        } else {
            prev_sep = false;
            for lc in ch.to_lowercase() {
                collapsed.push(lc);
            }
        }
    }
    collapsed.trim_end_matches('\\').to_string()
}

#[must_use]
pub fn background_gap_ms(last_took_ms: u64, last_ended_ms: u64, now_ms: u64) -> u64 {
    if last_took_ms < SLOW_STATUS_MS {
        return 0;
    }
    let delay = (last_took_ms.saturating_mul(3)).min(MAX_STATUS_GAP_MS);
    last_ended_ms.saturating_add(delay).saturating_sub(now_ms)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScmRefreshDecision {
    StartNow { seq: u64, background: bool },
    DelayBackground { wait_ms: u64 },
    CoalescedBehindRunning,
}

#[derive(Debug, Clone, Default)]
struct ScmFolderSlot {
    seq: u64,
    running_seq: Option<u64>,
    running_started_ms: u64,
    next_queued: bool,
    next_foreground: bool,
    last_took_ms: u64,
    last_ended_ms: u64,
}

/// Deterministic single-flight + 1-queued-next status coordinator per root (`scmState.js`).
#[derive(Debug, Clone, Default)]
pub struct ScmRefreshCoalescer {
    slots: HashMap<String, ScmFolderSlot>,
}

impl ScmRefreshCoalescer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request_refresh(
        &mut self,
        root: &str,
        background: bool,
        now_ms: u64,
    ) -> ScmRefreshDecision {
        let key = scm_root_key(root);
        let slot = self.slots.entry(key).or_default();
        if slot.running_seq.is_none() {
            let gap = if background {
                background_gap_ms(slot.last_took_ms, slot.last_ended_ms, now_ms)
            } else {
                0
            };
            if gap > 0 {
                return ScmRefreshDecision::DelayBackground { wait_ms: gap };
            }
            slot.seq += 1;
            slot.running_seq = Some(slot.seq);
            slot.running_started_ms = now_ms;
            return ScmRefreshDecision::StartNow {
                seq: slot.seq,
                background,
            };
        }

        slot.next_queued = true;
        if !background {
            slot.next_foreground = true;
        }
        ScmRefreshDecision::CoalescedBehindRunning
    }

    /// Marks `seq` complete; returns `(is_current_answer, follow_up_action)`.
    pub fn finish_refresh(
        &mut self,
        root: &str,
        seq: u64,
        now_ms: u64,
    ) -> (bool, Option<ScmRefreshDecision>) {
        let key = scm_root_key(root);
        let Some(slot) = self.slots.get_mut(&key) else {
            return (false, None);
        };
        if slot.running_seq != Some(seq) {
            return (false, None);
        }
        slot.last_took_ms = now_ms.saturating_sub(slot.running_started_ms);
        slot.last_ended_ms = now_ms;
        slot.running_seq = None;

        let is_current = slot.seq == seq;
        if !slot.next_queued {
            return (is_current, None);
        }

        let foreground = slot.next_foreground;
        slot.next_queued = false;
        slot.next_foreground = false;

        let gap = if foreground {
            0
        } else {
            background_gap_ms(slot.last_took_ms, slot.last_ended_ms, now_ms)
        };
        if gap > 0 {
            return (
                is_current,
                Some(ScmRefreshDecision::DelayBackground { wait_ms: gap }),
            );
        }
        slot.seq += 1;
        slot.running_seq = Some(slot.seq);
        slot.running_started_ms = now_ms;
        (
            is_current,
            Some(ScmRefreshDecision::StartNow {
                seq: slot.seq,
                background: !foreground,
            }),
        )
    }

    pub fn forget(&mut self, root: &str) {
        self.slots.remove(&scm_root_key(root));
    }
}

#[must_use]
pub const fn change_count(
    is_repo: bool,
    entries_len: usize,
    truncated: bool,
    total_entries: Option<usize>,
) -> usize {
    if !is_repo {
        return 0;
    }
    match (truncated, total_entries) {
        (true, Some(total)) => total,
        _ => entries_len,
    }
}

#[must_use]
pub fn scm_text(text: &str, lang: ScmLang) -> String {
    if text.is_empty() || lang == ScmLang::En {
        return text.to_string();
    }
    match text {
        "Commit" => return "Commit".to_string(),
        "Stage All" => return "Tout indexer".to_string(),
        "Push" => return "Push".to_string(),
        "Pull" => return "Pull".to_string(),
        "Sync" => return "Synchroniser".to_string(),
        "Fetch" => return "Fetch".to_string(),
        "Fast-forward" => return "Avance rapide".to_string(),
        "Publish Branch" => return "Publier la branche".to_string(),
        "No Branch" => return "Aucune branche".to_string(),
        "Commit & Push" => return "Commit et push".to_string(),
        "Commit & Sync" => return "Commit et synchroniser".to_string(),
        "Commit in progress…" => return "Commit en cours…".to_string(),
        "Remote operation in progress — try again once it finishes" => {
            return "Opération distante en cours — réessayez une fois celle-ci terminée"
                .to_string();
        }
        "Remote operation in progress…" => return "Opération distante en cours…".to_string(),
        "Resolve conflicts before committing" => {
            return "Résolvez les conflits avant de commiter".to_string();
        }
        "Commit staged changes" => return "Committer les modifications indexées".to_string(),
        "Enter a commit message to commit" => {
            return "Saisissez un message de commit pour commiter".to_string();
        }
        "Stage all changes" => return "Indexer toutes les modifications".to_string(),
        "Stage at least one file to commit" => {
            return "Indexez au moins un fichier pour commiter".to_string();
        }
        "Nothing to commit. Branch is up to date." => {
            return "Rien à commiter. La branche est à jour.".to_string();
        }
        "Nothing to push" => return "Rien à pousser".to_string(),
        "Nothing to pull" => return "Rien à tirer".to_string(),
        "Nothing to fast-forward" => return "Rien à avancer".to_string(),
        "Branch is up to date" => return "La branche est à jour".to_string(),
        "Branch is already published" => return "La branche est déjà publiée".to_string(),
        _ => {}
    }

    if let Some(rest) = text.strip_prefix("Push ")
        && let Some(num) = rest
            .strip_suffix(" commit")
            .or_else(|| rest.strip_suffix(" commits"))
            .and_then(|s| s.parse::<usize>().ok())
    {
        return if num == 1 {
            "Pousser 1 commit".to_string()
        } else {
            format!("Pousser {num} commits")
        };
    }
    if let Some(rest) = text.strip_prefix("Pull ")
        && let Some(num) = rest
            .strip_suffix(" commit")
            .or_else(|| rest.strip_suffix(" commits"))
            .and_then(|s| s.parse::<usize>().ok())
    {
        return if num == 1 {
            "Tirer 1 commit".to_string()
        } else {
            format!("Tirer {num} commits")
        };
    }
    if let Some(rest) = text.strip_prefix("Fast-forward ")
        && let Some(num) = rest
            .strip_suffix(" commit")
            .or_else(|| rest.strip_suffix(" commits"))
            .and_then(|s| s.parse::<usize>().ok())
    {
        return if num == 1 {
            "Avance rapide de 1 commit".to_string()
        } else {
            format!("Avance rapide de {num} commits")
        };
    }
    if let Some(rest) = text.strip_prefix("Sync (↓")
        && let Some(inner) = rest.strip_suffix(')')
        && let Some((behind, ahead)) = inner.split_once(" ↑")
        && behind.parse::<usize>().is_ok()
        && ahead.parse::<usize>().is_ok()
    {
        return format!("Synchroniser (↓{behind} ↑{ahead})");
    }
    if let Some(upstream) = text.strip_prefix("Nothing to push to ") {
        return format!("Rien à pousser vers {upstream}");
    }
    if let Some(action) = text.strip_suffix(" in progress…") {
        return format!("{} en cours…", scm_text(action, lang));
    }

    text.to_string()
}

#[must_use]
pub fn scm_status_title(status: &str, lang: ScmLang) -> &str {
    match (status, lang) {
        ("modified", ScmLang::En) => "Modified",
        ("modified", ScmLang::Fr) => "Modifié",
        ("added", ScmLang::En) => "Added",
        ("added", ScmLang::Fr) => "Ajouté",
        ("deleted", ScmLang::En) => "Deleted",
        ("deleted", ScmLang::Fr) => "Supprimé",
        ("renamed", ScmLang::En) => "Renamed",
        ("renamed", ScmLang::Fr) => "Renommé",
        ("untracked", ScmLang::En) => "Untracked",
        ("untracked", ScmLang::Fr) => "Non suivi",
        ("copied", ScmLang::En) => "Copied",
        ("copied", ScmLang::Fr) => "Copié",
        (other, _) => other,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScmSectionKind {
    Conflicts,
    Staged,
    Unstaged,
    Untracked,
}

impl ScmSectionKind {
    #[must_use]
    pub const fn label(self, lang: ScmLang) -> &'static str {
        match (self, lang) {
            (Self::Conflicts, ScmLang::En) => "Conflicts",
            (Self::Conflicts, ScmLang::Fr) => "Conflits",
            (Self::Staged, ScmLang::En) => "Staged Changes",
            (Self::Staged, ScmLang::Fr) => "Modifications indexées",
            (Self::Untracked, ScmLang::En) => "Untracked Files",
            (Self::Untracked, ScmLang::Fr) => "Fichiers non suivis",
            (Self::Unstaged, ScmLang::En) => "Changes",
            (Self::Unstaged, ScmLang::Fr) => "Modifications",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscardConfirmationCopy {
    pub title: String,
    pub description: String,
    pub confirm_label: String,
}

#[must_use]
pub fn discard_entry_copy(
    path: &str,
    status: &str,
    deletes_file: bool,
    remote: bool,
    lang: ScmLang,
) -> DiscardConfirmationCopy {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if deletes_file {
        return match (remote, lang) {
            (true, ScmLang::En) => DiscardConfirmationCopy {
                title: format!("Delete \"{name}\"?"),
                description: "This will move this file to the trash on the remote host."
                    .to_string(),
                confirm_label: "Delete".to_string(),
            },
            (false, ScmLang::En) => DiscardConfirmationCopy {
                title: format!("Delete \"{name}\"?"),
                description: "This will move this file to the Recycle Bin.".to_string(),
                confirm_label: "Delete".to_string(),
            },
            (true, ScmLang::Fr) => DiscardConfirmationCopy {
                title: format!("Supprimer « {name} » ?"),
                description: "Ce fichier sera déplacé dans la corbeille de l'hôte distant."
                    .to_string(),
                confirm_label: "Supprimer".to_string(),
            },
            (false, ScmLang::Fr) => DiscardConfirmationCopy {
                title: format!("Supprimer « {name} » ?"),
                description: "Ce fichier sera déplacé dans la Corbeille.".to_string(),
                confirm_label: "Supprimer".to_string(),
            },
        };
    }
    if status == "deleted" {
        return match lang {
            ScmLang::En => DiscardConfirmationCopy {
                title: format!("Restore \"{name}\"?"),
                description:
                    "This will restore the file and discard the deletion. This cannot be undone."
                        .to_string(),
                confirm_label: "Restore".to_string(),
            },
            ScmLang::Fr => DiscardConfirmationCopy {
                title: format!("Restaurer « {name} » ?"),
                description:
                    "Le fichier sera restauré et sa suppression abandonnée. Cette action est irréversible."
                        .to_string(),
                confirm_label: "Restaurer".to_string(),
            },
        };
    }
    match lang {
        ScmLang::En => DiscardConfirmationCopy {
            title: format!("Discard changes to \"{name}\"?"),
            description: "This will revert all changes to this file. This cannot be undone."
                .to_string(),
            confirm_label: "Discard".to_string(),
        },
        ScmLang::Fr => DiscardConfirmationCopy {
            title: format!("Abandonner les modifications de « {name} » ?"),
            description:
                "Toutes les modifications de ce fichier seront annulées. Cette action est irréversible."
                    .to_string(),
            confirm_label: "Abandonner".to_string(),
        },
    }
}
