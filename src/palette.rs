//! Pure models for `CommandPalette`, right `SidePanel` tabs/history, workspace edge-resize limits,
//! and `FloatingTerminal` height clamping.
//!
//! Transcribed directly from:
//! - `src/renderer/src/components/CommandPalette.vue`
//! - `src/renderer/src/sideTabs.js` & `src/renderer/src/components/SidePanel.vue`
//! - `src/renderer/src/layoutLimits.js`, `WorkspaceSidebar.vue`, & `App.vue` (`clampTaskPanel`, `toggleSideTab`)
//! - `src/renderer/src/floatingTerminal.js` (`clampHeight`)

use serde::{Deserialize, Serialize};

// --- CommandPalette (`CommandPalette.vue`) -------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteCommand {
    pub id: String,
    pub group: String,
    pub title: String,
    pub hint: Option<String>,
    pub shortcut: Option<String>,
}

impl PaletteCommand {
    pub fn new(id: impl Into<String>, group: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            group: group.into(),
            title: title.into(),
            hint: None,
            shortcut: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteRow {
    pub command: PaletteCommand,
    /// Present only where `group` changes from the previous row (`CommandPalette.vue:38-43`).
    pub header: Option<String>,
}

/// Filters and ranks commands (`CommandPalette.vue:20-43`):
/// - Every whitespace-separated word of `query.trim().to_lowercase()` must appear in
///   `"{title} {group} {hint}"`.
/// - Score `0` if `title` starts with `q`, `1` if `title` contains `q`, `2` otherwise.
/// - Ties preserve input order; group headers appear only at group boundaries.
pub fn filter_palette_commands(commands: &[PaletteCommand], query: &str) -> Vec<PaletteRow> {
    let q = query.trim().to_lowercase();
    let ordered: Vec<PaletteCommand> = if q.is_empty() {
        commands.to_vec()
    } else {
        let words: Vec<&str> = q.split_whitespace().collect();
        let mut scored: Vec<(u8, PaletteCommand)> = commands
            .iter()
            .filter_map(|cmd| {
                let title_lc = cmd.title.to_lowercase();
                let group_lc = cmd.group.to_lowercase();
                let hint_lc = cmd.hint.as_deref().unwrap_or("").to_lowercase();
                let hay = format!("{title_lc} {group_lc} {hint_lc}");
                if !words.iter().all(|w| hay.contains(w)) {
                    return None;
                }
                let score = if title_lc.starts_with(&q) {
                    0
                } else if title_lc.contains(&q) {
                    1
                } else {
                    2
                };
                Some((score, cmd.clone()))
            })
            .collect();
        scored.sort_by_key(|(score, _)| *score);
        scored.into_iter().map(|(_, cmd)| cmd).collect()
    };

    let mut rows = Vec::with_capacity(ordered.len());
    for (i, cmd) in ordered.into_iter().enumerate() {
        let prev_same = i > 0
            && rows
                .get(i - 1)
                .is_some_and(|prev: &PaletteRow| prev.command.group == cmd.group);
        let header = (!prev_same).then(|| cmd.group.clone());
        rows.push(PaletteRow {
            command: cmd,
            header,
        });
    }
    rows
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandPaletteState {
    pub open: bool,
    pub query: String,
    pub index: usize,
}

impl CommandPaletteState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.query.clear();
            self.index = 0;
        }
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn set_query(&mut self, next: impl Into<String>) {
        let next = next.into();
        if self.query != next {
            self.query = next;
            self.index = 0;
        }
    }

    pub fn move_selection(&mut self, delta: isize, count: usize) {
        if count == 0 {
            self.index = 0;
            return;
        }
        let cur = (self.index % count) as isize;
        self.index = (cur + delta).rem_euclid(count as isize) as usize;
    }

    pub fn selected_id<'a>(&self, rows: &'a [PaletteRow]) -> Option<&'a str> {
        rows.get(self.index).map(|r| r.command.id.as_str())
    }
}

// --- SidePanel (`sideTabs.js` + `SidePanel.vue` + `App.vue:toggleSideTab`) -----------

pub const SIDE_TABS: [&str; 7] = [
    "dashboard",
    "files",
    "changes",
    "tasks",
    "taskHistory",
    "history",
    "conversations",
];

pub const VISITED_TABS_CAP: usize = 20;

pub fn is_builtin_side_tab(id: &str) -> bool {
    SIDE_TABS.contains(&id)
}

/// Restores a saved side-panel tab (`sideTabs.js:restoredSideTab`):
/// - Known `SIDE_TABS` or existing `web-*` browser tab -> kept.
/// - Missing `web-*` browser tab -> `"dashboard"`.
/// - Empty or unknown -> `None`.
pub fn restored_side_tab(saved: Option<&str>, browser_ids: &[String]) -> Option<String> {
    let raw = saved?.trim();
    if raw.is_empty() {
        return None;
    }
    if is_builtin_side_tab(raw) || browser_ids.iter().any(|b| b == raw) {
        return Some(raw.to_owned());
    }
    if raw.starts_with("web-") {
        return Some("dashboard".to_owned());
    }
    None
}

/// Badge text for the `Changes` tab (`SidePanel.vue:147`): `None` at `0`, `"99+"` above `99`.
pub fn changes_badge(count: usize) -> Option<String> {
    if count == 0 {
        None
    } else if count > 99 {
        Some("99+".to_owned())
    } else {
        Some(count.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideBrowserPage {
    pub id: String,
    pub url: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidePanelState {
    pub open: bool,
    pub tab: String,
    pub fullscreen: bool,
    pub visited: Vec<String>,
    pub browsers: Vec<SideBrowserPage>,
}

impl Default for SidePanelState {
    fn default() -> Self {
        Self {
            open: false,
            tab: "tasks".to_owned(),
            fullscreen: false,
            visited: Vec::new(),
            browsers: Vec::new(),
        }
    }
}

impl SidePanelState {
    pub fn is_valid_tab(&self, id: &str) -> bool {
        is_builtin_side_tab(id) || self.browsers.iter().any(|b| b.id == id)
    }

    pub fn current_tab(&self) -> &str {
        if self.is_valid_tab(&self.tab) {
            &self.tab
        } else {
            "tasks"
        }
    }

    fn switch_tab_internal(&mut self, next: &str) {
        if self.tab != next {
            let prev = std::mem::replace(&mut self.tab, next.to_owned());
            if !prev.is_empty() {
                self.visited.push(prev);
                if self.visited.len() > VISITED_TABS_CAP {
                    self.visited.remove(0);
                }
            }
        }
    }

    /// Shortcut/button toggle (`App.vue:toggleSideTab`): same tab while open closes the panel.
    pub fn toggle_tab(&mut self, tab: &str) {
        if self.open && self.tab == tab {
            self.open = false;
            self.fullscreen = false;
        } else {
            self.switch_tab_internal(tab);
            self.open = true;
        }
    }

    /// Opens `tab` without ever closing the panel (`App.vue:showSideTab`).
    pub fn show_tab(&mut self, tab: &str) {
        self.switch_tab_internal(tab);
        self.open = true;
    }

    pub fn add_browser(&mut self, id: impl Into<String>, url: impl Into<String>, show: bool) {
        let id = id.into();
        self.browsers.push(SideBrowserPage {
            id: id.clone(),
            url: url.into(),
            title: String::new(),
        });
        if show {
            self.switch_tab_internal(&id);
            self.open = true;
        }
    }

    /// Closes a `web-*` side-browser tab and falls back to the latest valid `visited` tab,
    /// or `"dashboard"` (`SidePanel.vue:232-240`).
    pub fn close_browser(&mut self, id: &str) {
        self.browsers.retain(|b| b.id != id);
        if self.tab != id {
            return;
        }
        let fallback = self
            .visited
            .iter()
            .rev()
            .find(|cand| cand.as_str() != id && self.is_valid_tab(cand))
            .cloned()
            .unwrap_or_else(|| "dashboard".to_owned());
        self.tab = fallback;
    }
}

// --- Layout resize limits (`layoutLimits.js`, `WorkspaceSidebar.vue`, `App.vue`) -----

pub const SIDEBAR_DEFAULT_WIDTH: i32 = 280;
pub const SIDEBAR_MIN_WIDTH: i32 = 220;
pub const SIDEBAR_COLLAPSED_RAIL_WIDTH: i32 = 52;
pub const SIDEBAR_MIN_MAX: i32 = 500;

pub const TASK_PANEL_DEFAULT_WIDTH: i32 = 330;
pub const TASK_PANEL_MIN_WIDTH: i32 = 260;
pub const CENTER_MIN_WIDTH: i32 = 420;

pub const RESIZE_COLLAPSE_BELOW: i32 = 110;
pub const RESIZE_DOUBLE_CLICK_MS: u64 = 400;

/// Maximum width of the left workspace sidebar (`layoutLimits.js:sidebarMaxWidth`).
pub fn sidebar_max_width(window_width: i32) -> i32 {
    let scaled = (f64::from(window_width.max(0)) * 0.3).round() as i32;
    SIDEBAR_MIN_MAX.max(scaled)
}

pub fn clamp_sidebar_width(raw_width: i32, window_width: i32) -> i32 {
    raw_width.clamp(SIDEBAR_MIN_WIDTH, sidebar_max_width(window_width))
}

/// Effective width of the right side panel (`App.vue:clampTaskPanel`, `lines 3278-3282`):
/// always leaves at least `420px` for center panes beside the left sidebar.
pub fn clamp_task_panel_width(
    requested_width: i32,
    window_width: i32,
    sidebar_collapsed: bool,
    sidebar_width: i32,
) -> i32 {
    let side = if sidebar_collapsed {
        SIDEBAR_COLLAPSED_RAIL_WIDTH
    } else {
        sidebar_width
    };
    let max_width = TASK_PANEL_MIN_WIDTH.max(window_width - side - CENTER_MIN_WIDTH);
    requested_width.clamp(TASK_PANEL_MIN_WIDTH, max_width)
}

pub fn is_resize_double_click(prev_down_ms: Option<u64>, now_ms: u64) -> bool {
    prev_down_ms.is_some_and(|prev| now_ms.saturating_sub(prev) < RESIZE_DOUBLE_CLICK_MS)
}

/// Evaluates pointer release on a collapsible edge (`WorkspaceSidebar.vue:397-415`):
/// - When open and dragged `< 110px` from the window edge, collapses while preserving `saved_width`.
/// - When collapsed and dragged `> 110px` from the window edge, re-opens at `clamped_width`.
pub fn evaluate_edge_drag(
    distance_from_edge_px: i32,
    was_open: bool,
    saved_width: i32,
    clamped_width: i32,
) -> (bool, i32) {
    if was_open {
        if distance_from_edge_px < RESIZE_COLLAPSE_BELOW {
            (false, saved_width)
        } else {
            (true, clamped_width)
        }
    } else if distance_from_edge_px > RESIZE_COLLAPSE_BELOW {
        (true, clamped_width)
    } else {
        (false, saved_width)
    }
}

// --- FloatingTerminal height bounds (`floatingTerminal.js:15-24`) --------------------

pub const FLOATING_MIN_HEIGHT: f64 = 0.15;
pub const FLOATING_MAX_HEIGHT: f64 = 0.90;
pub const FLOATING_DEFAULT_HEIGHT: f64 = 0.45;

pub fn clamp_floating_height(height: f64) -> f64 {
    if !height.is_finite() {
        return FLOATING_DEFAULT_HEIGHT;
    }
    let rounded = (height * 1000.0).round() / 1000.0;
    rounded.clamp(FLOATING_MIN_HEIGHT, FLOATING_MAX_HEIGHT)
}
