//! Native `egui` left workspace/project sidebar (`WorkspaceSidebar.vue`, `WorktreeCard.vue`,
//! and `CompactAgentRow.vue` parity).
//!
//! Composes directly with [`crate::icons`], [`crate::shortcuts::chords_of`], and
//! [`crate::sidebar_model`] (`SidebarLang`, `WorktreeCardStatus`) with **zero** filesystem,
//! git, or process I/O inside the widget:
//! - Top `Search` button (`Icon::Search` + localized `"Search"` / `"Rechercher"` +
//!   `Ctrl+Shift+P` / `Ctrl+Maj+P` keycaps) emitting [`ProjectAction::Search`].
//! - `"Projects"` / `"Projets"` section header with vector `FolderPlus` `"Add project"` /
//!   `"Ajouter un projet"` button emitting [`ProjectAction::OpenFolder`], strictly omitting
//!   unimplemented activity/filter/worktree/team menus.
//! - Collapsible project cards (`SelectProject(Some(id))`) and `"Sessions without a project"` /
//!   `"Sessions sans projet"` group (`SelectProject(None)`) with runtime collapse state persisted
//!   by project ID and pruned automatically when projects leave the catalog.
//! - Full-width clickable session rows (`SelectSession { project, id }`) with truthful
//!   `sleeping` status indicators (never fabricating unobserved `working` / `waiting` states)
//!   and focus-scoped keyboard navigation (`↑` / `↓` / `←` / `→` / `Enter` / `Space`) that never
//!   consumes keys when focus belongs to a terminal, editor, or dialog outside the panel.

use std::collections::HashSet;

use eframe::egui::{
    self, Color32, FontId, Id, Key, Modifiers, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};

use crate::icons::{self, Icon};
use crate::shortcuts::chords_of;
use crate::sidebar_model::{SidebarLang, WorktreeCardStatus};

/// Runtime kind of a session row displayed in [`ProjectPanel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Shell,
    Agent,
    Chat,
}

/// Caller-supplied session row metadata (zero I/O inside [`ProjectPanel`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    pub id: u64,
    pub title: String,
    pub kind: SessionKind,
    pub sleeping: bool,
}

/// Caller-supplied project card and its child sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectEntry {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub sessions: Vec<SessionEntry>,
}

/// Frame input for [`ProjectPanel::show`].
#[derive(Debug, Clone, Copy)]
pub struct ProjectPanelInput<'a> {
    pub projects: &'a [ProjectEntry],
    pub global_sessions: &'a [SessionEntry],
    pub active_project: Option<u64>,
    pub active_session: Option<u64>,
    pub french: bool,
}

/// User action emitted by [`ProjectPanel::show`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectAction {
    Search,
    OpenFolder,
    SelectProject(Option<u64>),
    SelectSession { project: Option<u64>, id: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavItem {
    Project(u64),
    GlobalHeader,
    Session { project: Option<u64>, id: u64 },
}

impl NavItem {
    fn widget_id(self, panel_id: Id) -> Id {
        match self {
            Self::Project(id) => panel_id.with(("project-card", id)),
            Self::GlobalHeader => panel_id.with("global-card"),
            Self::Session { project, id } => panel_id.with(("session-row", project, id)),
        }
    }

    const fn activate_action(self) -> ProjectAction {
        match self {
            Self::Project(id) => ProjectAction::SelectProject(Some(id)),
            Self::GlobalHeader => ProjectAction::SelectProject(None),
            Self::Session { project, id } => ProjectAction::SelectSession { project, id },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusOwner {
    Search,
    OpenFolder,
    List,
}

const fn sidebar_lang(french: bool) -> SidebarLang {
    if french {
        SidebarLang::Fr
    } else {
        SidebarLang::En
    }
}

const fn group_status(sessions: &[SessionEntry]) -> WorktreeCardStatus {
    if sessions.is_empty() {
        return WorktreeCardStatus::Inactive;
    }
    let mut idx = 0;
    while idx < sessions.len() {
        if !sessions[idx].sleeping {
            return WorktreeCardStatus::Active;
        }
        idx += 1;
    }
    WorktreeCardStatus::Sleeping
}

fn session_display_title(session: &SessionEntry) -> &str {
    let trimmed = session.title.trim();
    if !trimmed.is_empty() {
        trimmed
    } else {
        match session.kind {
            SessionKind::Shell => "Terminal",
            SessionKind::Agent => "Agent",
            SessionKind::Chat => "Chat",
        }
    }
}

fn session_hover_text(session: &SessionEntry, lang: SidebarLang) -> String {
    let title = session_display_title(session);
    let kind_label = match (session.kind, lang) {
        (SessionKind::Shell, _) => "Terminal",
        (SessionKind::Agent, _) => "Agent",
        (SessionKind::Chat, SidebarLang::En) => "Chat, no terminal",
        (SessionKind::Chat, SidebarLang::Fr) => "Chat, sans terminal",
    };
    if session.sleeping {
        format!(
            "{title}\n{kind_label} · {}",
            WorktreeCardStatus::Sleeping.label(lang)
        )
    } else {
        format!("{title}\n{kind_label}")
    }
}

fn collapse_button_label(name: &str, collapsed: bool, french: bool) -> String {
    match (collapsed, french) {
        (true, false) => format!("Expand {name}"),
        (false, false) => format!("Collapse {name}"),
        (true, true) => format!("Développer {name}"),
        (false, true) => format!("Réduire {name}"),
    }
}

/// Paints a crisp vector folder outline (and optional inner `+` for `FolderPlus`) inside `rect`,
/// preserving a centered `1:1` square aspect ratio (`lucide-vue-next` `folder` / `folder-plus`).
fn paint_folder_icon(painter: &Painter, rect: Rect, color: Color32, with_plus: bool) {
    let side = rect.width().min(rect.height());
    if !side.is_finite() || side <= 0.0_f32 {
        return;
    }
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let scale = side / 24.0_f32;
    let map = |x: f32, y: f32| -> Pos2 { Pos2::new(sq.min.x + x * scale, sq.min.y + y * scale) };
    let stroke = Stroke::new((1.85_f32 * scale).max(0.8_f32), color);

    let outline = vec![
        map(3.0_f32, 5.2_f32),
        map(9.2_f32, 5.2_f32),
        map(11.4_f32, 7.6_f32),
        map(21.0_f32, 7.6_f32),
        map(21.0_f32, 19.2_f32),
        map(3.0_f32, 19.2_f32),
    ];
    painter.add(egui::epaint::PathShape::closed_line(outline, stroke));

    if with_plus {
        let plus_stroke = Stroke::new((1.85_f32 * scale).max(0.8_f32), color);
        painter.line_segment(
            [map(12.0_f32, 10.6_f32), map(12.0_f32, 16.2_f32)],
            plus_stroke,
        );
        painter.line_segment(
            [map(9.2_f32, 13.4_f32), map(14.8_f32, 13.4_f32)],
            plus_stroke,
        );
    }
}

/// Paints a right-pointing collapse chevron (`-90deg` rotated `Icon::ChevronDown`, `style.css:10623`).
fn paint_chevron_right(painter: &Painter, rect: Rect, color: Color32) {
    let side = rect.width().min(rect.height());
    if !side.is_finite() || side <= 0.0_f32 {
        return;
    }
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let scale = side / 16.0_f32;
    let map = |x: f32, y: f32| -> Pos2 { Pos2::new(sq.min.x + x * scale, sq.min.y + y * scale) };
    let stroke = Stroke::new((1.8_f32 * scale).max(0.8_f32), color);
    let pts = vec![
        map(6.0_f32, 4.0_f32),
        map(10.0_f32, 8.0_f32),
        map(6.0_f32, 12.0_f32),
    ];
    let first = pts[0];
    let last = pts[2];
    painter.add(egui::epaint::PathShape::line(pts, stroke));
    let cap_r = stroke.width * 0.5_f32;
    if cap_r >= 0.4_f32 {
        painter.circle_filled(first, cap_r, color);
        painter.circle_filled(last, cap_r, color);
    }
}

/// Paints Lucide's exact crescent moon (`M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z`, `AgentStateDot.vue:60`).
fn paint_sleeping_moon(painter: &Painter, rect: Rect, color: Color32) {
    let side = rect.width().min(rect.height());
    if !side.is_finite() || side <= 0.0_f32 {
        return;
    }
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let scale = side / 24.0_f32;
    let map = |x: f32, y: f32| -> Pos2 { Pos2::new(sq.min.x + x * scale, sq.min.y + y * scale) };

    let mut pts = Vec::with_capacity(30);
    // Inner concave arc from (12, 3) [225deg] -> (12, 12) [135deg] -> (21, 12) [45deg]
    // centered at (16.5, 7.5) with radius 4.5 * sqrt(2).
    let inner_r = 4.5_f32 * std::f32::consts::SQRT_2;
    let inner_steps = 12usize;
    for step in 0..=inner_steps {
        let t = (step as f32) / (inner_steps as f32);
        let ang = (225.0_f32 - 180.0_f32 * t).to_radians();
        pts.push(map(
            16.5_f32 + inner_r * ang.cos(),
            7.5_f32 + inner_r * ang.sin(),
        ));
    }
    // Outer convex arc from (21, 12) [0deg] -> (12, 21) [90deg] -> (3, 12) [180deg] -> (12, 3) [270deg]
    // centered at (12, 12) with radius 9.0.
    let outer_steps = 16usize;
    for step in 1..outer_steps {
        let t = (step as f32) / (outer_steps as f32);
        let ang = (270.0_f32 * t).to_radians();
        pts.push(map(
            12.0_f32 + 9.0_f32 * ang.cos(),
            12.0_f32 + 9.0_f32 * ang.sin(),
        ));
    }
    let stroke = Stroke::new((1.75_f32 * scale).max(0.75_f32), color);
    painter.add(egui::epaint::PathShape::closed_line(pts, stroke));
}

fn paint_chat_icon(painter: &Painter, rect: Rect, color: Color32) {
    let side = rect.width().min(rect.height());
    if !side.is_finite() || side <= 0.0_f32 {
        return;
    }
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let scale = side / 16.0_f32;
    let map = |x: f32, y: f32| -> Pos2 { Pos2::new(sq.min.x + x * scale, sq.min.y + y * scale) };
    let stroke = Stroke::new((1.3_f32 * scale).max(0.75_f32), color);
    let pts = vec![
        map(2.5_f32, 3.0_f32),
        map(13.5_f32, 3.0_f32),
        map(13.5_f32, 10.6_f32),
        map(7.2_f32, 10.6_f32),
        map(4.2_f32, 13.2_f32),
        map(4.2_f32, 10.6_f32),
        map(2.5_f32, 10.6_f32),
    ];
    painter.add(egui::epaint::PathShape::closed_line(pts, stroke));
    painter.line_segment([map(5.2_f32, 5.8_f32), map(10.8_f32, 5.8_f32)], stroke);
    painter.line_segment([map(5.2_f32, 8.1_f32), map(9.2_f32, 8.1_f32)], stroke);
}

fn paint_group_status(painter: &Painter, rect: Rect, status: WorktreeCardStatus, muted: Color32) {
    match status {
        WorktreeCardStatus::Sleeping => {
            let moon_rect = Rect::from_center_size(rect.center(), Vec2::splat(12.5_f32));
            paint_sleeping_moon(painter, moon_rect, muted);
        }
        WorktreeCardStatus::Active => {
            painter.circle_filled(rect.center(), 4.0_f32, Color32::from_rgb(0x10, 0xb9, 0x81));
        }
        _ => {
            painter.circle_filled(
                rect.center(),
                3.75_f32,
                Color32::from_rgb(0x73, 0x73, 0x73).gamma_multiply(0.45_f32),
            );
        }
    }
}

fn paint_session_dot(painter: &Painter, rect: Rect, sleeping: bool, muted: Color32) {
    if sleeping {
        let moon_rect = Rect::from_center_size(rect.center(), Vec2::splat(10.5_f32));
        paint_sleeping_moon(painter, moon_rect, muted);
    } else {
        painter.circle_filled(
            rect.center(),
            3.0_f32,
            Color32::from_rgb(0x73, 0x73, 0x73).gamma_multiply(0.55_f32),
        );
    }
}

fn paint_single_line_ellipsis(
    painter: &Painter,
    rect: Rect,
    text: &str,
    font: FontId,
    color: Color32,
) {
    if rect.width() <= 1.0_f32 || rect.height() <= 1.0_f32 {
        return;
    }
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, rect.width());
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = painter.layout_job(job);
    let y = rect.center().y - galley.size().y * 0.5_f32;
    painter
        .with_clip_rect(rect)
        .galley(Pos2::new(rect.min.x, y), galley, color);
}

/// Native `egui` left sidebar project & session panel (`WorkspaceSidebar.vue` parity).
#[derive(Debug, Default, Clone)]
pub struct ProjectPanel {
    collapsed_projects: HashSet<u64>,
    global_collapsed: bool,
    focus_requested: bool,
    list_root_id: Option<Id>,
    focused_nav: Option<NavItem>,
    last_nav_items: Vec<NavItem>,
    held_activate_key: Option<Key>,
}

impl ProjectPanel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when project `id` is currently collapsed in this panel.
    #[must_use]
    pub fn is_project_collapsed(&self, id: u64) -> bool {
        self.collapsed_projects.contains(&id)
    }

    /// Returns `true` when the `"Sessions without a project"` group is currently collapsed.
    #[must_use]
    pub const fn is_global_collapsed(&self) -> bool {
        self.global_collapsed
    }

    /// Returns the number of collapsed project IDs currently retained by the panel.
    #[must_use]
    pub fn collapsed_project_count(&self) -> usize {
        self.collapsed_projects.len()
    }

    /// Requests keyboard focus on the active (or first) sidebar item on the next frame.
    pub fn request_focus(&mut self, ctx: &egui::Context) {
        self.focus_requested = true;
        if let Some(id) = self.list_root_id {
            ctx.memory_mut(|m| m.request_focus(id));
        }
        ctx.request_repaint();
    }

    fn build_nav_items(&self, input: &ProjectPanelInput<'_>) -> Vec<NavItem> {
        let mut out = Vec::new();
        for project in input.projects {
            out.push(NavItem::Project(project.id));
            if !self.collapsed_projects.contains(&project.id) {
                for session in &project.sessions {
                    out.push(NavItem::Session {
                        project: Some(project.id),
                        id: session.id,
                    });
                }
            }
        }
        if !input.global_sessions.is_empty() {
            out.push(NavItem::GlobalHeader);
            if !self.global_collapsed {
                for session in input.global_sessions {
                    out.push(NavItem::Session {
                        project: None,
                        id: session.id,
                    });
                }
            }
        }
        out
    }

    fn default_nav_index(
        &self,
        nav_items: &[NavItem],
        input: &ProjectPanelInput<'_>,
    ) -> Option<usize> {
        if nav_items.is_empty() {
            return None;
        }
        if let Some(sid) = input.active_session
            && let Some(idx) = nav_items.iter().position(|item| {
                matches!(
                    item,
                    NavItem::Session { project, id }
                        if *project == input.active_project && *id == sid
                )
            })
        {
            return Some(idx);
        }
        if let Some(pid) = input.active_project
            && let Some(idx) = nav_items
                .iter()
                .position(|item| *item == NavItem::Project(pid))
        {
            return Some(idx);
        }
        if input.active_project.is_none()
            && let Some(idx) = nav_items
                .iter()
                .position(|item| *item == NavItem::GlobalHeader)
        {
            return Some(idx);
        }
        Some(0)
    }

    fn recover_deleted_focus(
        &self,
        deleted: NavItem,
        nav_items: &[NavItem],
        input: &ProjectPanelInput<'_>,
    ) -> Option<NavItem> {
        if nav_items.is_empty() {
            return None;
        }
        if let Some(old_idx) = self.last_nav_items.iter().position(|it| *it == deleted) {
            if let NavItem::Session { project, .. } = deleted {
                if old_idx < nav_items.len()
                    && matches!(nav_items[old_idx], NavItem::Session { project: p, .. } if p == project)
                {
                    return Some(nav_items[old_idx]);
                }
                if old_idx > 0 {
                    let prev = nav_items[old_idx - 1];
                    let same_parent = matches!(prev, NavItem::Session { project: p, .. } if p == project)
                        || project.is_some_and(|pid| prev == NavItem::Project(pid))
                        || (project.is_none() && prev == NavItem::GlobalHeader);
                    if same_parent {
                        return Some(prev);
                    }
                }
            }
            return Some(nav_items[old_idx.min(nav_items.len() - 1)]);
        }
        self.default_nav_index(nav_items, input)
            .map(|idx| nav_items[idx])
    }

    fn list_a11y_label(item: Option<NavItem>, input: &ProjectPanelInput<'_>) -> String {
        let base = if input.french { "Projets" } else { "Projects" };
        let Some(nav) = item else {
            return base.to_owned();
        };
        let unnamed = if input.french {
            "(sans nom)"
        } else {
            "(unnamed)"
        };
        let global_title = if input.french {
            "Sessions sans projet"
        } else {
            "Sessions without a project"
        };
        let detail = match nav {
            NavItem::Project(pid) => input.projects.iter().find(|p| p.id == pid).map(|p| {
                if p.name.trim().is_empty() {
                    unnamed.to_owned()
                } else {
                    p.name.clone()
                }
            }),
            NavItem::GlobalHeader => Some(global_title.to_owned()),
            NavItem::Session {
                project: Some(pid),
                id,
            } => input.projects.iter().find(|p| p.id == pid).map(|p| {
                let pname = if p.name.trim().is_empty() {
                    unnamed
                } else {
                    p.name.as_str()
                };
                let stitle = p
                    .sessions
                    .iter()
                    .find(|s| s.id == id)
                    .map_or("Session", session_display_title);
                format!("{pname} / {stitle}")
            }),
            NavItem::Session { project: None, id } => input
                .global_sessions
                .iter()
                .find(|s| s.id == id)
                .map(|s| format!("{global_title} / {}", session_display_title(s))),
        };
        match detail {
            Some(d) => format!("{base} — {d}"),
            None => base.to_owned(),
        }
    }

    fn classify_focus(
        focused: Option<Id>,
        panel_id: Id,
        has_list_items: bool,
    ) -> Option<FocusOwner> {
        let fid = focused?;
        if fid == panel_id.with("search") {
            Some(FocusOwner::Search)
        } else if fid == panel_id.with("open-folder") {
            Some(FocusOwner::OpenFolder)
        } else if has_list_items && fid == panel_id.with("list") {
            Some(FocusOwner::List)
        } else {
            None
        }
    }

    fn handle_focused_keys(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        owner: FocusOwner,
        input: &ProjectPanelInput<'_>,
        nav_items: &[NavItem],
    ) -> (Option<ProjectAction>, bool) {
        let mut pressed_activate: Option<Key> = None;
        let held_before = self.held_activate_key;
        let (down, up, left, right) = ui.input_mut(|i| {
            for ev in &i.events {
                if let egui::Event::Key {
                    key: k @ (Key::Enter | Key::Space),
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = ev
                    && modifiers.matches_logically(Modifiers::NONE)
                    && held_before != Some(*k)
                {
                    pressed_activate = Some(*k);
                    break;
                }
            }
            let _ = i.consume_key(Modifiers::NONE, Key::Enter);
            let _ = i.consume_key(Modifiers::NONE, Key::Space);
            let down = i.consume_key(Modifiers::NONE, Key::ArrowDown);
            let up = i.consume_key(Modifiers::NONE, Key::ArrowUp);
            let left = i.consume_key(Modifiers::NONE, Key::ArrowLeft);
            let right = i.consume_key(Modifiers::NONE, Key::ArrowRight);
            (down, up, left, right)
        });

        if let Some(k) = pressed_activate {
            self.held_activate_key = Some(k);
        }

        let mut action = None;
        let mut scroll_to_focused = false;

        if pressed_activate.is_some() {
            match owner {
                FocusOwner::Search => action = Some(ProjectAction::Search),
                FocusOwner::OpenFolder => action = Some(ProjectAction::OpenFolder),
                FocusOwner::List => {
                    if self.focused_nav.is_none()
                        && let Some(idx) = self.default_nav_index(nav_items, input)
                    {
                        self.focused_nav = Some(nav_items[idx]);
                    }
                    if let Some(item) = self.focused_nav {
                        action = Some(item.activate_action());
                    }
                }
            }
        }

        if (down || up) && !nav_items.is_empty() {
            let len = nav_items.len() as isize;
            let step: isize = if down { 1 } else { -1 };
            let cur_idx = self
                .focused_nav
                .and_then(|cur| nav_items.iter().position(|it| *it == cur));
            let target_idx = match (owner, cur_idx) {
                (FocusOwner::List, Some(cur)) => (cur as isize + step).rem_euclid(len) as usize,
                _ => {
                    ui.ctx()
                        .memory_mut(|m| m.request_focus(panel_id.with("list")));
                    if let Some(def) = self.default_nav_index(nav_items, input) {
                        def
                    } else if down {
                        0
                    } else {
                        nav_items.len() - 1
                    }
                }
            };
            self.focused_nav = Some(nav_items[target_idx]);
            scroll_to_focused = true;
        } else if left {
            if owner == FocusOwner::List
                && let Some(item) = self.focused_nav
            {
                match item {
                    NavItem::Project(pid) => {
                        self.collapsed_projects.insert(pid);
                    }
                    NavItem::GlobalHeader => {
                        self.global_collapsed = true;
                    }
                    NavItem::Session {
                        project: Some(pid), ..
                    } => {
                        self.focused_nav = Some(NavItem::Project(pid));
                        scroll_to_focused = true;
                    }
                    NavItem::Session { project: None, .. } => {
                        self.focused_nav = Some(NavItem::GlobalHeader);
                        scroll_to_focused = true;
                    }
                }
            }
        } else if right
            && owner == FocusOwner::List
            && let Some(item) = self.focused_nav
        {
            match item {
                NavItem::Project(pid) => {
                    self.collapsed_projects.remove(&pid);
                }
                NavItem::GlobalHeader => {
                    self.global_collapsed = false;
                }
                NavItem::Session { .. } => {}
            }
        }

        (action, scroll_to_focused)
    }

    /// Renders the left project/session sidebar and returns any triggered [`ProjectAction`].
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        input: ProjectPanelInput<'_>,
    ) -> Option<ProjectAction> {
        self.collapsed_projects
            .retain(|id| input.projects.iter().any(|p| p.id == *id));
        if input.global_sessions.is_empty() {
            self.global_collapsed = false;
        }

        let panel_id = ui.id().with("tessel-project-panel");
        let list_root_id = panel_id.with("list");
        self.list_root_id = Some(list_root_id);

        let keyboard_blocked = !ui.is_enabled() || egui::Popup::is_any_open(ui.ctx());

        if let Some(held) = self.held_activate_key {
            let released = keyboard_blocked
                || !ui.input(|i| i.focused)
                || ui.input(|i| {
                    i.events.iter().any(|ev| {
                        matches!(
                            ev,
                            egui::Event::Key {
                                key,
                                pressed: false,
                                ..
                            } if *key == held
                        )
                    })
                });
            if released {
                self.held_activate_key = None;
            }
        }

        let pre_nav_items = self.build_nav_items(&input);
        let has_list_items = !pre_nav_items.is_empty();
        if let Some(cur) = self.focused_nav
            && !pre_nav_items.contains(&cur)
        {
            self.focused_nav = self.recover_deleted_focus(cur, &pre_nav_items, &input);
        }

        if !has_list_items
            && ui.is_enabled()
            && ui.ctx().memory(|m| m.focused()) == Some(list_root_id)
        {
            ui.ctx()
                .memory_mut(|m| m.request_focus(panel_id.with("open-folder")));
        }

        let focused_now = ui.ctx().memory(|m| m.focused());
        let focus_owner = if keyboard_blocked {
            if !ui.is_enabled() {
                self.focus_requested = false;
            }
            self.held_activate_key = None;
            None
        } else if self.focus_requested {
            self.focus_requested = false;
            if let Some(idx) = self.default_nav_index(&pre_nav_items, &input) {
                self.focused_nav = Some(pre_nav_items[idx]);
                ui.ctx().memory_mut(|m| m.request_focus(list_root_id));
                Some(FocusOwner::List)
            } else {
                let search_id = panel_id.with("search");
                ui.ctx().memory_mut(|m| m.request_focus(search_id));
                Some(FocusOwner::Search)
            }
        } else {
            let classified = Self::classify_focus(focused_now, panel_id, has_list_items);
            if classified == Some(FocusOwner::List) && self.focused_nav.is_none() {
                self.focused_nav = self
                    .default_nav_index(&pre_nav_items, &input)
                    .map(|idx| pre_nav_items[idx]);
            }
            if classified.is_none() {
                self.held_activate_key = None;
            }
            classified
        };

        let (mut action, scroll_to_focused) = if let Some(owner) = focus_owner {
            self.handle_focused_keys(ui, panel_id, owner, &input, &pre_nav_items)
        } else {
            (None, false)
        };

        ui.spacing_mut().item_spacing.y = 4.0_f32;

        if let Some(search_act) = self.render_search_button(ui, panel_id, input.french) {
            action = Some(search_act);
        }
        if let Some(header_act) = self.render_projects_header(ui, panel_id, input.french) {
            action = Some(header_act);
        }

        let list_resp = if has_list_items {
            let list_viewport = ui.available_rect_before_wrap();
            Some(ui.interact(
                list_viewport,
                list_root_id,
                Sense::focusable_noninteractive(),
            ))
        } else {
            None
        };

        let list_has_focus =
            has_list_items && ui.is_enabled() && ui.ctx().memory(|m| m.has_focus(list_root_id));
        let active_focused_nav = if list_has_focus {
            self.focused_nav
        } else {
            None
        };
        let lang = sidebar_lang(input.french);

        egui::ScrollArea::vertical()
            .id_salt(panel_id.with("scroll"))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0_f32;

                if input.projects.is_empty() && input.global_sessions.is_empty() {
                    ui.add_space(12.0_f32);
                    let empty_text = if input.french {
                        "Aucun projet ouvert pour l'instant."
                    } else {
                        "No project open yet."
                    };
                    ui.vertical_centered(|ui| {
                        ui.colored_label(ui.visuals().weak_text_color(), empty_text);
                    });
                    return;
                }

                for project in input.projects {
                    if let Some(proj_act) = self.render_project_group(
                        ui,
                        panel_id,
                        project,
                        &input,
                        lang,
                        active_focused_nav,
                        scroll_to_focused,
                    ) {
                        action = Some(proj_act);
                    }
                }

                if !input.global_sessions.is_empty() {
                    if !input.projects.is_empty() {
                        ui.add_space(2.0_f32);
                        ui.separator();
                        ui.add_space(2.0_f32);
                    }
                    if let Some(glob_act) = self.render_global_group(
                        ui,
                        panel_id,
                        &input,
                        lang,
                        active_focused_nav,
                        scroll_to_focused,
                    ) {
                        action = Some(glob_act);
                    }
                }
            });

        let post_nav_items = self.build_nav_items(&input);
        if let Some(list_resp) = list_resp {
            let a11y_item = self
                .focused_nav
                .filter(|item| post_nav_items.contains(item))
                .or_else(|| {
                    self.default_nav_index(&post_nav_items, &input)
                        .map(|idx| post_nav_items[idx])
                });
            let list_label = Self::list_a11y_label(a11y_item, &input);
            list_resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), &list_label)
            });
        }
        let end_focus = ui.ctx().memory(|m| m.focused());
        if !keyboard_blocked
            && Self::classify_focus(end_focus, panel_id, !post_nav_items.is_empty()).is_some()
            && let Some(fid) = end_focus
        {
            ui.ctx().memory_mut(|m| {
                m.set_focus_lock_filter(
                    fid,
                    egui::EventFilter {
                        tab: false,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: false,
                    },
                );
            });
        }
        self.last_nav_items = post_nav_items;

        action
    }

    fn render_search_button(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        french: bool,
    ) -> Option<ProjectAction> {
        let search_id = panel_id.with("search");
        let label = if french { "Rechercher" } else { "Search" };
        let shortcut_keys = chords_of("Ctrl+Shift+P", french);
        let chord = shortcut_keys.first().map_or(&[][..], Vec::as_slice);
        let chord_str = chord.join("+");
        let a11y_label = format!("{label} ({chord_str})");

        let full_w = ui.available_width().max(80.0_f32);
        let (_, outer_rect) = ui.allocate_space(Vec2::new(full_w, 32.0_f32));
        let btn_rect = outer_rect.shrink2(Vec2::new(2.0_f32, 1.0_f32));
        let resp = ui.interact(btn_rect, search_id, Sense::click());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &a11y_label)
        });

        let has_focus = ui.ctx().memory(|m| m.has_focus(search_id));
        let painter = ui.painter().with_clip_rect(btn_rect);
        let fg = ui.visuals().strong_text_color();
        let bg = if resp.hovered() || has_focus {
            fg.gamma_multiply(0.085_f32)
        } else {
            fg.gamma_multiply(0.05_f32)
        };
        let stroke = if has_focus {
            Stroke::new(1.0_f32, ui.visuals().selection.stroke.color)
        } else {
            Stroke::new(
                1.0_f32,
                ui.visuals()
                    .widgets
                    .noninteractive
                    .bg_stroke
                    .color
                    .gamma_multiply(0.55_f32),
            )
        };
        painter.rect(btn_rect, 6.0_f32, bg, stroke, StrokeKind::Inside);

        let icon_color = if resp.hovered() || has_focus {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        let icon_rect = Rect::from_center_size(
            Pos2::new(btn_rect.min.x + 15.0_f32, btn_rect.center().y),
            Vec2::splat(14.0_f32),
        );
        icons::paint(&painter, icon_rect, Icon::Search, icon_color);

        let mut text_right = btn_rect.max.x - 8.0_f32;
        if btn_rect.width() >= 188.0_f32 && !chord.is_empty() {
            let kbd_font = FontId::monospace(9.5_f32);
            let kbd_fg = if resp.hovered() || has_focus {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            };
            let kbd_bg = fg.gamma_multiply(0.075_f32);
            let kbd_stroke = Stroke::new(
                1.0_f32,
                ui.visuals()
                    .widgets
                    .noninteractive
                    .bg_stroke
                    .color
                    .gamma_multiply(0.75_f32),
            );
            let mut cursor_x = btn_rect.max.x - 6.0_f32;
            for &cap in chord.iter().rev() {
                let galley = painter.layout_no_wrap(cap.to_owned(), kbd_font.clone(), kbd_fg);
                let cap_w = (galley.size().x + 8.0_f32).max(16.0_f32);
                let cap_rect = Rect::from_center_size(
                    Pos2::new(cursor_x - cap_w * 0.5_f32, btn_rect.center().y),
                    Vec2::new(cap_w, 16.0_f32),
                );
                painter.rect(cap_rect, 4.0_f32, kbd_bg, kbd_stroke, StrokeKind::Inside);
                painter.galley(
                    Pos2::new(
                        cap_rect.center().x - galley.size().x * 0.5_f32,
                        cap_rect.center().y - galley.size().y * 0.5_f32,
                    ),
                    galley,
                    kbd_fg,
                );
                cursor_x = cap_rect.min.x - 2.0_f32;
            }
            text_right = cursor_x - 4.0_f32;
        }

        let label_rect = Rect::from_min_max(
            Pos2::new(icon_rect.max.x + 8.0_f32, btn_rect.min.y),
            Pos2::new(text_right.max(icon_rect.max.x + 12.0_f32), btn_rect.max.y),
        );
        let label_color = if resp.hovered() || has_focus {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color().gamma_multiply(0.82_f32)
        };
        paint_single_line_ellipsis(
            &painter,
            label_rect,
            label,
            FontId::proportional(12.5_f32),
            label_color,
        );

        let clicked = resp.clicked();
        resp.on_hover_text(a11y_label);
        if clicked {
            ui.ctx().memory_mut(|m| m.request_focus(search_id));
            Some(ProjectAction::Search)
        } else {
            None
        }
    }

    fn render_projects_header(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        french: bool,
    ) -> Option<ProjectAction> {
        let open_id = panel_id.with("open-folder");
        let title = if french { "Projets" } else { "Projects" };
        let add_label = if french {
            "Ajouter un projet"
        } else {
            "Add project"
        };

        let full_w = ui.available_width().max(80.0_f32);
        let (_, row_rect) = ui.allocate_space(Vec2::new(full_w, 28.0_f32));

        let btn_rect = Rect::from_center_size(
            Pos2::new(row_rect.max.x - 14.0_f32, row_rect.center().y),
            Vec2::splat(24.0_f32),
        );
        let title_rect = Rect::from_min_max(
            Pos2::new(row_rect.min.x + 6.0_f32, row_rect.min.y),
            Pos2::new(
                (btn_rect.min.x - 6.0_f32).max(row_rect.min.x + 12.0_f32),
                row_rect.max.y,
            ),
        );
        paint_single_line_ellipsis(
            ui.painter(),
            title_rect,
            title,
            FontId::proportional(12.0_f32),
            ui.visuals().weak_text_color(),
        );

        let resp = ui.interact(btn_rect, open_id, Sense::click());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), add_label)
        });
        let has_focus = ui.ctx().memory(|m| m.has_focus(open_id));
        let painter = ui.painter().with_clip_rect(btn_rect);
        if resp.hovered() || has_focus {
            let stroke = if has_focus {
                Stroke::new(1.0_f32, ui.visuals().selection.stroke.color)
            } else {
                Stroke::NONE
            };
            painter.rect(
                btn_rect,
                6.0_f32,
                ui.visuals().widgets.hovered.bg_fill,
                stroke,
                StrokeKind::Inside,
            );
        }
        let icon_color = if resp.hovered() || has_focus {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        let icon_rect = Rect::from_center_size(btn_rect.center(), Vec2::splat(14.5_f32));
        paint_folder_icon(&painter, icon_rect, icon_color, true);

        let clicked = resp.clicked();
        resp.on_hover_text(add_label);
        if clicked {
            ui.ctx().memory_mut(|m| m.request_focus(open_id));
            Some(ProjectAction::OpenFolder)
        } else {
            None
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_project_group(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        project: &ProjectEntry,
        input: &ProjectPanelInput<'_>,
        lang: SidebarLang,
        focused_nav: Option<NavItem>,
        scroll_to_focused: bool,
    ) -> Option<ProjectAction> {
        let collapsed = self.collapsed_projects.contains(&project.id);
        let is_active = input.active_project == Some(project.id);
        let status = group_status(&project.sessions);
        let show_path = !collapsed && !project.path.trim().is_empty();
        let show_sessions = !collapsed && !project.sessions.is_empty();

        let header_h = if show_path { 44.0_f32 } else { 28.0_f32 };
        let sessions_h = if show_sessions {
            (project.sessions.len() as f32) * 24.0_f32
                + (project.sessions.len().saturating_sub(1) as f32) * 2.0_f32
                + 6.0_f32
        } else {
            0.0_f32
        };
        let card_h = header_h + sessions_h + 4.0_f32;
        let full_w = ui.available_width().max(80.0_f32);
        let (_, outer_rect) = ui.allocate_space(Vec2::new(full_w, card_h));
        let card_rect = outer_rect.shrink2(Vec2::new(2.0_f32, 0.0_f32));

        let nav_item = NavItem::Project(project.id);
        let card_id = nav_item.widget_id(panel_id);
        let collapse_id = panel_id.with(("project-collapse", project.id));

        let top_band_rect = Rect::from_min_size(
            Pos2::new(card_rect.min.x, card_rect.min.y + 2.0_f32),
            Vec2::new(card_rect.width(), header_h),
        );
        let collapse_rect = Rect::from_center_size(
            Pos2::new(card_rect.max.x - 14.0_f32, card_rect.min.y + 16.0_f32),
            Vec2::splat(20.0_f32),
        );
        let card_click_rect = Rect::from_min_max(
            top_band_rect.min,
            Pos2::new(
                (collapse_rect.min.x - 2.0_f32).max(top_band_rect.min.x + 24.0_f32),
                top_band_rect.max.y,
            ),
        );

        let card_resp = ui.interact(card_click_rect, card_id, Sense::CLICK);
        let display_name = if project.name.trim().is_empty() {
            if input.french {
                "(sans nom)"
            } else {
                "(unnamed)"
            }
        } else {
            project.name.as_str()
        };
        card_resp.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                ui.is_enabled(),
                is_active,
                display_name,
            )
        });
        let card_focused = focused_nav == Some(nav_item);
        if scroll_to_focused && card_focused {
            card_resp.scroll_to_me(Some(egui::Align::Center));
        }

        let collapse_label = collapse_button_label(display_name, collapsed, input.french);
        let collapse_resp = ui.interact(collapse_rect, collapse_id, Sense::CLICK);
        collapse_resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &collapse_label)
        });

        // Card container surface (`.wtc-surface`).
        let painter = ui.painter().with_clip_rect(card_rect);
        let accent = ui.visuals().selection.stroke.color;
        let card_bg = if is_active {
            ui.visuals()
                .widgets
                .hovered
                .bg_fill
                .gamma_multiply(0.55_f32)
        } else if card_resp.hovered() {
            ui.visuals()
                .widgets
                .hovered
                .bg_fill
                .gamma_multiply(0.32_f32)
        } else {
            Color32::TRANSPARENT
        };
        let card_stroke = if card_focused {
            Stroke::new(1.0_f32, accent)
        } else if is_active {
            Stroke::new(
                1.0_f32,
                ui.visuals()
                    .widgets
                    .noninteractive
                    .bg_stroke
                    .color
                    .gamma_multiply(0.75_f32),
            )
        } else {
            Stroke::NONE
        };
        painter.rect(card_rect, 8.0_f32, card_bg, card_stroke, StrokeKind::Inside);

        if is_active {
            let bar_rect = Rect::from_min_size(
                Pos2::new(card_rect.min.x + 1.0_f32, card_rect.min.y + 6.0_f32),
                Vec2::new(2.5_f32, (card_rect.height() - 12.0_f32).max(12.0_f32)),
            );
            painter.rect_filled(bar_rect, 1.5_f32, accent);
        }

        let row1_y = card_rect.min.y + 16.0_f32;
        let status_rect = Rect::from_center_size(
            Pos2::new(card_rect.min.x + 13.0_f32, row1_y),
            Vec2::splat(14.0_f32),
        );
        paint_group_status(
            &painter,
            status_rect,
            status,
            ui.visuals().weak_text_color(),
        );

        let folder_color = if is_active {
            accent
        } else {
            ui.visuals().weak_text_color()
        };
        let folder_rect = Rect::from_center_size(
            Pos2::new(card_rect.min.x + 29.0_f32, row1_y),
            Vec2::splat(14.0_f32),
        );
        paint_folder_icon(&painter, folder_rect, folder_color, false);

        // Optional session count pill before collapse chevron.
        let mut title_right = collapse_rect.min.x - 4.0_f32;
        if !project.sessions.is_empty() && card_rect.width() >= 150.0_f32 {
            let count_str = project.sessions.len().to_string();
            let count_font = FontId::proportional(10.0_f32);
            let count_fg = ui.visuals().weak_text_color();
            let galley = painter.layout_no_wrap(count_str, count_font, count_fg);
            let pill_w = (galley.size().x + 10.0_f32).max(18.0_f32);
            let pill_rect = Rect::from_center_size(
                Pos2::new(title_right - pill_w * 0.5_f32, row1_y),
                Vec2::new(pill_w, 16.0_f32),
            );
            painter.rect(
                pill_rect,
                8.0_f32,
                ui.visuals().strong_text_color().gamma_multiply(0.06_f32),
                Stroke::new(
                    1.0_f32,
                    ui.visuals()
                        .widgets
                        .noninteractive
                        .bg_stroke
                        .color
                        .gamma_multiply(0.6_f32),
                ),
                StrokeKind::Inside,
            );
            painter.galley(
                Pos2::new(
                    pill_rect.center().x - galley.size().x * 0.5_f32,
                    pill_rect.center().y - galley.size().y * 0.5_f32,
                ),
                galley,
                count_fg,
            );
            title_right = pill_rect.min.x - 4.0_f32;
        }

        let name_color = if status == WorktreeCardStatus::Sleeping && !is_active {
            ui.visuals().text_color().gamma_multiply(0.62_f32)
        } else {
            ui.visuals().strong_text_color()
        };
        let name_rect = Rect::from_min_max(
            Pos2::new(folder_rect.max.x + 6.0_f32, card_rect.min.y + 4.0_f32),
            Pos2::new(
                title_right.max(folder_rect.max.x + 16.0_f32),
                card_rect.min.y + 28.0_f32,
            ),
        );
        paint_single_line_ellipsis(
            &painter,
            name_rect,
            display_name,
            FontId::proportional(13.0_f32),
            name_color,
        );

        // Collapse chevron button visuals.
        if collapse_resp.hovered() {
            painter.rect(
                collapse_rect,
                5.0_f32,
                ui.visuals().widgets.hovered.bg_fill,
                Stroke::NONE,
                StrokeKind::Inside,
            );
        }
        let chev_color = if collapse_resp.hovered() {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        let chev_icon_rect = Rect::from_center_size(collapse_rect.center(), Vec2::splat(13.0_f32));
        if collapsed {
            paint_chevron_right(&painter, chev_icon_rect, chev_color);
        } else {
            icons::paint(&painter, chev_icon_rect, Icon::ChevronDown, chev_color);
        }

        if show_path {
            let path_rect = Rect::from_min_max(
                Pos2::new(folder_rect.min.x, card_rect.min.y + 28.0_f32),
                Pos2::new(
                    (card_rect.max.x - 8.0_f32).max(folder_rect.min.x + 16.0_f32),
                    card_rect.min.y + 44.0_f32,
                ),
            );
            paint_single_line_ellipsis(
                &painter,
                path_rect,
                project.path.trim(),
                FontId::monospace(10.5_f32),
                ui.visuals().weak_text_color(),
            );
        }

        let mut action = None;
        let list_id = panel_id.with("list");
        let collapse_clicked = collapse_resp.clicked();
        collapse_resp.on_hover_text(&collapse_label);
        if collapse_clicked {
            if !self.collapsed_projects.remove(&project.id) {
                self.collapsed_projects.insert(project.id);
            }
            self.focused_nav = Some(nav_item);
            ui.ctx().memory_mut(|m| m.request_focus(list_id));
        }

        let card_hover = if project.path.trim().is_empty() {
            format!("{display_name}\n{}", status.label(lang))
        } else {
            format!(
                "{display_name}\n{}\n{}",
                project.path.trim(),
                status.label(lang)
            )
        };
        let card_clicked = card_resp.clicked();
        card_resp.on_hover_text(card_hover);
        if card_clicked && !collapse_clicked {
            self.focused_nav = Some(nav_item);
            ui.ctx().memory_mut(|m| m.request_focus(list_id));
            action = Some(ProjectAction::SelectProject(Some(project.id)));
        }

        if show_sessions {
            let mut row_y = card_rect.min.y + 2.0_f32 + header_h + 3.0_f32;
            let row_min_x = card_rect.min.x + 6.0_f32;
            let row_max_x = (card_rect.max.x - 6.0_f32).max(row_min_x + 32.0_f32);
            for session in &project.sessions {
                let row_rect = Rect::from_min_max(
                    Pos2::new(row_min_x, row_y),
                    Pos2::new(row_max_x, row_y + 24.0_f32),
                );
                if let Some(sess_act) = self.render_session_row(
                    ui,
                    panel_id,
                    row_rect,
                    Some(project.id),
                    session,
                    is_active && input.active_session == Some(session.id),
                    lang,
                    focused_nav,
                    scroll_to_focused,
                ) {
                    action = Some(sess_act);
                }
                row_y += 26.0_f32;
            }
        }

        action
    }

    fn render_global_group(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        input: &ProjectPanelInput<'_>,
        lang: SidebarLang,
        focused_nav: Option<NavItem>,
        scroll_to_focused: bool,
    ) -> Option<ProjectAction> {
        let collapsed = self.global_collapsed;
        let is_active = input.active_project.is_none();
        let status = group_status(input.global_sessions);
        let show_sessions = !collapsed && !input.global_sessions.is_empty();

        let header_h = 28.0_f32;
        let sessions_h = if show_sessions {
            (input.global_sessions.len() as f32) * 24.0_f32
                + (input.global_sessions.len().saturating_sub(1) as f32) * 2.0_f32
                + 6.0_f32
        } else {
            0.0_f32
        };
        let card_h = header_h + sessions_h + 4.0_f32;
        let full_w = ui.available_width().max(80.0_f32);
        let (_, outer_rect) = ui.allocate_space(Vec2::new(full_w, card_h));
        let card_rect = outer_rect.shrink2(Vec2::new(2.0_f32, 0.0_f32));

        let nav_item = NavItem::GlobalHeader;
        let card_id = nav_item.widget_id(panel_id);
        let collapse_id = panel_id.with("global-collapse");

        let top_band_rect = Rect::from_min_size(
            Pos2::new(card_rect.min.x, card_rect.min.y + 2.0_f32),
            Vec2::new(card_rect.width(), header_h),
        );
        let collapse_rect = Rect::from_center_size(
            Pos2::new(card_rect.max.x - 14.0_f32, card_rect.min.y + 16.0_f32),
            Vec2::splat(20.0_f32),
        );
        let card_click_rect = Rect::from_min_max(
            top_band_rect.min,
            Pos2::new(
                (collapse_rect.min.x - 2.0_f32).max(top_band_rect.min.x + 24.0_f32),
                top_band_rect.max.y,
            ),
        );

        let title = if input.french {
            "Sessions sans projet"
        } else {
            "Sessions without a project"
        };

        let card_resp = ui.interact(card_click_rect, card_id, Sense::CLICK);
        card_resp.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                ui.is_enabled(),
                is_active,
                title,
            )
        });
        let card_focused = focused_nav == Some(nav_item);
        if scroll_to_focused && card_focused {
            card_resp.scroll_to_me(Some(egui::Align::Center));
        }

        let collapse_label = collapse_button_label(title, collapsed, input.french);
        let collapse_resp = ui.interact(collapse_rect, collapse_id, Sense::CLICK);
        collapse_resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &collapse_label)
        });

        let painter = ui.painter().with_clip_rect(card_rect);
        let accent = ui.visuals().selection.stroke.color;
        let card_bg = if is_active {
            ui.visuals()
                .widgets
                .hovered
                .bg_fill
                .gamma_multiply(0.55_f32)
        } else if card_resp.hovered() {
            ui.visuals()
                .widgets
                .hovered
                .bg_fill
                .gamma_multiply(0.32_f32)
        } else {
            Color32::TRANSPARENT
        };
        let card_stroke = if card_focused {
            Stroke::new(1.0_f32, accent)
        } else if is_active {
            Stroke::new(
                1.0_f32,
                ui.visuals()
                    .widgets
                    .noninteractive
                    .bg_stroke
                    .color
                    .gamma_multiply(0.75_f32),
            )
        } else {
            Stroke::NONE
        };
        painter.rect(card_rect, 8.0_f32, card_bg, card_stroke, StrokeKind::Inside);

        if is_active {
            let bar_rect = Rect::from_min_size(
                Pos2::new(card_rect.min.x + 1.0_f32, card_rect.min.y + 6.0_f32),
                Vec2::new(2.5_f32, (card_rect.height() - 12.0_f32).max(12.0_f32)),
            );
            painter.rect_filled(bar_rect, 1.5_f32, accent);
        }

        let row1_y = card_rect.min.y + 16.0_f32;
        let status_rect = Rect::from_center_size(
            Pos2::new(card_rect.min.x + 13.0_f32, row1_y),
            Vec2::splat(14.0_f32),
        );
        paint_group_status(
            &painter,
            status_rect,
            status,
            ui.visuals().weak_text_color(),
        );

        let term_icon_rect = Rect::from_center_size(
            Pos2::new(card_rect.min.x + 29.0_f32, row1_y),
            Vec2::splat(13.5_f32),
        );
        icons::paint(
            &painter,
            term_icon_rect,
            Icon::Terminal,
            if is_active {
                accent
            } else {
                ui.visuals().weak_text_color()
            },
        );

        let mut title_right = collapse_rect.min.x - 4.0_f32;
        if card_rect.width() >= 150.0_f32 {
            let count_str = input.global_sessions.len().to_string();
            let count_font = FontId::proportional(10.0_f32);
            let count_fg = ui.visuals().weak_text_color();
            let galley = painter.layout_no_wrap(count_str, count_font, count_fg);
            let pill_w = (galley.size().x + 10.0_f32).max(18.0_f32);
            let pill_rect = Rect::from_center_size(
                Pos2::new(title_right - pill_w * 0.5_f32, row1_y),
                Vec2::new(pill_w, 16.0_f32),
            );
            painter.rect(
                pill_rect,
                8.0_f32,
                ui.visuals().strong_text_color().gamma_multiply(0.06_f32),
                Stroke::new(
                    1.0_f32,
                    ui.visuals()
                        .widgets
                        .noninteractive
                        .bg_stroke
                        .color
                        .gamma_multiply(0.6_f32),
                ),
                StrokeKind::Inside,
            );
            painter.galley(
                Pos2::new(
                    pill_rect.center().x - galley.size().x * 0.5_f32,
                    pill_rect.center().y - galley.size().y * 0.5_f32,
                ),
                galley,
                count_fg,
            );
            title_right = pill_rect.min.x - 4.0_f32;
        }

        let title_rect = Rect::from_min_max(
            Pos2::new(term_icon_rect.max.x + 6.0_f32, card_rect.min.y + 4.0_f32),
            Pos2::new(
                title_right.max(term_icon_rect.max.x + 16.0_f32),
                card_rect.min.y + 28.0_f32,
            ),
        );
        paint_single_line_ellipsis(
            &painter,
            title_rect,
            title,
            FontId::proportional(12.5_f32),
            ui.visuals().strong_text_color(),
        );

        if collapse_resp.hovered() {
            painter.rect(
                collapse_rect,
                5.0_f32,
                ui.visuals().widgets.hovered.bg_fill,
                Stroke::NONE,
                StrokeKind::Inside,
            );
        }
        let chev_color = if collapse_resp.hovered() {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        let chev_icon_rect = Rect::from_center_size(collapse_rect.center(), Vec2::splat(13.0_f32));
        if collapsed {
            paint_chevron_right(&painter, chev_icon_rect, chev_color);
        } else {
            icons::paint(&painter, chev_icon_rect, Icon::ChevronDown, chev_color);
        }

        let mut action = None;
        let list_id = panel_id.with("list");
        let collapse_clicked = collapse_resp.clicked();
        collapse_resp.on_hover_text(&collapse_label);
        if collapse_clicked {
            self.global_collapsed = !self.global_collapsed;
            self.focused_nav = Some(nav_item);
            ui.ctx().memory_mut(|m| m.request_focus(list_id));
        }

        let card_clicked = card_resp.clicked();
        card_resp.on_hover_text(format!("{title}\n{}", status.label(lang)));
        if card_clicked && !collapse_clicked {
            self.focused_nav = Some(nav_item);
            ui.ctx().memory_mut(|m| m.request_focus(list_id));
            action = Some(ProjectAction::SelectProject(None));
        }

        if show_sessions {
            let mut row_y = card_rect.min.y + 2.0_f32 + header_h + 3.0_f32;
            let row_min_x = card_rect.min.x + 6.0_f32;
            let row_max_x = (card_rect.max.x - 6.0_f32).max(row_min_x + 32.0_f32);
            for session in input.global_sessions {
                let row_rect = Rect::from_min_max(
                    Pos2::new(row_min_x, row_y),
                    Pos2::new(row_max_x, row_y + 24.0_f32),
                );
                if let Some(sess_act) = self.render_session_row(
                    ui,
                    panel_id,
                    row_rect,
                    None,
                    session,
                    is_active && input.active_session == Some(session.id),
                    lang,
                    focused_nav,
                    scroll_to_focused,
                ) {
                    action = Some(sess_act);
                }
                row_y += 26.0_f32;
            }
        }

        action
    }

    #[allow(clippy::too_many_arguments)]
    fn render_session_row(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        row_rect: Rect,
        project: Option<u64>,
        session: &SessionEntry,
        is_selected: bool,
        lang: SidebarLang,
        focused_nav: Option<NavItem>,
        scroll_to_focused: bool,
    ) -> Option<ProjectAction> {
        let nav_item = NavItem::Session {
            project,
            id: session.id,
        };
        let row_id = nav_item.widget_id(panel_id);
        let display_title = session_display_title(session);
        let resp = ui.interact(row_rect, row_id, Sense::CLICK);
        resp.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                ui.is_enabled(),
                is_selected,
                display_title,
            )
        });
        let has_focus = focused_nav == Some(nav_item);
        if scroll_to_focused && has_focus {
            resp.scroll_to_me(Some(egui::Align::Center));
        }
        let painter = ui.painter().with_clip_rect(row_rect);
        let accent = ui.visuals().selection.stroke.color;

        if is_selected || resp.hovered() || has_focus {
            let bg = if is_selected {
                ui.visuals()
                    .widgets
                    .hovered
                    .bg_fill
                    .gamma_multiply(0.85_f32)
            } else {
                ui.visuals()
                    .widgets
                    .hovered
                    .bg_fill
                    .gamma_multiply(0.40_f32)
            };
            let stroke = if has_focus {
                Stroke::new(1.0_f32, accent)
            } else {
                Stroke::NONE
            };
            painter.rect(row_rect, 4.0_f32, bg, stroke, StrokeKind::Inside);
        }

        let dot_rect = Rect::from_center_size(
            Pos2::new(row_rect.min.x + 10.0_f32, row_rect.center().y),
            Vec2::splat(10.0_f32),
        );
        paint_session_dot(
            &painter,
            dot_rect,
            session.sleeping,
            ui.visuals().weak_text_color(),
        );

        let icon_color = if session.sleeping {
            ui.visuals().weak_text_color().gamma_multiply(0.65_f32)
        } else if is_selected || resp.hovered() {
            match session.kind {
                SessionKind::Shell => ui.visuals().strong_text_color(),
                SessionKind::Agent | SessionKind::Chat => accent,
            }
        } else {
            match session.kind {
                SessionKind::Shell => ui.visuals().weak_text_color(),
                SessionKind::Agent | SessionKind::Chat => accent.gamma_multiply(0.85_f32),
            }
        };
        let icon_rect = Rect::from_center_size(
            Pos2::new(row_rect.min.x + 25.0_f32, row_rect.center().y),
            Vec2::splat(13.0_f32),
        );
        match session.kind {
            SessionKind::Shell | SessionKind::Agent => {
                icons::paint(&painter, icon_rect, Icon::Terminal, icon_color);
            }
            SessionKind::Chat => {
                paint_chat_icon(&painter, icon_rect, icon_color);
            }
        }

        let right_tag: Option<&str> = if session.sleeping {
            Some(WorktreeCardStatus::Sleeping.label(lang))
        } else if session.kind == SessionKind::Chat {
            Some("chat")
        } else {
            None
        };

        let mut text_right = row_rect.max.x - 6.0_f32;
        if let Some(tag) = right_tag
            && row_rect.width() >= 148.0_f32
        {
            let tag_font = FontId::proportional(10.0_f32);
            let tag_fg = ui.visuals().weak_text_color();
            let galley = painter.layout_no_wrap(tag.to_owned(), tag_font, tag_fg);
            let max_tag_w = (row_rect.width() * 0.34_f32).max(28.0_f32);
            if galley.size().x <= max_tag_w {
                let tag_x = row_rect.max.x - 6.0_f32 - galley.size().x;
                painter.galley(
                    Pos2::new(tag_x, row_rect.center().y - galley.size().y * 0.5_f32),
                    galley,
                    tag_fg,
                );
                text_right = tag_x - 6.0_f32;
            }
        }

        let title_color = if session.sleeping && !is_selected {
            ui.visuals().weak_text_color().gamma_multiply(0.75_f32)
        } else if is_selected || resp.hovered() || has_focus {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color().gamma_multiply(0.88_f32)
        };
        let title_rect = Rect::from_min_max(
            Pos2::new(icon_rect.max.x + 6.0_f32, row_rect.min.y),
            Pos2::new(text_right.max(icon_rect.max.x + 14.0_f32), row_rect.max.y),
        );
        paint_single_line_ellipsis(
            &painter,
            title_rect,
            display_title,
            FontId::proportional(11.5_f32),
            title_color,
        );

        let hover_text = session_hover_text(session, lang);
        let clicked = resp.clicked();
        resp.on_hover_text(hover_text);
        if clicked {
            self.focused_nav = Some(nav_item);
            ui.ctx()
                .memory_mut(|m| m.request_focus(panel_id.with("list")));
            Some(ProjectAction::SelectSession {
                project,
                id: session.id,
            })
        } else {
            None
        }
    }
}
