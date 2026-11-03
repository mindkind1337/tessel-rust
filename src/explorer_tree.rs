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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeContextKind {
    CreateFile,
    CreateFolder,
    Rename,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeAction {
    OpenFile(PathBuf),
    Context {
        kind: TreeContextKind,
        root: PathBuf,
        path: PathBuf,
        name: String,
    },
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
        self.cached_tree = build_explorer_rows(
            root_key,
            &self.nodes,
            &self.open_dirs,
            &ExplorerBuildOpts {
                compact: self.compact_folders,
                hidden_paths: None,
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
            if self.is_loading() {
                ui.spinner();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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

        if !self.error.is_empty() {
            ui.colored_label(ui.visuals().error_fg_color, &self.error);
        }
        if self.truncated {
            ui.small(tr("Search limit reached; some files are not listed."));
        }

        // Re-ensure if `Refresh` button was clicked in the header above.
        self.ensure_rows_built(&root_key);

        self.last_row_rects.clear();
        if self.cached_tree.rows.is_empty() {
            if self.is_loading() {
                ui.label(tr("Working…"));
            } else if self.error.is_empty() {
                ui.label(tr("No matching files."));
            }
            return emitted;
        }

        let active_key = active.map(norm_tree_key);
        let guide_target = self.selected_key.clone().or_else(|| active_key.clone());
        let guide = active_guide(
            &self.cached_tree.rows,
            &self.cached_tree.index,
            guide_target.as_deref(),
        );
        let mut clicked_row: Option<(usize, PathBuf)> = None;
        let row_count = self.cached_tree.rows.len();

        egui::ScrollArea::vertical()
            .id_salt("explorer-tree-scroll")
            .auto_shrink([false, false])
            .show_rows(ui, EXPLORER_ROW_HEIGHT, row_count, |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let font_id = egui::TextStyle::Body.resolve(ui.style());
                for idx in range {
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
                    let text_color = if is_active_doc {
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

                    response.context_menu(|ui| {
                        if !actions_enabled {
                            ui.label(tr("Save pending edits before changing files."));
                            return;
                        }
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

                    if response.clicked() {
                        clicked_row = Some((idx, row_path));
                    }
                }
            });

        if let Some((idx, row_path)) = clicked_row {
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
            } else {
                emitted = Some(TreeAction::OpenFile(row_path));
            }
        }

        emitted
    }
}
