//! Native `egui` file explorer tree (`ExplorerPanel.vue` / `explorerRows.js` parity).
//!
//! Guarantees:
//! - Zero filesystem I/O on the UI thread (`show`, `set_root`, `refresh`, `poll` never touch `std::fs`).
//! - At most one background worker thread active at any instant; `set_root()` or `refresh()`
//!   immediately cancels any in-flight enumeration (even when the panel is closed) and discards
//!   stale results before pumping the next queued directory on that single worker slot.
//! - Dirty-flagged `ExplorerTreeResult` row cache (`rows_dirty`) so steady-state 60fps `egui`
//!   frames do zero cloning or tree rebuilding.
//! - Expanded folders persist across `refresh()` and across project switches (bounded LRU),
//!   while `refresh()` resets `path_by_key` and prunes deleted/unreadable folders.
//! - Non-lossy `OsString` path joining (`dir.join(item.file_name())`), raw examined-entry and
//!   wall-clock budgets, and canonicalized root confinement refusing symlinks and Windows
//!   junctions (`FILE_ATTRIBUTE_REPARSE_POINT`).

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt::Write as _,
    fs,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use eframe::egui;

use crate::{
    explorer_scm_model::{
        EXPLORER_ROW_HEIGHT, EXPLORER_TWISTIE, ExplorerBuildOpts, ExplorerEntry,
        ExplorerTreeResult, active_guide, build_explorer_rows, guide_x, row_padding, toggled_paths,
    },
    i18n::tr,
};

pub const WIN_FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

#[must_use]
pub const fn has_reparse_point_attr(file_attributes: u32) -> bool {
    (file_attributes & WIN_FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

#[must_use]
pub fn norm_tree_key(path: &Path) -> String {
    if let Some(utf8) = path.to_str() {
        #[cfg(windows)]
        {
            let mut s = utf8.replace('/', "\\");
            if s.len() > 3 {
                s = s.trim_end_matches('\\').to_string();
            }
            return s.to_lowercase();
        }
        #[cfg(not(windows))]
        {
            let mut s = utf8.to_string();
            if s.len() > 1 {
                s = s.trim_end_matches('/').to_string();
            }
            return s;
        }
    }

    // Lossless disambiguation for non-UTF-8 / unpaired-surrogate OS paths:
    let mut key = path.to_string_lossy().to_lowercase();
    key.push_str("#raw:");
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for unit in path.as_os_str().encode_wide() {
            let norm = if unit == u16::from(b'/') {
                u16::from(b'\\')
            } else if let Ok(ascii) = u8::try_from(unit) {
                u16::from(ascii.to_ascii_lowercase())
            } else {
                unit
            };
            let _ = write!(key, "{norm:04x}");
        }
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::ffi::OsStrExt;
        for &b in path.as_os_str().as_bytes() {
            let _ = write!(key, "{b:02x}");
        }
    }
    key
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeLimits {
    pub max_entries_per_dir: usize,
    pub max_raw_dir_entries: usize,
    pub max_cached_dirs: usize,
    pub max_total_cached_entries: usize,
    pub max_depth: usize,
    pub max_remembered_projects: usize,
    pub dir_list_timeout: Duration,
}

impl Default for TreeLimits {
    fn default() -> Self {
        Self {
            max_entries_per_dir: 5_000,
            max_raw_dir_entries: 25_000,
            max_cached_dirs: 512,
            max_total_cached_entries: 25_000,
            max_depth: 24,
            max_remembered_projects: 32,
            dir_list_timeout: Duration::from_secs(3),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeDirEntry {
    pub name: String,
    pub path: PathBuf,
    pub directory: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirListing {
    pub entries: Vec<TreeDirEntry>,
    pub truncated: bool,
}

pub trait DirectoryLister: Send + Sync + 'static {
    fn list_dir(
        &self,
        root: &Path,
        dir: &Path,
        limits: &TreeLimits,
        cancel: &AtomicBool,
    ) -> Result<DirListing, String>;
}

fn is_meta_symlink_or_reparse(meta: &fs::Metadata) -> bool {
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

fn is_path_symlink_or_reparse(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return true;
    };
    is_meta_symlink_or_reparse(&meta)
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FsDirectoryLister;

impl DirectoryLister for FsDirectoryLister {
    fn list_dir(
        &self,
        root: &Path,
        dir: &Path,
        limits: &TreeLimits,
        cancel: &AtomicBool,
    ) -> Result<DirListing, String> {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        if is_path_symlink_or_reparse(root) {
            return Err("Path is outside this project (including symlinks).".into());
        }
        let canon_root = root.canonicalize().map_err(|e| e.to_string())?;
        if dir != root {
            let rel = dir
                .strip_prefix(root)
                .map_err(|_| "Path is outside this project (including symlinks).".to_string())?;
            let mut cur = root.to_path_buf();
            for comp in rel.components() {
                let Component::Normal(part) = comp else {
                    return Err("Path is outside this project (including symlinks).".into());
                };
                cur.push(part);
                if is_path_symlink_or_reparse(&cur) {
                    return Err("Path is outside this project (including symlinks).".into());
                }
            }
        }
        let canon_dir = dir.canonicalize().map_err(|e| e.to_string())?;
        if !canon_dir.starts_with(&canon_root) {
            return Err("Path is outside this project (including symlinks).".into());
        }

        let started = Instant::now();
        let read_dir = fs::read_dir(&canon_dir).map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        let mut raw_seen = 0usize;
        let mut truncated = false;

        for item in read_dir {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            if started.elapsed() >= limits.dir_list_timeout {
                truncated = true;
                break;
            }
            raw_seen += 1;
            if raw_seen > limits.max_raw_dir_entries {
                truncated = true;
                break;
            }
            let item = item.map_err(|e| e.to_string())?;
            let Ok(meta) = fs::symlink_metadata(item.path()) else {
                continue;
            };
            if is_meta_symlink_or_reparse(&meta) {
                continue;
            }
            if entries.len() >= limits.max_entries_per_dir {
                truncated = true;
                break;
            }
            let os_name = item.file_name();
            let entry_path = dir.join(&os_name);
            let display_name = os_name.to_string_lossy().into_owned();
            entries.push(TreeDirEntry {
                name: display_name,
                path: entry_path,
                directory: meta.is_dir(),
            });
        }
        entries.sort_by(|a, b| {
            b.directory
                .cmp(&a.directory)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(DirListing { entries, truncated })
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeContextKind {
    CreateFile,
    CreateFolder,
    Rename,
    Trash,
    Delete,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeAction {
    OpenFile(PathBuf),
    OpenPinnedFile(PathBuf),
    CopyPath(PathBuf),
    Context {
        kind: TreeContextKind,
        root: PathBuf,
        path: PathBuf,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContentMatch {
    path: PathBuf,
    rel_display: String,
    line: usize,
    preview: String,
}

struct ContentSearchTask {
    query_key: String,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Vec<ContentMatch>>,
}

impl Drop for ContentSearchTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn search_project_contents(root: &Path, query: &str, cancel: &AtomicBool) -> Vec<ContentMatch> {
    const MAX_MATCHES: usize = 200;
    const MAX_FILES_EXAMINED: usize = 4_000;
    const MAX_FILE_BYTES: u64 = 512 * 1024;
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let Ok(canonical_root) = root.canonicalize() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0_usize)];
    let mut files_seen = 0_usize;
    let deadline = Instant::now() + Duration::from_secs(3);

    while let Some((dir, depth)) = stack.pop() {
        if cancel.load(Ordering::Relaxed)
            || Instant::now() >= deadline
            || out.len() >= MAX_MATCHES
            || files_seen >= MAX_FILES_EXAMINED
        {
            break;
        }
        let Ok(read_dir) = fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<fs::DirEntry> = read_dir.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries.into_iter().rev() {
            if cancel.load(Ordering::Relaxed) || out.len() >= MAX_MATCHES {
                break;
            }
            let name = entry.file_name();
            let name_lossy = name.to_string_lossy();
            if matches!(
                name_lossy.as_ref(),
                ".git" | "node_modules" | "target" | "dist" | ".next"
            ) {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let path = entry.path();
            if is_path_symlink_or_reparse(&path) {
                continue;
            }
            let Ok(resolved) = path.canonicalize() else {
                continue;
            };
            if !resolved.starts_with(&canonical_root) {
                continue;
            }
            if meta.is_dir() {
                if depth < 16 {
                    stack.push((path, depth + 1));
                }
            } else if meta.is_file() && meta.len() <= MAX_FILE_BYTES {
                files_seen += 1;
                let Ok(bytes) = fs::read(&path) else {
                    continue;
                };
                if bytes.iter().take(4096).any(|&b| b == 0) {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes);
                let rel_display = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                for (idx, line) in text.lines().enumerate() {
                    if line.to_lowercase().contains(&needle) {
                        let trimmed: String = line.trim().chars().take(120).collect();
                        out.push(ContentMatch {
                            path: path.clone(),
                            rel_display: rel_display.clone(),
                            line: idx + 1,
                            preview: trimmed,
                        });
                        if out.len() >= MAX_MATCHES {
                            break;
                        }
                    }
                }
            }
        }
    }
    out
}

fn git_badge_color(ch: char) -> egui::Color32 {
    match ch {
        'M' => egui::Color32::from_rgb(0xe2, 0xc0, 0x8d),
        'A' | '?' => egui::Color32::from_rgb(0x73, 0xc9, 0x91),
        'D' | 'U' => egui::Color32::from_rgb(0xf1, 0x4c, 0x4c),
        'R' => egui::Color32::from_rgb(0x4e, 0xc9, 0xb0),
        _ => egui::Color32::GRAY,
    }
}

struct WorkerTask {
    generation: u64,
    root_key: String,
    dir_key: String,
    depth: usize,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Result<DirListing, String>>,
}

impl Drop for WorkerTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub struct ExplorerTree {
    lister: Arc<dyn DirectoryLister>,
    limits: TreeLimits,
    project_root: Option<PathBuf>,
    generation: u64,
    dirs: HashMap<String, Vec<TreeDirEntry>>,
    nodes: HashMap<String, Vec<ExplorerEntry>>,
    path_by_key: HashMap<String, PathBuf>,
    total_cached_entries: usize,
    truncated: bool,
    error: String,
    open_dirs: HashSet<String>,
    restore_open: HashSet<String>,
    expanded_by_project: VecDeque<(String, HashSet<String>)>,
    selected_key: Option<String>,
    compact_folders: bool,
    show_dotfiles: bool,
    filter_query: String,
    gpui_applied_filter: String,
    gpui_filter_due: Option<Instant>,
    content_search_mode: bool,
    content_search_hits: Vec<ContentMatch>,
    content_search_last_key: String,
    content_search_task: Option<ContentSearchTask>,
    git_badges: HashMap<String, char>,
    cached_tree: ExplorerTreeResult,
    rows_dirty: bool,
    row_rebuild_count: usize,
    pending_queue: VecDeque<(PathBuf, usize)>,
    queued_keys: HashSet<String>,
    in_flight: Option<WorkerTask>,
    last_row_rects: HashMap<String, egui::Rect>,
}

impl Default for ExplorerTree {
    fn default() -> Self {
        Self::new()
    }
}

impl ExplorerTree {
    #[must_use]
    pub fn new() -> Self {
        Self::with_lister_and_limits(Arc::new(FsDirectoryLister), TreeLimits::default())
    }

    #[must_use]
    pub fn with_lister_and_limits(lister: Arc<dyn DirectoryLister>, limits: TreeLimits) -> Self {
        Self {
            lister,
            limits,
            project_root: None,
            generation: 0,
            dirs: HashMap::new(),
            nodes: HashMap::new(),
            path_by_key: HashMap::new(),
            total_cached_entries: 0,
            truncated: false,
            error: String::new(),
            open_dirs: HashSet::new(),
            restore_open: HashSet::new(),
            expanded_by_project: VecDeque::new(),
            selected_key: None,
            compact_folders: false,
            show_dotfiles: true,
            filter_query: String::new(),
            gpui_applied_filter: String::new(),
            gpui_filter_due: None,
            content_search_mode: false,
            content_search_hits: Vec::new(),
            content_search_last_key: String::new(),
            content_search_task: None,
            git_badges: HashMap::new(),
            cached_tree: ExplorerTreeResult::default(),
            rows_dirty: true,
            row_rebuild_count: 0,
            pending_queue: VecDeque::new(),
            queued_keys: HashSet::new(),
            in_flight: None,
            last_row_rects: HashMap::new(),
        }
    }

    #[must_use]
    pub fn fixture(
        root: PathBuf,
        dirs: Vec<(PathBuf, Vec<TreeDirEntry>)>,
        expanded: Vec<PathBuf>,
    ) -> Self {
        let mut tree = Self::new();
        let root_key = norm_tree_key(&root);
        tree.project_root = Some(root.clone());
        tree.path_by_key.insert(root_key, root);
        for exp in expanded.into_iter().take(tree.limits.max_cached_dirs) {
            let k = norm_tree_key(&exp);
            tree.path_by_key.insert(k.clone(), exp);
            tree.open_dirs.insert(k);
        }
        for (dir, entries) in dirs {
            let dk = norm_tree_key(&dir);
            tree.path_by_key.insert(dk.clone(), dir);
            let mapped = entries
                .iter()
                .map(|entry| {
                    let ek = norm_tree_key(&entry.path);
                    tree.path_by_key.insert(ek.clone(), entry.path.clone());
                    ExplorerEntry {
                        name: entry.name.clone(),
                        path: ek,
                        dir: entry.directory,
                        link: false,
                    }
                })
                .collect();
            tree.total_cached_entries += entries.len();
            tree.nodes.insert(dk.clone(), mapped);
            tree.dirs.insert(dk, entries);
        }
        tree.rows_dirty = true;
        tree
    }

    pub fn set_compact_folders(&mut self, compact: bool) {
        if self.compact_folders != compact {
            self.compact_folders = compact;
            self.rows_dirty = true;
        }
    }

    #[allow(dead_code)]
    pub fn set_show_dotfiles(&mut self, show: bool) {
        if self.show_dotfiles != show {
            self.show_dotfiles = show;
            self.rows_dirty = true;
        }
    }

    #[allow(dead_code)]
    #[must_use]
    pub const fn show_dotfiles(&self) -> bool {
        self.show_dotfiles
    }

    #[allow(dead_code)]
    #[must_use]
    pub const fn compact_folders(&self) -> bool {
        self.compact_folders
    }

    #[allow(dead_code)]
    pub fn set_filter_query(&mut self, query: impl Into<String>) {
        self.filter_query = query.into();
        self.gpui_applied_filter = self.filter_query.trim().to_owned();
        self.gpui_filter_due = None;
    }

    #[allow(dead_code)]
    pub fn set_git_badges(&mut self, badges: HashMap<String, char>) {
        self.git_badges = badges;
    }

    pub fn collapse_all(&mut self) {
        if !self.open_dirs.is_empty() || !self.restore_open.is_empty() {
            self.open_dirs.clear();
            self.restore_open.clear();
            self.rows_dirty = true;
            self.save_current_expanded();
        }
    }

    #[must_use]
    pub fn is_loading(&self) -> bool {
        self.in_flight.is_some() || !self.pending_queue.is_empty()
    }

    #[must_use]
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    #[must_use]
    pub fn error(&self) -> &str {
        &self.error
    }

    #[must_use]
    pub fn cached_dir_count(&self) -> usize {
        self.dirs.len()
    }

    #[must_use]
    pub fn total_cached_entries(&self) -> usize {
        self.total_cached_entries
    }

    #[must_use]
    pub fn tracked_path_count(&self) -> usize {
        self.path_by_key.len()
    }

    #[must_use]
    pub fn row_rebuild_count(&self) -> usize {
        self.row_rebuild_count
    }

    #[must_use]
    pub fn is_expanded(&self, dir: &Path) -> bool {
        self.open_dirs.contains(&norm_tree_key(dir))
    }

    #[must_use]
    pub fn rendered_row_rect(&self, path: &Path) -> Option<egui::Rect> {
        self.last_row_rects.get(&norm_tree_key(path)).copied()
    }

    fn effective_expanded_for_save(&self) -> HashSet<String> {
        let mut combined = self.open_dirs.clone();
        for k in &self.restore_open {
            if combined.len() >= self.limits.max_cached_dirs {
                break;
            }
            combined.insert(k.clone());
        }
        combined
    }

    fn save_current_expanded(&mut self) {
        let Some(cur_root) = &self.project_root else {
            return;
        };
        let key = norm_tree_key(cur_root);
        self.expanded_by_project.retain(|(k, _)| k != &key);
        if self.limits.max_remembered_projects == 0 {
            return;
        }
        while self.expanded_by_project.len() >= self.limits.max_remembered_projects {
            self.expanded_by_project.pop_front();
        }
        let saved = self.effective_expanded_for_save();
        self.expanded_by_project.push_back((key, saved));
    }

    fn restore_expanded_for(&self, root_key: &str) -> HashSet<String> {
        self.expanded_by_project
            .iter()
            .find(|(k, _)| k == root_key)
            .map(|(_, set)| {
                set.iter()
                    .take(self.limits.max_cached_dirs)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn enqueue_dir(&mut self, dir: PathBuf, depth: usize) {
        if depth > self.limits.max_depth {
            self.truncated = true;
            return;
        }
        if self.dirs.len() + self.queued_keys.len() >= self.limits.max_cached_dirs {
            self.truncated = true;
            return;
        }
        let key = norm_tree_key(&dir);
        if self.dirs.contains_key(&key) || !self.queued_keys.insert(key.clone()) {
            return;
        }
        self.path_by_key.insert(key, dir.clone());
        self.pending_queue.push_back((dir, depth));
    }

    fn pump_worker(&mut self) {
        if self.in_flight.is_some() {
            return;
        }
        let Some(root) = self.project_root.clone() else {
            self.pending_queue.clear();
            self.queued_keys.clear();
            return;
        };
        let Some((dir, depth)) = self.pending_queue.pop_front() else {
            return;
        };
        let dir_key = norm_tree_key(&dir);
        self.queued_keys.remove(&dir_key);
        let root_key = norm_tree_key(&root);
        let generation = self.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let lister = Arc::clone(&self.lister);
        let limits = self.limits.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let res = lister.list_dir(&root, &dir, &limits, &worker_cancel);
            let _ = tx.send(res);
        });
        self.in_flight = Some(WorkerTask {
            generation,
            root_key,
            dir_key,
            depth,
            cancel,
            rx,
        });
    }

    /// Updates the active project root immediately (even when the panel is closed),
    /// cancelling any in-flight enumeration from the previous project without UI-thread I/O.
    pub fn set_root(&mut self, root: Option<&Path>) {
        let next_key = root.map(norm_tree_key);
        let cur_key = self.project_root.as_deref().map(norm_tree_key);
        if next_key == cur_key {
            return;
        }
        self.save_current_expanded();
        self.generation = self.generation.wrapping_add(1);
        if let Some(task) = &self.in_flight {
            task.cancel.store(true, Ordering::Relaxed);
        }
        self.dirs.clear();
        self.nodes.clear();
        self.path_by_key.clear();
        self.total_cached_entries = 0;
        self.truncated = false;
        self.error.clear();
        self.open_dirs.clear();
        self.restore_open.clear();
        self.cached_tree = ExplorerTreeResult::default();
        self.rows_dirty = true;
        self.pending_queue.clear();
        self.queued_keys.clear();
        self.selected_key = None;
        self.last_row_rects.clear();
        self.content_search_hits.clear();
        self.content_search_last_key.clear();
        self.content_search_task = None;
        self.gpui_applied_filter = self.filter_query.trim().to_owned();
        self.gpui_filter_due = None;
        self.project_root = root.map(Path::to_path_buf);

        if let Some(r) = root {
            let rk = norm_tree_key(r);
            self.restore_open = self.restore_expanded_for(&rk);
            self.path_by_key.insert(rk, r.to_path_buf());
            self.enqueue_dir(r.to_path_buf(), 0);
            self.pump_worker();
        }
    }

    pub fn refresh(&mut self) {
        let Some(root) = self.project_root.clone() else {
            return;
        };
        let preserved = self.effective_expanded_for_save();
        self.generation = self.generation.wrapping_add(1);
        if let Some(task) = &self.in_flight {
            task.cancel.store(true, Ordering::Relaxed);
        }
        self.dirs.clear();
        self.nodes.clear();
        self.path_by_key.clear();
        self.total_cached_entries = 0;
        self.truncated = false;
        self.error.clear();
        self.open_dirs.clear();
        self.restore_open = preserved;
        self.cached_tree = ExplorerTreeResult::default();
        self.rows_dirty = true;
        self.pending_queue.clear();
        self.queued_keys.clear();
        self.last_row_rects.clear();

        let rk = norm_tree_key(&root);
        self.path_by_key.insert(rk, root.clone());
        self.enqueue_dir(root, 0);
        self.pump_worker();
    }

    pub fn poll(&mut self) {
        let max_steps = self.limits.max_cached_dirs.saturating_add(2);
        for _ in 0..max_steps {
            if let Some(task) = &self.in_flight {
                match task.rx.try_recv() {
                    Err(TryRecvError::Empty) => break,
                    Ok(result) => {
                        let finished = self.in_flight.take().expect("checked Some");
                        let current_root_key = self.project_root.as_deref().map(norm_tree_key);
                        let is_current = finished.generation == self.generation
                            && current_root_key.as_deref() == Some(finished.root_key.as_str())
                            && !finished.cancel.load(Ordering::Relaxed);
                        if is_current {
                            self.apply_listing(finished.dir_key.clone(), finished.depth, result);
                        }
                    }
                    Err(TryRecvError::Disconnected) => {
                        let finished = self.in_flight.take().expect("checked Some");
                        let current_root_key = self.project_root.as_deref().map(norm_tree_key);
                        if finished.generation == self.generation
                            && current_root_key.as_deref() == Some(finished.root_key.as_str())
                            && !finished.cancel.load(Ordering::Relaxed)
                        {
                            self.error = tr("Failed").to_string();
                        }
                    }
                }
            }
            if self.in_flight.is_none() {
                if self.pending_queue.is_empty() {
                    break;
                }
                self.pump_worker();
            }
        }

        if !self.is_loading() {
            if !self.restore_open.is_empty() {
                self.restore_open.clear();
                self.save_current_expanded();
            }
            if let Some(sel) = &self.selected_key
                && !self.path_by_key.contains_key(sel)
            {
                self.selected_key = None;
            }
        }
    }

    fn apply_listing(&mut self, dir_key: String, depth: usize, result: Result<DirListing, String>) {
        match result {
            Ok(mut listing) => {
                if self.dirs.len() >= self.limits.max_cached_dirs {
                    self.truncated = true;
                    return;
                }
                if listing.truncated {
                    self.truncated = true;
                }
                let remaining_budget = self
                    .limits
                    .max_total_cached_entries
                    .saturating_sub(self.total_cached_entries);
                if listing.entries.len() > remaining_budget {
                    listing.entries.truncate(remaining_budget);
                    self.truncated = true;
                }
                let mut child_dirs_to_expand = Vec::new();
                let mut mapped_entries = Vec::with_capacity(listing.entries.len());
                let post_dirs = self.dirs.len() + usize::from(!self.dirs.contains_key(&dir_key));
                for entry in &listing.entries {
                    let ek = norm_tree_key(&entry.path);
                    self.path_by_key.insert(ek.clone(), entry.path.clone());
                    if entry.directory {
                        let should_open =
                            self.open_dirs.contains(&ek) || self.restore_open.remove(&ek);
                        if should_open {
                            let open_fits = self.open_dirs.contains(&ek)
                                || self.open_dirs.len() < self.limits.max_cached_dirs;
                            let cache_fits = self.dirs.contains_key(&ek)
                                || self.queued_keys.contains(&ek)
                                || (post_dirs
                                    + self.queued_keys.len()
                                    + child_dirs_to_expand.len()
                                    < self.limits.max_cached_dirs);
                            if depth < self.limits.max_depth && open_fits && cache_fits {
                                self.open_dirs.insert(ek.clone());
                                child_dirs_to_expand.push(entry.path.clone());
                            } else {
                                self.open_dirs.remove(&ek);
                                self.truncated = true;
                            }
                        }
                    }
                    mapped_entries.push(ExplorerEntry {
                        name: entry.name.clone(),
                        path: ek,
                        dir: entry.directory,
                        link: false,
                    });
                }
                self.total_cached_entries += listing.entries.len();
                self.nodes.insert(dir_key.clone(), mapped_entries);
                self.dirs.insert(dir_key, listing.entries);
                self.rows_dirty = true;
                for child_path in child_dirs_to_expand {
                    self.enqueue_dir(child_path, depth + 1);
                }
            }
            Err(err) => {
                let removed = self.open_dirs.remove(&dir_key);
                let removed_restore = self.restore_open.remove(&dir_key);
                if removed {
                    self.rows_dirty = true;
                }
                if removed || removed_restore {
                    self.save_current_expanded();
                }
                self.error = err;
            }
        }
    }

    fn ensure_rows_built(&mut self, root_key: &str) {
        if !self.rows_dirty {
            return;
        }
        let hidden = if self.show_dotfiles {
            None
        } else {
            let mut set = HashSet::new();
            for list in self.nodes.values() {
                for entry in list {
                    if entry.name.starts_with('.') {
                        set.insert(entry.path.clone());
                    }
                }
            }
            Some(set)
        };
        self.cached_tree = build_explorer_rows(
            root_key,
            &self.nodes,
            &self.open_dirs,
            &ExplorerBuildOpts {
                compact: self.compact_folders,
                hidden_paths: hidden.as_ref(),
                no_join_paths: None,
            },
        );
        self.rows_dirty = false;
        self.row_rebuild_count = self.row_rebuild_count.saturating_add(1);
    }

    fn selected_target_dir(&self, root_path: &Path) -> PathBuf {
        let Some(sel_key) = &self.selected_key else {
            return root_path.to_path_buf();
        };
        let Some(sel_path) = self.path_by_key.get(sel_key) else {
            return root_path.to_path_buf();
        };
        let is_known_dir = self.open_dirs.contains(sel_key)
            || self.dirs.contains_key(sel_key)
            || self
                .cached_tree
                .index
                .get(sel_key)
                .and_then(|&i| self.cached_tree.rows.get(i))
                .is_some_and(|r| r.dir);
        if is_known_dir {
            sel_path.clone()
        } else {
            sel_path
                .parent()
                .filter(|p| p.starts_with(root_path))
                .unwrap_or(root_path)
                .to_path_buf()
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        root: Option<&Path>,
        active: Option<&Path>,
        actions_enabled: bool,
    ) -> Option<TreeAction> {
        self.set_root(root);
        self.poll();
        if self.is_loading() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }

        let Some(root_path) = self.project_root.clone() else {
            self.last_row_rects.clear();
            ui.label(tr("Select a project first."));
            return None;
        };

        let root_key = norm_tree_key(&root_path);
        self.ensure_rows_built(&root_key);

        let mut emitted: Option<TreeAction> = None;
        let root_title = root_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| tr("Files").to_string());
        let new_target_dir = self.selected_target_dir(&root_path);

        ui.horizontal(|ui| {
            ui.strong(root_title);
            if self.is_loading() || self.content_search_task.is_some() {
                ui.spinner();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("⋯", |ui| {
                    let mut show_dots = self.show_dotfiles;
                    if ui.checkbox(&mut show_dots, tr("Show dotfiles")).changed() {
                        self.set_show_dotfiles(show_dots);
                    }
                    let mut compact = self.compact_folders;
                    if ui.checkbox(&mut compact, tr("Compact folders")).changed() {
                        self.set_compact_folders(compact);
                    }
                });
                if ui
                    .small_button("⊟")
                    .on_hover_text(tr("Collapse all"))
                    .clicked()
                {
                    self.collapse_all();
                }
                ui.add_enabled_ui(actions_enabled, |ui| {
                    ui.menu_button(tr("New…"), |ui| {
                        for (title, kind) in [
                            ("Create file", TreeContextKind::CreateFile),
                            ("Create folder", TreeContextKind::CreateFolder),
                        ] {
                            if ui.button(tr(title)).clicked() {
                                emitted = Some(TreeAction::Context {
                                    kind,
                                    root: root_path.clone(),
                                    path: new_target_dir.clone(),
                                    name: String::new(),
                                });
                                ui.close();
                            }
                        }
                    });
                });
                if ui.small_button(tr("Refresh")).clicked() {
                    self.refresh();
                }
            });
        });

        // `.explorer-tools`: filename filter + bounded project content search toggle
        ui.horizontal(|ui| {
            let mode_label = if self.content_search_mode {
                tr("Text")
            } else {
                tr("Name")
            };
            if ui
                .selectable_label(self.content_search_mode, mode_label)
                .on_hover_text(tr("Toggle file name filter vs project text search"))
                .clicked()
            {
                self.content_search_mode = !self.content_search_mode;
                self.content_search_last_key.clear();
            }
            let hint = if self.content_search_mode {
                tr("Search in files…")
            } else {
                tr("Filter files…")
            };
            let avail = (ui.available_width() - 28.0_f32).max(60.0_f32);
            ui.add_sized(
                [avail, 20.0_f32],
                egui::TextEdit::singleline(&mut self.filter_query).hint_text(hint),
            );
            if !self.filter_query.is_empty() && ui.small_button("×").clicked() {
                self.filter_query.clear();
                self.content_search_hits.clear();
                self.content_search_last_key.clear();
                self.content_search_task = None;
            }
        });

        if let Some(task) = &self.content_search_task {
            match task.rx.try_recv() {
                Ok(hits) => {
                    if task.query_key == self.content_search_last_key {
                        self.content_search_hits = hits;
                    }
                    self.content_search_task = None;
                }
                Err(TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(60));
                }
                Err(TryRecvError::Disconnected) => {
                    self.content_search_task = None;
                }
            }
        }

        let trimmed_query = self.filter_query.trim().to_owned();
        if self.content_search_mode && !trimmed_query.is_empty() {
            let query_key = format!("{}::{trimmed_query}", root_key);
            if self.content_search_last_key != query_key {
                self.content_search_last_key = query_key.clone();
                let cancel = Arc::new(AtomicBool::new(false));
                let worker_cancel = Arc::clone(&cancel);
                let worker_root = root_path.clone();
                let worker_q = trimmed_query.clone();
                let (tx, rx) = mpsc::sync_channel(1);
                thread::spawn(move || {
                    let res = search_project_contents(&worker_root, &worker_q, &worker_cancel);
                    let _ = tx.send(res);
                });
                self.content_search_task = Some(ContentSearchTask {
                    query_key,
                    cancel,
                    rx,
                });
                ui.ctx().request_repaint_after(Duration::from_millis(60));
            }
            self.last_row_rects.clear();
            if self.content_search_hits.is_empty() {
                if self.content_search_task.is_some() {
                    ui.label(tr("Working…"));
                } else {
                    ui.label(tr("No matching files."));
                }
                return emitted;
            }
            egui::ScrollArea::vertical()
                .id_salt("explorer-content-hits")
                .auto_shrink([false, false])
                .show_rows(ui, 36.0_f32, self.content_search_hits.len(), |ui, range| {
                    for idx in range {
                        let hit = &self.content_search_hits[idx];
                        let w = ui.available_width().max(120.0_f32);
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(w, 34.0_f32), egui::Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                4.0_f32,
                                ui.visuals().widgets.hovered.bg_fill,
                            );
                        }
                        let p = ui
                            .painter()
                            .with_clip_rect(rect.shrink2(egui::vec2(4.0, 2.0)));
                        p.text(
                            egui::pos2(rect.left() + 6.0_f32, rect.top() + 9.0_f32),
                            egui::Align2::LEFT_CENTER,
                            format!("{}:{}", hit.rel_display, hit.line),
                            egui::FontId::proportional(11.5_f32),
                            ui.visuals().strong_text_color(),
                        );
                        p.text(
                            egui::pos2(rect.left() + 6.0_f32, rect.top() + 24.0_f32),
                            egui::Align2::LEFT_CENTER,
                            &hit.preview,
                            egui::FontId::monospace(10.5_f32),
                            ui.visuals().weak_text_color(),
                        );
                        if resp.double_clicked() {
                            emitted = Some(TreeAction::OpenPinnedFile(hit.path.clone()));
                        } else if resp.clicked() {
                            emitted = Some(TreeAction::OpenFile(hit.path.clone()));
                        }
                    }
                });
            return emitted;
        }

        if !self.error.is_empty() {
            ui.colored_label(ui.visuals().error_fg_color, &self.error);
        }
        if self.truncated {
            ui.small(tr("Search limit reached; some files are not listed."));
        }

        // Re-ensure if `Refresh` or options changed in the header above.
        self.ensure_rows_built(&root_key);

        self.last_row_rects.clear();
        let filter_lc = trimmed_query.to_lowercase();
        let visible_indices: Vec<usize> = if filter_lc.is_empty() {
            (0..self.cached_tree.rows.len()).collect()
        } else {
            self.cached_tree
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    r.name.to_lowercase().contains(&filter_lc)
                        || r.path.to_lowercase().contains(&filter_lc)
                })
                .map(|(i, _)| i)
                .collect()
        };

        if visible_indices.is_empty() {
            if self.is_loading() {
                ui.label(tr("Working…"));
            } else if self.error.is_empty() {
                ui.label(tr("No matching files."));
            }
            return emitted;
        }

        let active_key = active.map(norm_tree_key);
        let guide_target = self.selected_key.clone().or_else(|| active_key.clone());
        let guide = if filter_lc.is_empty() {
            active_guide(
                &self.cached_tree.rows,
                &self.cached_tree.index,
                guide_target.as_deref(),
            )
        } else {
            None
        };
        let mut clicked_row: Option<(usize, PathBuf, bool)> = None;
        let row_count = visible_indices.len();

        egui::ScrollArea::vertical()
            .id_salt("explorer-tree-scroll")
            .auto_shrink([false, false])
            .show_rows(ui, EXPLORER_ROW_HEIGHT, row_count, |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let font_id = egui::TextStyle::Body.resolve(ui.style());
                for vis_i in range {
                    let idx = visible_indices[vis_i];
                    let row = &self.cached_tree.rows[idx];
                    let row_path = self
                        .path_by_key
                        .get(&row.path)
                        .cloned()
                        .unwrap_or_else(|| PathBuf::from(&row.path));
                    let is_active_doc = active_key.as_deref() == Some(row.path.as_str());
                    let is_selected = self.selected_key.as_deref() == Some(row.path.as_str());

                    let width = ui.available_width().max(120.0);
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(width, EXPLORER_ROW_HEIGHT),
                        egui::Sense::click(),
                    );
                    self.last_row_rects.insert(row.path.clone(), rect);

                    let display_name = if row.chain.len() > 1 {
                        row.chain
                            .iter()
                            .map(|c| c.name.as_str())
                            .collect::<Vec<_>>()
                            .join("/")
                    } else {
                        row.name.clone()
                    };
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::SelectableLabel,
                            ui.is_enabled(),
                            is_selected || is_active_doc,
                            &display_name,
                        )
                    });

                    let painter = ui.painter_at(rect);
                    if is_active_doc || is_selected {
                        painter.rect_filled(rect, 3.0_f32, ui.visuals().selection.bg_fill);
                    } else if response.hovered() {
                        painter.rect_filled(rect, 3.0_f32, ui.visuals().widgets.hovered.bg_fill);
                    }

                    for d in 0..row.depth {
                        let gx = rect.left() + guide_x(d);
                        let active_seg =
                            guide.is_some_and(|g| g.depth == d && idx >= g.from && idx < g.to);
                        let stroke_color = if active_seg {
                            ui.visuals().widgets.active.fg_stroke.color
                        } else {
                            ui.visuals().widgets.noninteractive.bg_stroke.color
                        };
                        painter.line_segment(
                            [egui::pos2(gx, rect.top()), egui::pos2(gx, rect.bottom())],
                            egui::Stroke::new(1.0_f32, stroke_color),
                        );
                    }

                    let pad_x = rect.left() + row_padding(row.depth);
                    if row.dir {
                        let twistie = if row.open && !self.dirs.contains_key(&row.path) {
                            "…"
                        } else if row.open {
                            "▾"
                        } else {
                            "▸"
                        };
                        painter.text(
                            egui::pos2(pad_x + EXPLORER_TWISTIE * 0.5, rect.center().y),
                            egui::Align2::CENTER_CENTER,
                            twistie,
                            font_id.clone(),
                            ui.visuals().weak_text_color(),
                        );
                    }

                    let rel_badge_key = row_path
                        .strip_prefix(&root_path)
                        .ok()
                        .map(|rel| rel.to_string_lossy().replace('\\', "/").to_lowercase());
                    let git_badge = rel_badge_key
                        .as_deref()
                        .and_then(|k| self.git_badges.get(k).copied());

                    let text_color = if let Some(ch) = git_badge
                        && !row.dir
                    {
                        git_badge_color(ch)
                    } else if is_active_doc {
                        ui.visuals().strong_text_color()
                    } else {
                        ui.visuals().text_color()
                    };
                    painter.text(
                        egui::pos2(pad_x + EXPLORER_TWISTIE + 2.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &display_name,
                        font_id.clone(),
                        text_color,
                    );

                    if let Some(ch) = git_badge {
                        let badge_col = git_badge_color(ch);
                        if row.dir {
                            painter.circle_filled(
                                egui::pos2(rect.right() - 10.0_f32, rect.center().y),
                                3.0_f32,
                                badge_col,
                            );
                        } else {
                            painter.text(
                                egui::pos2(rect.right() - 8.0_f32, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                ch.to_string(),
                                egui::FontId::monospace(10.5_f32),
                                badge_col,
                            );
                        }
                    }

                    response.context_menu(|ui| {
                        if ui.button(tr("Copy path")).clicked() {
                            emitted = Some(TreeAction::CopyPath(row_path.clone()));
                            ui.close();
                            return;
                        }
                        if !actions_enabled {
                            ui.separator();
                            ui.label(tr("Save pending edits before changing files."));
                            return;
                        }
                        ui.separator();
                        if row.dir {
                            for (title, kind) in [
                                ("Create file", TreeContextKind::CreateFile),
                                ("Create folder", TreeContextKind::CreateFolder),
                            ] {
                                if ui.button(tr(title)).clicked() {
                                    emitted = Some(TreeAction::Context {
                                        kind,
                                        root: root_path.clone(),
                                        path: row_path.clone(),
                                        name: String::new(),
                                    });
                                    ui.close();
                                }
                            }
                            ui.separator();
                        }
                        for (title, kind) in [
                            ("Rename…", TreeContextKind::Rename),
                            ("Move to Recycle Bin", TreeContextKind::Trash),
                            ("Delete…", TreeContextKind::Delete),
                        ] {
                            if ui.button(tr(title)).clicked() {
                                emitted = Some(TreeAction::Context {
                                    kind,
                                    root: root_path.clone(),
                                    path: row_path.clone(),
                                    name: row.name.clone(),
                                });
                                ui.close();
                            }
                        }
                    });

                    if response.double_clicked() && !row.dir {
                        clicked_row = Some((idx, row_path, true));
                    } else if response.clicked() {
                        clicked_row = Some((idx, row_path, false));
                    }
                }
            });

        if let Some((idx, row_path, pinned)) = clicked_row {
            let row = self.cached_tree.rows[idx].clone();
            self.selected_key = Some(row.path.clone());
            if row.dir {
                let paths = toggled_paths(&row);
                let new_open_count = paths
                    .iter()
                    .filter(|p| !self.open_dirs.contains(p.as_str()))
                    .count();
                let needs_fetch =
                    !self.dirs.contains_key(&row.path) && !self.queued_keys.contains(&row.path);
                let can_fetch = !needs_fetch
                    || (self.dirs.len()
                        + self.queued_keys.len()
                        + usize::from(self.in_flight.is_some())
                        < self.limits.max_cached_dirs);
                if row.open {
                    for p in paths {
                        self.open_dirs.remove(&p);
                        self.restore_open.remove(&p);
                    }
                    self.rows_dirty = true;
                    self.save_current_expanded();
                } else if row.depth.saturating_add(row.chain.len()) <= self.limits.max_depth
                    && self.open_dirs.len().saturating_add(new_open_count)
                        <= self.limits.max_cached_dirs
                    && can_fetch
                {
                    let next_depth = row.depth.saturating_add(row.chain.len());
                    for p in paths {
                        self.open_dirs.insert(p);
                    }
                    self.rows_dirty = true;
                    self.save_current_expanded();
                    if !self.dirs.contains_key(&row.path) {
                        self.enqueue_dir(row_path, next_depth);
                        self.pump_worker();
                    }
                } else {
                    self.truncated = true;
                }
            } else if pinned {
                emitted = Some(TreeAction::OpenPinnedFile(row_path));
            } else {
                emitted = Some(TreeAction::OpenFile(row_path));
            }
        }

        emitted
    }

    #[allow(dead_code)]
    pub(crate) fn gpui_filter_query(&self) -> &str {
        &self.filter_query
    }

    #[allow(dead_code)]
    pub(crate) fn gpui_set_filter_query(&mut self, query: String) {
        if self.filter_query == query {
            return;
        }
        self.filter_query = query;
        self.gpui_schedule_filter(Instant::now());
    }
    fn gpui_schedule_filter(&mut self, now: Instant) {
        // Drop cancels an obsolete content worker before accepting any result.
        self.content_search_task = None;
        self.content_search_last_key.clear();
        if self.filter_query.trim().is_empty() {
            self.content_search_hits.clear();
            self.gpui_applied_filter.clear();
            self.gpui_filter_due = None;
        } else {
            self.gpui_filter_due =
                Some(now + Duration::from_millis(if self.content_search_mode { 300 } else { 120 }));
        }
    }
    fn gpui_advance_filter(&mut self, now: Instant) {
        if self.gpui_filter_due.is_some_and(|due| now >= due) {
            self.gpui_filter_due = None;
            self.gpui_applied_filter = self.filter_query.trim().to_owned();
        }
    }
    pub(crate) fn gpui_needs_frame(&self) -> bool {
        self.is_loading() || self.content_search_task.is_some() || self.gpui_filter_due.is_some()
    }
    #[allow(dead_code)]
    pub(crate) fn gpui_toggle_search_mode(&mut self) {
        self.content_search_mode = !self.content_search_mode;
        self.gpui_schedule_filter(Instant::now());
    }

    #[allow(dead_code)]
    pub(crate) fn gpui_click_row(&mut self, key: &str, pinned: bool) -> Option<TreeAction> {
        let idx = self.cached_tree.rows.iter().position(|r| r.path == key)?;
        let row = self.cached_tree.rows[idx].clone();
        let row_path = self
            .path_by_key
            .get(&row.path)
            .cloned()
            .unwrap_or_else(|| PathBuf::from(&row.path));
        self.selected_key = Some(row.path.clone());
        if row.dir {
            let paths = toggled_paths(&row);
            let new_open_count = paths
                .iter()
                .filter(|p| !self.open_dirs.contains(p.as_str()))
                .count();
            let needs_fetch =
                !self.dirs.contains_key(&row.path) && !self.queued_keys.contains(&row.path);
            let can_fetch = !needs_fetch
                || (self.dirs.len()
                    + self.queued_keys.len()
                    + usize::from(self.in_flight.is_some())
                    < self.limits.max_cached_dirs);
            if row.open {
                for p in paths {
                    self.open_dirs.remove(&p);
                    self.restore_open.remove(&p);
                }
                self.rows_dirty = true;
                self.save_current_expanded();
            } else if row.depth.saturating_add(row.chain.len()) <= self.limits.max_depth
                && self.open_dirs.len().saturating_add(new_open_count)
                    <= self.limits.max_cached_dirs
                && can_fetch
            {
                let next_depth = row.depth.saturating_add(row.chain.len());
                for p in paths {
                    self.open_dirs.insert(p);
                }
                self.rows_dirty = true;
                self.save_current_expanded();
                if !self.dirs.contains_key(&row.path) {
                    self.enqueue_dir(row_path, next_depth);
                    self.pump_worker();
                }
            } else {
                self.truncated = true;
            }
            None
        } else if pinned {
            Some(TreeAction::OpenPinnedFile(row_path))
        } else {
            Some(TreeAction::OpenFile(row_path))
        }
    }

    #[allow(dead_code)]
    pub(crate) fn gpui_rows_snapshot(
        &mut self,
        root: Option<&Path>,
        active: Option<&Path>,
    ) -> ExplorerGpuiSnapshot {
        self.set_root(root);
        self.poll();
        self.gpui_advance_filter(Instant::now());
        if let Some(task) = &self.content_search_task {
            match task.rx.try_recv() {
                Ok(hits) => {
                    if task.query_key == self.content_search_last_key {
                        self.content_search_hits = hits;
                    }
                    self.content_search_task = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.content_search_task = None;
                }
            }
        }
        let Some(root_path) = self.project_root.clone() else {
            return ExplorerGpuiSnapshot::default();
        };
        let root_key = norm_tree_key(&root_path);
        self.ensure_rows_built(&root_key);
        let root_title = root_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| tr("Files").to_string());
        let new_target_dir = self.selected_target_dir(&root_path);

        let trimmed_query = self.gpui_applied_filter.clone();
        if self.content_search_mode && !self.filter_query.trim().is_empty() {
            let query_key = format!("{root_key}::{trimmed_query}");
            if self.gpui_filter_due.is_none() && self.content_search_last_key != query_key {
                self.content_search_last_key = query_key.clone();
                let cancel = Arc::new(AtomicBool::new(false));
                let worker_cancel = Arc::clone(&cancel);
                let worker_root = root_path.clone();
                let worker_q = trimmed_query.clone();
                let (tx, rx) = mpsc::sync_channel(1);
                thread::spawn(move || {
                    let res = search_project_contents(&worker_root, &worker_q, &worker_cancel);
                    let _ = tx.send(res);
                });
                self.content_search_task = Some(ContentSearchTask {
                    query_key,
                    cancel,
                    rx,
                });
            }
            let content_hits = self
                .content_search_hits
                .iter()
                .take(250)
                .map(|h| ExplorerGpuiContentHit {
                    path: h.path.clone(),
                    rel_display: h.rel_display.clone(),
                    line: h.line,
                    preview: h.preview.clone(),
                })
                .collect();
            return ExplorerGpuiSnapshot {
                has_root: true,
                root_path,
                root_title,
                new_target_dir,
                filter_query: self.filter_query.clone(),
                content_search_mode: true,
                show_dotfiles: self.show_dotfiles,
                compact_folders: self.compact_folders,
                loading: self.gpui_needs_frame(),
                truncated: self.truncated,
                error: self.error.clone(),
                rows: Vec::new(),
                content_hits,
            };
        }

        let active_key = active.map(norm_tree_key);
        let filter_lc = trimmed_query.to_lowercase();
        let rows = self
            .cached_tree
            .rows
            .iter()
            .filter(|r| {
                filter_lc.is_empty()
                    || r.name.to_lowercase().contains(&filter_lc)
                    || r.path.to_lowercase().contains(&filter_lc)
            })
            .take(500)
            .map(|row| {
                let row_path = self
                    .path_by_key
                    .get(&row.path)
                    .cloned()
                    .unwrap_or_else(|| PathBuf::from(&row.path));
                let display_name = if row.chain.len() > 1 {
                    row.chain
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join("/")
                } else {
                    row.name.clone()
                };
                ExplorerGpuiRow {
                    key: row.path.clone(),
                    path: row_path,
                    name: row.name.clone(),
                    display_name,
                    depth: row.depth,
                    dir: row.dir,
                    open: row.open,
                    selected: self.selected_key.as_deref() == Some(row.path.as_str()),
                    active_doc: active_key.as_deref() == Some(row.path.as_str()),
                }
            })
            .collect();

        ExplorerGpuiSnapshot {
            has_root: true,
            root_path,
            root_title,
            new_target_dir,
            filter_query: self.filter_query.clone(),
            content_search_mode: self.content_search_mode,
            show_dotfiles: self.show_dotfiles,
            compact_folders: self.compact_folders,
            loading: self.gpui_needs_frame(),
            truncated: self.truncated,
            error: self.error.clone(),
            rows,
            content_hits: Vec::new(),
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct ExplorerGpuiRow {
    pub key: String,
    pub path: PathBuf,
    pub name: String,
    pub display_name: String,
    pub depth: usize,
    pub dir: bool,
    pub open: bool,
    pub selected: bool,
    pub active_doc: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct ExplorerGpuiContentHit {
    pub path: PathBuf,
    pub rel_display: String,
    pub line: usize,
    pub preview: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub(crate) struct ExplorerGpuiSnapshot {
    pub has_root: bool,
    pub root_path: PathBuf,
    pub root_title: String,
    pub new_target_dir: PathBuf,
    pub filter_query: String,
    pub content_search_mode: bool,
    pub show_dotfiles: bool,
    pub compact_folders: bool,
    pub loading: bool,
    pub truncated: bool,
    pub error: String,
    pub rows: Vec<ExplorerGpuiRow>,
    pub content_hits: Vec<ExplorerGpuiContentHit>,
}

#[cfg(test)]
mod native_filter_tests {
    use super::*;
    fn fixture(root: &Path) -> ExplorerTree {
        ExplorerTree::fixture(
            root.to_path_buf(),
            vec![(
                root.to_path_buf(),
                vec![
                    TreeDirEntry {
                        name: "alpha.txt".into(),
                        path: root.join("alpha.txt"),
                        directory: false,
                    },
                    TreeDirEntry {
                        name: "beta.txt".into(),
                        path: root.join("beta.txt"),
                        directory: false,
                    },
                ],
            )],
            vec![],
        )
    }
    #[test]
    fn native_filter_debounce_coalesces_names_and_clear_is_immediate() {
        let dir = tempfile::tempdir().unwrap();
        let mut tree = fixture(dir.path());
        tree.gpui_set_filter_query("a".into());
        tree.gpui_set_filter_query("alpha".into());
        let now = Instant::now() + Duration::from_secs(5);
        tree.gpui_schedule_filter(now);
        assert_eq!(tree.gpui_filter_due, Some(now + Duration::from_millis(120)));
        let before = tree.gpui_rows_snapshot(Some(dir.path()), None);
        assert_eq!(before.rows.len(), 2);
        assert!(before.loading);
        tree.gpui_advance_filter(now + Duration::from_millis(120));
        let after = tree.gpui_rows_snapshot(Some(dir.path()), None);
        assert_eq!(after.rows.len(), 1);
        assert_eq!(after.rows[0].name, "alpha.txt");
        tree.gpui_set_filter_query(String::new());
        assert_eq!(
            tree.gpui_rows_snapshot(Some(dir.path()), None).rows.len(),
            2
        );
        assert!(!tree.gpui_needs_frame());
    }
    #[test]
    fn native_filter_content_waits_cancels_obsolete_worker_and_repaints_completion() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alpha.txt"), "needle\n").unwrap();
        let mut tree = fixture(dir.path());
        tree.gpui_toggle_search_mode();
        tree.gpui_set_filter_query("needle".into());
        let now = Instant::now() + Duration::from_secs(5);
        tree.gpui_schedule_filter(now);
        assert_eq!(tree.gpui_filter_due, Some(now + Duration::from_millis(300)));
        let waiting = tree.gpui_rows_snapshot(Some(dir.path()), None);
        assert!(waiting.loading);
        assert!(waiting.rows.is_empty());
        assert!(tree.content_search_task.is_none());
        tree.gpui_advance_filter(now + Duration::from_millis(300));
        tree.gpui_rows_snapshot(Some(dir.path()), None);
        let cancel = Arc::clone(&tree.content_search_task.as_ref().unwrap().cancel);
        assert!(tree.gpui_needs_frame());
        tree.content_search_hits.push(ContentMatch {
            path: dir.path().join("alpha.txt"),
            rel_display: "alpha.txt".into(),
            line: 1,
            preview: "needle".into(),
        });
        tree.gpui_set_filter_query("different".into());
        assert!(cancel.load(Ordering::Relaxed));
        assert!(tree.content_search_task.is_none());
        let pending = tree.gpui_rows_snapshot(Some(dir.path()), None);
        assert!(pending.loading);
        assert_eq!(pending.content_hits.len(), 1);
        assert_eq!(pending.content_hits[0].preview, "needle");
        tree.gpui_set_filter_query(String::new());
        let cleared = tree.gpui_rows_snapshot(Some(dir.path()), None);
        assert!(cleared.content_hits.is_empty());
        assert!(!tree.gpui_needs_frame());
    }
}
