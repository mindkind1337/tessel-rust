//! Native `egui` terminal & AI agent launcher menu (`LaunchMenu.vue` parity).
//!
//! Composes directly with [`crate::finder_model`] (`LaunchMenuAction`, `LaunchMenuItem`,
//! `LaunchPlacement`, `LauncherLang`, `where_text`) and [`crate::icons`] (`Icon::Terminal`)
//! with **zero** process spawning and **zero** filesystem or environment I/O inside the widget:
//! - Renders only the caller-injected [`LauncherEntry`] catalog grouped into `Terminals`,
//!   `Native chats`, and `AI agents` (plus `Browser` if injected), never advertising unimplemented
//!   worktree, multi-account, Live Share host, or agent-installer controls.
//! - Renders the segmented placement selector (`Auto` / `Left` / `Right` / `Below`) with the exact
//!   `LaunchMenu.vue` `16×16` shaded-split vector geometry and localized `where_text` summary.
//! - Supports `↑` / `↓` / `Enter` / `Esc` keyboard navigation and pointer clicks, strictly skipping
//!   unavailable (`available == false`) entries and clamping selection safely when the catalog
//!   changes between frames.

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Vec2,
};

use crate::finder_model::{
    LaunchMenuAction, LaunchMenuItem, LaunchPlacement, LauncherLang, where_text,
};
use crate::icons::{self, Icon};

/// Single launchable or disabled catalog entry supplied by the application shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherEntry {
    pub item: LaunchMenuItem,
    pub available: bool,
    pub reason: Option<String>,
}

/// Frame input for [`LauncherPanel::show`].
#[derive(Debug, Clone, Copy)]
pub struct LauncherInput<'a> {
    pub entries: &'a [LauncherEntry],
    pub placements: &'a [LaunchPlacement],
    pub target_title: Option<&'a str>,
    pub lang: LauncherLang,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionKind {
    Terminals,
    NativeChats,
    Agents,
    Browser,
}

impl SectionKind {
    const fn of(item: &LaunchMenuItem) -> Self {
        match item {
            LaunchMenuItem::Shell { .. } => Self::Terminals,
            LaunchMenuItem::ChatAgent { .. } => Self::NativeChats,
            LaunchMenuItem::Agent { .. } | LaunchMenuItem::InstallAgent { .. } => Self::Agents,
            LaunchMenuItem::Browser => Self::Browser,
        }
    }

    const fn heading(self, lang: LauncherLang) -> Option<&'static str> {
        match (self, lang) {
            (Self::Terminals, LauncherLang::En) => Some("Terminals"),
            (Self::Terminals, LauncherLang::Fr) => Some("Terminaux"),
            (Self::NativeChats, LauncherLang::En) => Some("Native chats"),
            (Self::NativeChats, LauncherLang::Fr) => Some("Chats natifs"),
            (Self::Agents, LauncherLang::En) => Some("AI agents"),
            (Self::Agents, LauncherLang::Fr) => Some("Agents IA"),
            (Self::Browser, _) => None,
        }
    }
}

fn entry_display_name(item: &LaunchMenuItem, lang: LauncherLang) -> &str {
    match item {
        LaunchMenuItem::Shell { name, .. }
        | LaunchMenuItem::Agent { name, .. }
        | LaunchMenuItem::InstallAgent { name, .. }
        | LaunchMenuItem::ChatAgent { name, .. } => name.as_str(),
        LaunchMenuItem::Browser => match lang {
            LauncherLang::En => "Browser",
            LauncherLang::Fr => "Navigateur",
        },
    }
}

const fn placement_tooltip(placement: LaunchPlacement, lang: LauncherLang) -> &'static str {
    match (placement, lang) {
        (LaunchPlacement::Auto, LauncherLang::En) => "Into the grid with the other panes",
        (LaunchPlacement::Auto, LauncherLang::Fr) => "Dans la grille avec les autres panneaux",
        (LaunchPlacement::Left, LauncherLang::En) => "Split to the left",
        (LaunchPlacement::Left, LauncherLang::Fr) => "Diviser vers la gauche",
        (LaunchPlacement::Right, LauncherLang::En) => "Split to the right",
        (LaunchPlacement::Right, LauncherLang::Fr) => "Diviser vers la droite",
        (LaunchPlacement::Down, LauncherLang::En) => "Split below",
        (LaunchPlacement::Down, LauncherLang::Fr) => "Diviser vers le bas",
    }
}

/// Paints the exact `LaunchMenu.vue` (`62..95`, `237..241`) `16×16` placement icon inside `rect`,
/// preserving a centered `1:1` square aspect ratio:
/// - Outer pane frame `x=1.5, y=2.5, w=13, h=11, rx=2, stroke-width=1.3`
/// - `Auto`: `2×2` grid lines `M8 2.5v11` + `M1.5 8h13` (no shaded half)
/// - `Left`: shaded left half `(2.1, 3.1, 5.9, 9.8)` (`0.35` alpha) + vertical split `M8 2.5v11`
/// - `Right`: shaded right half `(8.0, 3.1, 5.9, 9.8)` (`0.35` alpha) + vertical split `M8 2.5v11`
/// - `Down`: shaded bottom half `(2.1, 8.0, 11.8, 4.9)` (`0.35` alpha) + horizontal split `M1.5 8h13`
pub fn paint_placement_icon(
    painter: &egui::Painter,
    rect: Rect,
    placement: LaunchPlacement,
    color: Color32,
) {
    let size = rect.width().min(rect.height());
    if !size.is_finite() || size <= 0.0_f32 {
        return;
    }
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(size));
    let scale = size / 16.0_f32;
    let map = |x: f32, y: f32| -> Pos2 { Pos2::new(sq.min.x + x * scale, sq.min.y + y * scale) };
    let map_rect = |x: f32, y: f32, w: f32, h: f32| -> Rect {
        Rect::from_min_max(map(x, y), map(x + w, y + h))
    };

    let shade_spec = match placement {
        LaunchPlacement::Auto => None,
        LaunchPlacement::Left => Some((2.1_f32, 3.1_f32, 5.9_f32, 9.8_f32)),
        LaunchPlacement::Right => Some((8.0_f32, 3.1_f32, 5.9_f32, 9.8_f32)),
        LaunchPlacement::Down => Some((2.1_f32, 8.0_f32, 11.8_f32, 4.9_f32)),
    };
    if let Some((sx, sy, sw, sh)) = shade_spec {
        painter.rect_filled(
            map_rect(sx, sy, sw, sh),
            1.0_f32 * scale,
            color.gamma_multiply(0.35_f32),
        );
    }

    let stroke = Stroke::new((1.3_f32 * scale).max(0.75_f32), color);
    painter.rect_stroke(
        map_rect(1.5_f32, 2.5_f32, 13.0_f32, 11.0_f32),
        2.0_f32 * scale,
        stroke,
        StrokeKind::Middle,
    );

    match placement {
        LaunchPlacement::Auto => {
            painter.line_segment([map(8.0_f32, 2.5_f32), map(8.0_f32, 13.5_f32)], stroke);
            painter.line_segment([map(1.5_f32, 8.0_f32), map(14.5_f32, 8.0_f32)], stroke);
        }
        LaunchPlacement::Left | LaunchPlacement::Right => {
            painter.line_segment([map(8.0_f32, 2.5_f32), map(8.0_f32, 13.5_f32)], stroke);
        }
        LaunchPlacement::Down => {
            painter.line_segment([map(1.5_f32, 8.0_f32), map(14.5_f32, 8.0_f32)], stroke);
        }
    }
}

fn paint_chat_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let size = rect.width().min(rect.height());
    if !size.is_finite() || size <= 0.0_f32 {
        return;
    }
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(size));
    let scale = size / 16.0_f32;
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

fn paint_browser_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let size = rect.width().min(rect.height());
    if !size.is_finite() || size <= 0.0_f32 {
        return;
    }
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(size));
    let scale = size / 16.0_f32;
    let map = |x: f32, y: f32| -> Pos2 { Pos2::new(sq.min.x + x * scale, sq.min.y + y * scale) };
    let outer = Stroke::new((1.3_f32 * scale).max(0.75_f32), color);
    let inner = Stroke::new((1.1_f32 * scale).max(0.70_f32), color);
    painter.circle_stroke(map(8.0_f32, 8.0_f32), 6.5_f32 * scale, outer);
    painter.line_segment([map(1.5_f32, 8.0_f32), map(14.5_f32, 8.0_f32)], inner);
    for sign in [-1.0_f32, 1.0_f32] {
        let mut meridian = Vec::with_capacity(9);
        for step in 0..=8 {
            let t = (step as f32) / 8.0_f32;
            let angle = t * std::f32::consts::PI;
            let x = 8.0_f32 + sign * 2.6_f32 * angle.sin();
            let y = 1.5_f32 + 13.0_f32 * t;
            meridian.push(map(x, y));
        }
        painter.add(egui::epaint::PathShape::line(meridian, inner));
    }
}

fn paint_entry_icon(
    painter: &egui::Painter,
    rect: Rect,
    item: &LaunchMenuItem,
    base_color: Color32,
    accent_color: Color32,
    available: bool,
) {
    match item {
        LaunchMenuItem::Shell { .. }
        | LaunchMenuItem::Agent { .. }
        | LaunchMenuItem::InstallAgent { .. } => {
            icons::paint(painter, rect, Icon::Terminal, base_color);
        }
        LaunchMenuItem::ChatAgent { id, .. } => {
            let tint = if !available {
                base_color
            } else if id.eq_ignore_ascii_case("claude") {
                Color32::from_rgb(0xd9, 0x77, 0x57)
            } else {
                accent_color
            };
            paint_chat_icon(painter, rect, tint);
        }
        LaunchMenuItem::Browser => {
            paint_browser_icon(painter, rect, base_color);
        }
    }
}

fn paint_single_line_ellipsis(
    painter: &egui::Painter,
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

fn render_keycap(ui: &mut egui::Ui, key: &str) {
    let visuals = ui.visuals();
    let bg = visuals.widgets.noninteractive.bg_fill;
    let stroke = visuals.widgets.noninteractive.bg_stroke;
    let text_color = visuals.strong_text_color();
    egui::Frame::new()
        .fill(bg)
        .stroke(stroke)
        .corner_radius(4.0_f32)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(key)
                        .monospace()
                        .size(10.5_f32)
                        .color(text_color),
                )
                .selectable(false)
                .extend(),
            );
        });
}

fn visual_entry_order(entries: &[LauncherEntry]) -> Vec<usize> {
    let mut order = Vec::with_capacity(entries.len());
    for section in [
        SectionKind::Terminals,
        SectionKind::NativeChats,
        SectionKind::Agents,
        SectionKind::Browser,
    ] {
        for (idx, entry) in entries.iter().enumerate() {
            if SectionKind::of(&entry.item) == section {
                order.push(idx);
            }
        }
    }
    order
}

/// Native `egui` launcher menu widget (`LaunchMenu.vue`).
#[derive(Debug, Clone)]
pub struct LauncherPanel {
    selected_slot: Option<usize>,
    selected_entry_idx: Option<usize>,
    selected_item: Option<LaunchMenuItem>,
    picked_placement: Option<LaunchPlacement>,
    last_side: LaunchPlacement,
}

impl Default for LauncherPanel {
    fn default() -> Self {
        Self {
            selected_slot: None,
            selected_entry_idx: None,
            selected_item: None,
            picked_placement: None,
            last_side: LaunchPlacement::Right,
        }
    }
}

impl LauncherPanel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets per-open selection and placement override while preserving the last non-`Auto` side
    /// (`LaunchMenu.vue:102-107`).
    pub fn reset(&mut self) {
        self.selected_slot = None;
        self.selected_entry_idx = None;
        self.selected_item = None;
        self.picked_placement = None;
    }

    /// Returns the currently selected index into the last rendered `LauncherInput::entries` slice,
    /// or `None` if no selectable (`available == true`) entry exists.
    #[must_use]
    pub const fn selected_entry_index(&self) -> Option<usize> {
        self.selected_entry_idx
    }

    /// Resolves the active [`LaunchPlacement`] against the caller's `placements` slice.
    #[must_use]
    pub fn effective_placement(&self, placements: &[LaunchPlacement]) -> LaunchPlacement {
        if placements.is_empty() {
            return LaunchPlacement::Auto;
        }
        if let Some(picked) = self.picked_placement
            && placements.contains(&picked)
        {
            return picked;
        }
        if placements.contains(&LaunchPlacement::Auto) {
            LaunchPlacement::Auto
        } else if placements.contains(&self.last_side) {
            self.last_side
        } else {
            placements[0]
        }
    }

    fn sync_selection(&mut self, visual_order: &[usize], entries: &[LauncherEntry]) {
        let len = visual_order.len();
        let any_available = visual_order.iter().any(|&idx| entries[idx].available);
        if len == 0 || !any_available {
            self.selected_slot = None;
            self.selected_entry_idx = None;
            self.selected_item = None;
            return;
        }

        let resolved_slot = if self.selected_slot.is_none() {
            (0..len).find(|&slot| entries[visual_order[slot]].available)
        } else if let Some(prev_item) = &self.selected_item
            && let Some(slot) = (0..len).find(|&slot| {
                let e = &entries[visual_order[slot]];
                e.available && e.item == *prev_item
            })
        {
            Some(slot)
        } else {
            let clamped = self.selected_slot.unwrap_or(0).min(len - 1);
            (clamped..len)
                .chain(0..clamped)
                .find(|&slot| entries[visual_order[slot]].available)
        };

        if let Some(slot) = resolved_slot {
            let entry_idx = visual_order[slot];
            self.selected_slot = Some(slot);
            self.selected_entry_idx = Some(entry_idx);
            self.selected_item = Some(entries[entry_idx].item.clone());
        } else {
            self.selected_slot = None;
            self.selected_entry_idx = None;
            self.selected_item = None;
        }
    }

    fn step_selection(&mut self, visual_order: &[usize], entries: &[LauncherEntry], delta: isize) {
        let avail_slots: Vec<usize> = (0..visual_order.len())
            .filter(|&slot| entries[visual_order[slot]].available)
            .collect();
        if avail_slots.is_empty() {
            self.selected_slot = None;
            self.selected_entry_idx = None;
            self.selected_item = None;
            return;
        }
        let cur_pos = self
            .selected_slot
            .and_then(|cur| avail_slots.iter().position(|&s| s == cur))
            .unwrap_or(0);
        let n = avail_slots.len() as isize;
        let next_pos = (cur_pos as isize + delta).rem_euclid(n) as usize;
        let next_slot = avail_slots[next_pos];
        let entry_idx = visual_order[next_slot];
        self.selected_slot = Some(next_slot);
        self.selected_entry_idx = Some(entry_idx);
        self.selected_item = Some(entries[entry_idx].item.clone());
    }

    /// Renders the launcher menu and returns `Some(LaunchMenuAction::Launch { .. })` when an
    /// available entry is activated or `Some(LaunchMenuAction::Close)` on `Escape`.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        input: LauncherInput<'_>,
    ) -> Option<LaunchMenuAction> {
        let viewport = ui.ctx().content_rect();
        let min_menu_w = 328.0_f32.min((viewport.width() - 36.0_f32).max(200.0_f32));
        if ui.available_width() < min_menu_w {
            ui.set_min_width(min_menu_w);
        }

        let visual_order = visual_entry_order(input.entries);
        self.sync_selection(&visual_order, input.entries);

        let mut fresh_enter = false;
        let mut fresh_escape = false;
        let (down, up) = if ui.is_enabled() {
            ui.input_mut(|i| {
                fresh_enter = i.events.iter().any(|ev| {
                    matches!(
                        ev,
                        egui::Event::Key {
                            key: egui::Key::Enter,
                            pressed: true,
                            repeat: false,
                            modifiers,
                            ..
                        } if modifiers.matches_logically(egui::Modifiers::NONE)
                    )
                });
                fresh_escape = i.events.iter().any(|ev| {
                    matches!(
                        ev,
                        egui::Event::Key {
                            key: egui::Key::Escape,
                            pressed: true,
                            repeat: false,
                            modifiers,
                            ..
                        } if modifiers.matches_logically(egui::Modifiers::NONE)
                    )
                });
                let _ = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                let _ = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                let down = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
                let up = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
                (down, up)
            })
        } else {
            (false, false)
        };

        if fresh_escape {
            return Some(LaunchMenuAction::Close);
        }
        if down {
            self.step_selection(&visual_order, input.entries, 1);
        }
        if up {
            self.step_selection(&visual_order, input.entries, -1);
        }

        self.render_where_header(ui, &input);
        let active_placement = self.effective_placement(input.placements);

        let mut action = if fresh_enter {
            self.selected_entry_idx
                .and_then(|idx| input.entries.get(idx))
                .filter(|entry| entry.available)
                .map(|entry| LaunchMenuAction::Launch {
                    item: entry.item.clone(),
                    placement: active_placement,
                    in_worktree: false,
                })
        } else {
            None
        };

        let scroll_max_h = (viewport.height() * 0.80_f32 - 140.0_f32).clamp(120.0_f32, 480.0_f32);
        let moved_by_key = down || up;
        egui::ScrollArea::vertical()
            .id_salt("tessel-launcher-panel-scroll")
            .auto_shrink([false, true])
            .max_height(scroll_max_h)
            .show(ui, |ui| {
                let mut current_section: Option<SectionKind> = None;
                for (slot, &entry_idx) in visual_order.iter().enumerate() {
                    let entry = &input.entries[entry_idx];
                    let section = SectionKind::of(&entry.item);
                    if current_section != Some(section) {
                        if current_section.is_some() {
                            ui.add_space(4.0_f32);
                        }
                        if let Some(heading) = section.heading(input.lang) {
                            ui.add_space(2.0_f32);
                            ui.label(
                                egui::RichText::new(heading.to_uppercase())
                                    .size(10.5_f32)
                                    .strong()
                                    .color(ui.visuals().weak_text_color()),
                            );
                            ui.add_space(2.0_f32);
                        }
                        current_section = Some(section);
                    }

                    let is_selected = self.selected_slot == Some(slot) && entry.available;
                    let resp = self.render_entry_row(ui, slot, entry, is_selected, input.lang);
                    if moved_by_key && is_selected {
                        resp.scroll_to_me(Some(egui::Align::Center));
                    }
                    if entry.available && resp.clicked() {
                        self.selected_slot = Some(slot);
                        self.selected_entry_idx = Some(entry_idx);
                        self.selected_item = Some(entry.item.clone());
                        action = Some(LaunchMenuAction::Launch {
                            item: entry.item.clone(),
                            placement: active_placement,
                            in_worktree: false,
                        });
                    }
                }

                let has_any_agent = input.entries.iter().any(|e| {
                    matches!(
                        e.item,
                        LaunchMenuItem::Agent { .. } | LaunchMenuItem::InstallAgent { .. }
                    )
                });
                if !input.entries.is_empty() && !has_any_agent {
                    ui.add_space(4.0_f32);
                    if let Some(heading) = SectionKind::Agents.heading(input.lang) {
                        ui.label(
                            egui::RichText::new(heading.to_uppercase())
                                .size(10.5_f32)
                                .strong()
                                .color(ui.visuals().weak_text_color()),
                        );
                    }
                    let no_agents = match input.lang {
                        LauncherLang::En => "No AI agents installed yet.",
                        LauncherLang::Fr => "Aucun agent IA installé pour l'instant.",
                    };
                    ui.colored_label(ui.visuals().weak_text_color(), no_agents);
                }
            });

        self.render_footer(ui, input.lang);
        action
    }

    fn render_where_header(&mut self, ui: &mut egui::Ui, input: &LauncherInput<'_>) {
        let current = self.effective_placement(input.placements);
        if !input.placements.is_empty() {
            let seg_bg = ui.visuals().extreme_bg_color;
            egui::Frame::new()
                .fill(seg_bg)
                .corner_radius(8.0_f32)
                .inner_margin(egui::Margin::same(2))
                .show(ui, |ui| {
                    let count = input.placements.len() as f32;
                    let gap = 2.0_f32;
                    let total_w = ui.available_width();
                    let btn_w =
                        ((total_w - gap * (count - 1.0_f32).max(0.0_f32)) / count).max(48.0_f32);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for &place in input.placements {
                            let active = place == current;
                            let label = place.label(input.lang);
                            let (rect, resp) =
                                ui.allocate_exact_size(Vec2::new(btn_w, 28.0_f32), Sense::click());
                            resp.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::RadioButton,
                                    ui.is_enabled(),
                                    active,
                                    label,
                                )
                            });
                            if resp.clicked() {
                                self.picked_placement = Some(place);
                                if place != LaunchPlacement::Auto {
                                    self.last_side = place;
                                }
                            }
                            let painter = ui.painter().with_clip_rect(rect);
                            if active {
                                painter.rect(
                                    rect,
                                    6.0_f32,
                                    ui.visuals().widgets.hovered.bg_fill,
                                    Stroke::new(
                                        1.0_f32,
                                        ui.visuals().widgets.noninteractive.bg_stroke.color,
                                    ),
                                    StrokeKind::Inside,
                                );
                            } else if resp.hovered() {
                                painter.rect_filled(
                                    rect,
                                    6.0_f32,
                                    ui.visuals().widgets.inactive.weak_bg_fill,
                                );
                            }
                            let fg = if active || resp.hovered() {
                                ui.visuals().strong_text_color()
                            } else {
                                ui.visuals().weak_text_color()
                            };
                            let font = FontId::proportional(11.5_f32);
                            let galley = painter.layout_no_wrap(label.to_owned(), font, fg);
                            let content_w = 14.0_f32 + 5.0_f32 + galley.size().x;
                            let start_x =
                                (rect.center().x - content_w * 0.5_f32).max(rect.min.x + 4.0_f32);
                            let icon_rect = Rect::from_center_size(
                                Pos2::new(start_x + 7.0_f32, rect.center().y),
                                Vec2::splat(14.0_f32),
                            );
                            paint_placement_icon(&painter, icon_rect, place, fg);
                            let text_pos = Pos2::new(
                                icon_rect.max.x + 5.0_f32,
                                rect.center().y - galley.size().y * 0.5_f32,
                            );
                            painter.galley(text_pos, galley, fg);
                            resp.on_hover_text(placement_tooltip(place, input.lang));
                        }
                    });
                });
            ui.add_space(4.0_f32);
        }

        let updated = self.effective_placement(input.placements);
        let has_panes = !input.placements.is_empty() || input.target_title.is_some();
        let fallback_target = match input.lang {
            LauncherLang::En => "active pane",
            LauncherLang::Fr => "le panneau actif",
        };
        let resolved_target = input
            .target_title
            .filter(|s| !s.trim().is_empty())
            .or_else(|| has_panes.then_some(fallback_target));
        let summary = where_text(
            updated,
            has_panes,
            input.placements.contains(&LaunchPlacement::Auto),
            resolved_target,
            input.lang,
        );
        let (line_rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0_f32), Sense::hover());
        paint_single_line_ellipsis(
            ui.painter(),
            line_rect,
            &summary,
            FontId::proportional(11.0_f32),
            ui.visuals().weak_text_color(),
        );
        ui.add_space(2.0_f32);
        ui.separator();
    }

    fn render_entry_row(
        &self,
        ui: &mut egui::Ui,
        slot: usize,
        entry: &LauncherEntry,
        is_selected: bool,
        lang: LauncherLang,
    ) -> Response {
        let label = entry_display_name(&entry.item, lang);
        let row_w = ui.available_width().max(160.0_f32);
        let sense = if entry.available {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(row_w, 32.0_f32), sense);
        let _ = slot;
        if entry.available {
            resp.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    is_selected,
                    label,
                )
            });
        } else {
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, false, label));
        }

        let painter = ui.painter().with_clip_rect(rect);
        let highlighted = entry.available && (is_selected || resp.hovered());
        if highlighted {
            let stroke = if is_selected {
                Stroke::new(
                    1.0_f32,
                    ui.visuals().selection.stroke.color.gamma_multiply(0.55_f32),
                )
            } else {
                Stroke::NONE
            };
            painter.rect(
                rect,
                7.0_f32,
                ui.visuals().widgets.hovered.bg_fill,
                stroke,
                StrokeKind::Inside,
            );
        }

        let text_color = if !entry.available {
            ui.visuals().weak_text_color().gamma_multiply(0.55_f32)
        } else if highlighted {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        let icon_color = if !entry.available {
            ui.visuals().weak_text_color().gamma_multiply(0.45_f32)
        } else if highlighted {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };

        let icon_rect = Rect::from_center_size(
            Pos2::new(rect.min.x + 18.0_f32, rect.center().y),
            Vec2::splat(16.0_f32),
        );
        paint_entry_icon(
            &painter,
            icon_rect,
            &entry.item,
            icon_color,
            ui.visuals().selection.stroke.color,
            entry.available,
        );

        let default_tag = match (&entry.item, entry.available) {
            (LaunchMenuItem::Shell { id, .. }, true) if id == "default" => Some(match lang {
                LauncherLang::En => "Default",
                LauncherLang::Fr => "Par défaut",
            }),
            _ => None,
        };
        let tag_info: Option<(&str, bool)> = if !entry.available {
            entry.reason.as_deref().map(|r| (r, true))
        } else {
            default_tag.map(|t| (t, false))
        };

        let mut name_right = rect.max.x - 10.0_f32;
        if let Some((tag_text, muted)) = tag_info {
            let max_tag_w = (rect.width() * 0.44_f32).clamp(56.0_f32, 168.0_f32);
            let tag_font = FontId::proportional(10.5_f32);
            let tag_fg = if muted {
                ui.visuals().weak_text_color()
            } else {
                ui.visuals().strong_text_color()
            };
            let mut job = egui::text::LayoutJob::simple(
                tag_text.to_owned(),
                tag_font,
                tag_fg,
                (max_tag_w - 14.0_f32).max(24.0_f32),
            );
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('…');
            let galley = painter.layout_job(job);
            let pill_w = (galley.size().x + 14.0_f32).min(max_tag_w);
            let pill_rect = Rect::from_center_size(
                Pos2::new(rect.max.x - 8.0_f32 - pill_w * 0.5_f32, rect.center().y),
                Vec2::new(pill_w, 18.0_f32),
            );
            let pill_bg = if muted {
                ui.visuals().widgets.noninteractive.bg_fill
            } else {
                ui.visuals().selection.bg_fill.gamma_multiply(0.45_f32)
            };
            painter.rect_filled(pill_rect, 9.0_f32, pill_bg);
            painter.with_clip_rect(pill_rect).galley(
                Pos2::new(
                    pill_rect.min.x + 7.0_f32,
                    pill_rect.center().y - galley.size().y * 0.5_f32,
                ),
                galley,
                tag_fg,
            );
            name_right = (pill_rect.min.x - 8.0_f32).max(rect.min.x + 40.0_f32);
        }

        let name_rect = Rect::from_min_max(
            Pos2::new(rect.min.x + 36.0_f32, rect.min.y),
            Pos2::new(name_right, rect.max.y),
        );
        paint_single_line_ellipsis(
            &painter,
            name_rect,
            label,
            FontId::proportional(12.5_f32),
            text_color,
        );

        if let Some(reason) = &entry.reason {
            resp.on_hover_text(reason.as_str())
        } else if entry.available {
            let start_hint = match lang {
                LauncherLang::En => format!("Start {label}"),
                LauncherLang::Fr => format!("Démarrer {label}"),
            };
            resp.on_hover_text(start_hint)
        } else {
            let _ = Align2::LEFT_CENTER;
            resp
        }
    }

    fn render_footer(&self, ui: &mut egui::Ui, lang: LauncherLang) {
        ui.add_space(4.0_f32);
        ui.separator();
        ui.add_space(2.0_f32);
        let weak = ui.visuals().weak_text_color();
        let (move_lbl, enter_key, open_lbl, esc_key, close_lbl) = match lang {
            LauncherLang::En => ("to move,", "Enter", "to open,", "Esc", "to close"),
            LauncherLang::Fr => (
                "pour naviguer,",
                "Entrée",
                "pour ouvrir,",
                "Échap",
                "pour fermer",
            ),
        };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0_f32;
            render_keycap(ui, "↑");
            render_keycap(ui, "↓");
            ui.colored_label(weak, egui::RichText::new(move_lbl).size(10.5_f32));
            render_keycap(ui, enter_key);
            ui.colored_label(weak, egui::RichText::new(open_lbl).size(10.5_f32));
            render_keycap(ui, esc_key);
            ui.colored_label(weak, egui::RichText::new(close_lbl).size(10.5_f32));
        });
    }
}
