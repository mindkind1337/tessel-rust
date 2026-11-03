//! Native `egui` Appearance settings page (`SettingsAppearance.vue` + terminal font size parity).
//!
//! Pure UI component with zero filesystem, process, network, or persistence side effects:
//! - Composes directly with [`crate::theme_palettes`] (`THEMES`, [`ThemeId`], localized labels,
//!   descriptions, UI chrome/surface/accent tokens, and terminal/ANSI preview swatches).
//! - Renders ONLY settings backed by real native state in [`AppearanceInput`]:
//!   1. **Interface language** (`English` / `Français` segmented control).
//!   2. **Theme** (adaptive 7-card grid with miniature 4-zone Tessel shell + ANSI palette preview,
//!      plus an honest non-selectable notice when `theme == None && !dark` so Legacy light is
//!      represented truthfully without inventing an 8th theme option).
//!   3. **Terminal font size** (`10.0..=28.0` px stepper + range slider, explicitly scoped to
//!      terminal panes and the code editor rather than claiming whole-UI chrome scaling).
//! - Respects external keyboard focus (`SettingsNav` rail, modals, popups), `!ui.is_enabled()`,
//!   unfocused windows (`!ui.input(|i| i.focused)`), and held-key anti-repeat, and never emits an
//!   action on plain render.

use eframe::egui::{
    self, Color32, FontId, Id, Key, Modifiers, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};

use crate::theme_palettes::{THEMES, ThemeId, WARP_OK_GREEN};

/// Minimum supported terminal/editor monospace font size (`px`).
pub const MIN_TERMINAL_FONT_SIZE: f32 = 10.0;

/// Maximum supported terminal/editor monospace font size (`px`).
pub const MAX_TERMINAL_FONT_SIZE: f32 = 28.0;

/// Fallback terminal font size when caller supplies a non-finite value (`Workspace::default()`).
pub const DEFAULT_TERMINAL_FONT_SIZE: f32 = 15.0;

/// Frame input for [`AppearancePanel::show`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppearanceInput {
    pub theme: Option<ThemeId>,
    pub dark: bool,
    pub french: bool,
    pub terminal_font_size: f32,
}

/// User action emitted by [`AppearancePanel::show`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AppearanceAction {
    Theme(ThemeId),
    Language { french: bool },
    TerminalFontSize(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusGroup {
    Language,
    Themes,
    FontSize,
}

fn clamp_font_size(size: f32) -> f32 {
    if !size.is_finite() {
        DEFAULT_TERMINAL_FONT_SIZE
    } else {
        size.clamp(MIN_TERMINAL_FONT_SIZE, MAX_TERMINAL_FONT_SIZE)
    }
}

fn format_font_px(size: f32) -> String {
    let clamped = clamp_font_size(size);
    if (clamped - clamped.round()).abs() < 0.05_f32 {
        format!("{} px", clamped.round() as i32)
    } else {
        format!("{clamped:.1} px")
    }
}

fn key_held_after_events(events: &[egui::Event], target: Key) -> bool {
    let mut held = false;
    for ev in events {
        if let egui::Event::Key { key, pressed, .. } = ev
            && *key == target
        {
            held = *pressed;
        }
    }
    held
}

fn effective_selected_theme(input: &AppearanceInput) -> Option<ThemeId> {
    if let Some(id) = input.theme {
        Some(id)
    } else if input.dark {
        Some(ThemeId::default())
    } else {
        None
    }
}

fn theme_swatches(theme: ThemeId) -> [Color32; 5] {
    let spec = theme.spec();
    if let Some(ansi) = &spec.term.ansi {
        [spec.ui.accent, ansi.green, ansi.yellow, ansi.red, ansi.cyan]
    } else {
        let ok_green = if theme == ThemeId::Warp {
            WARP_OK_GREEN
        } else {
            Color32::from_rgb(0x3d, 0xdc, 0x84)
        };
        [
            spec.ui.accent,
            ok_green,
            spec.ui.warn,
            spec.ui.danger,
            spec.term.selection_background,
        ]
    }
}

const fn page_title(french: bool) -> &'static str {
    if french { "Apparence" } else { "Appearance" }
}

const fn page_desc(french: bool) -> &'static str {
    if french {
        "La langue de l’interface, le thème de couleurs et la taille de police du terminal."
    } else {
        "Interface language, color theme, and terminal font size."
    }
}

const fn interface_group_title(_french: bool) -> &'static str {
    "Interface"
}

const fn language_title(french: bool) -> &'static str {
    if french { "Langue" } else { "Language" }
}

const fn language_hint(french: bool) -> &'static str {
    if french {
        "La langue des menus, panneaux et raccourcis de Tessel."
    } else {
        "The language of Tessel’s menus, panels, and shortcuts."
    }
}

const fn theme_group_title(french: bool) -> &'static str {
    if french { "Thème" } else { "Theme" }
}

const fn theme_group_desc(french: bool) -> &'static str {
    if french {
        "S’applique immédiatement. Vos sessions continuent."
    } else {
        "Applies immediately. Your sessions keep running."
    }
}

const fn legacy_light_callout(french: bool) -> &'static str {
    if french {
        "Thème clair hérité actif — choisissez l’un des 7 thèmes ci-dessous pour changer."
    } else {
        "Legacy light theme active — choose one of the 7 themes below to switch."
    }
}

const fn font_group_title(french: bool) -> &'static str {
    if french {
        "Police du terminal"
    } else {
        "Terminal font"
    }
}

const fn font_group_desc(french: bool) -> &'static str {
    if french {
        "S’applique aux panneaux de terminal et à l’éditeur de code (10 à 28 px), pas à toute l’interface."
    } else {
        "Applies to terminal panes and the code editor (10–28 px), not the whole application interface."
    }
}

const fn font_row_title(french: bool) -> &'static str {
    if french {
        "Taille de police du terminal"
    } else {
        "Terminal font size"
    }
}

const fn font_row_hint(french: bool) -> &'static str {
    if french {
        "De 10 px à 28 px"
    } else {
        "10 px to 28 px"
    }
}

const fn smaller_label(french: bool) -> &'static str {
    if french { "Plus petit" } else { "Smaller" }
}

const fn bigger_label(french: bool) -> &'static str {
    if french { "Plus grand" } else { "Bigger" }
}

#[allow(clippy::too_many_arguments)]
fn paint_wrapped_text(
    ui: &egui::Ui,
    painter: &Painter,
    pos: Pos2,
    width: f32,
    text: &str,
    font: FontId,
    color: Color32,
    max_rows: usize,
) -> f32 {
    if width <= 4.0_f32 {
        return 0.0_f32;
    }
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, width);
    job.wrap.max_rows = max_rows;
    job.wrap.break_anywhere = false;
    job.wrap.overflow_character = Some('…');
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let h = galley.size().y;
    painter.galley(pos, galley, color);
    h
}

fn layout_wrapped_galley(
    ui: &egui::Ui,
    width: f32,
    text: &str,
    font: FontId,
    color: Color32,
    max_rows: usize,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, width.max(12.0_f32));
    job.wrap.max_rows = max_rows;
    job.wrap.break_anywhere = false;
    job.wrap.overflow_character = Some('…');
    ui.fonts_mut(|f| f.layout_job(job))
}

fn paint_checkmark(painter: &Painter, rect: Rect, color: Color32) {
    let side = rect.width().min(rect.height());
    if !side.is_finite() || side <= 0.0_f32 {
        return;
    }
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let s = side / 14.0_f32;
    let pt = |x: f32, y: f32| Pos2::new(sq.min.x + x * s, sq.min.y + y * s);
    painter.add(egui::Shape::line(
        vec![
            pt(2.6_f32, 7.3_f32),
            pt(5.8_f32, 10.5_f32),
            pt(11.5_f32, 4.0_f32),
        ],
        Stroke::new((1.7_f32 * s).max(1.1_f32), color),
    ));
}

fn paint_plus_or_minus(painter: &Painter, rect: Rect, color: Color32, plus: bool) {
    let side = rect.width().min(rect.height());
    if !side.is_finite() || side <= 0.0_f32 {
        return;
    }
    let c = rect.center();
    let half = (side * 0.32_f32).clamp(2.5_f32, 5.5_f32);
    let stroke = Stroke::new(1.5_f32, color);
    painter.line_segment(
        [Pos2::new(c.x - half, c.y), Pos2::new(c.x + half, c.y)],
        stroke,
    );
    if plus {
        painter.line_segment(
            [Pos2::new(c.x, c.y - half), Pos2::new(c.x, c.y + half)],
            stroke,
        );
    }
}

fn paint_info_badge(painter: &Painter, center: Pos2, color: Color32) {
    painter.circle_stroke(center, 6.5_f32, Stroke::new(1.3_f32, color));
    painter.circle_filled(Pos2::new(center.x, center.y - 2.8_f32), 1.0_f32, color);
    painter.line_segment(
        [
            Pos2::new(center.x, center.y - 0.5_f32),
            Pos2::new(center.x, center.y + 3.6_f32),
        ],
        Stroke::new(1.4_f32, color),
    );
}

/// Native `egui` Appearance settings page (`SettingsAppearance.vue` parity).
#[derive(Clone, Debug, Default)]
pub struct AppearancePanel {
    focused_lang_french: Option<bool>,
    focused_theme: Option<ThemeId>,
    held_key: Option<Key>,
    dragging_slider: bool,
    slider_gesture_cancelled: bool,
}

impl AppearancePanel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets transient keyboard focus, held-key, and slider drag state.
    pub fn reset(&mut self) {
        self.focused_lang_french = None;
        self.focused_theme = None;
        self.held_key = None;
        self.dragging_slider = false;
        self.slider_gesture_cancelled = false;
    }

    fn classify_focus(
        focused: Option<Id>,
        lang_id: Id,
        themes_id: Id,
        font_id: Id,
    ) -> Option<FocusGroup> {
        let fid = focused?;
        if fid == lang_id {
            Some(FocusGroup::Language)
        } else if fid == themes_id {
            Some(FocusGroup::Themes)
        } else if fid == font_id {
            Some(FocusGroup::FontSize)
        } else {
            None
        }
    }

    fn update_key_releases(&mut self, ui: &egui::Ui, interaction_blocked: bool) {
        if interaction_blocked {
            self.held_key = None;
            return;
        }
        if let Some(held) = self.held_key {
            let released = ui.input(|i| {
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
                self.held_key = None;
            }
        }
    }

    fn handle_focused_keys(
        &mut self,
        ui: &mut egui::Ui,
        group: FocusGroup,
        owner_id: Id,
        input: &AppearanceInput,
        active_theme: Option<ThemeId>,
        current_font_size: f32,
    ) -> (Option<AppearanceAction>, bool) {
        let held_before = self.held_key;
        let mut action = None;
        let mut scroll_to_theme = false;

        match group {
            FocusGroup::Language => {
                let mut chosen_key: Option<Key> = None;
                let mut still_held = false;
                let mut arrow_target: Option<bool> = None;
                let mut activate_focused = false;

                ui.input_mut(|i| {
                    for ev in &i.events {
                        if let egui::Event::Key {
                            key: k @ (Key::ArrowLeft | Key::ArrowRight | Key::Enter | Key::Space),
                            pressed: true,
                            repeat: false,
                            modifiers,
                            ..
                        } = ev
                            && modifiers.matches_logically(Modifiers::NONE)
                            && held_before != Some(*k)
                        {
                            chosen_key = Some(*k);
                            match k {
                                Key::ArrowLeft => arrow_target = Some(false),
                                Key::ArrowRight => arrow_target = Some(true),
                                Key::Enter | Key::Space => activate_focused = true,
                                _ => {}
                            }
                            break;
                        }
                    }
                    if let Some(k) = chosen_key {
                        still_held = key_held_after_events(&i.events, k);
                    }
                    let _ = i.consume_key(Modifiers::NONE, Key::ArrowLeft);
                    let _ = i.consume_key(Modifiers::NONE, Key::ArrowRight);
                    let _ = i.consume_key(Modifiers::NONE, Key::Enter);
                    let _ = i.consume_key(Modifiers::NONE, Key::Space);
                });

                if let Some(k) = chosen_key {
                    ui.ctx().memory_mut(|m| m.request_focus(owner_id));
                    self.held_key = still_held.then_some(k);
                    if let Some(target_fr) = arrow_target {
                        self.focused_lang_french = Some(target_fr);
                        if target_fr != input.french {
                            action = Some(AppearanceAction::Language { french: target_fr });
                        }
                    } else if activate_focused {
                        let target_fr = self.focused_lang_french.unwrap_or(!input.french);
                        self.focused_lang_french = Some(target_fr);
                        if target_fr != input.french {
                            action = Some(AppearanceAction::Language { french: target_fr });
                        }
                    }
                }
            }
            FocusGroup::Themes => {
                let mut activate_key: Option<Key> = None;
                let mut still_held = false;
                let (left, right, up, down) = ui.input_mut(|i| {
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
                            activate_key = Some(*k);
                            break;
                        }
                    }
                    if let Some(k) = activate_key {
                        still_held = key_held_after_events(&i.events, k);
                    }
                    let _ = i.consume_key(Modifiers::NONE, Key::Enter);
                    let _ = i.consume_key(Modifiers::NONE, Key::Space);
                    let left = i.consume_key(Modifiers::NONE, Key::ArrowLeft);
                    let right = i.consume_key(Modifiers::NONE, Key::ArrowRight);
                    let up = i.consume_key(Modifiers::NONE, Key::ArrowUp);
                    let down = i.consume_key(Modifiers::NONE, Key::ArrowDown);
                    (left, right, up, down)
                });

                if left || right || up || down {
                    ui.ctx().memory_mut(|m| m.request_focus(owner_id));
                    let step: isize = if right || down { 1 } else { -1 };
                    let base = self
                        .focused_theme
                        .or(active_theme)
                        .unwrap_or(ThemeId::Classic);
                    let cur_idx = THEMES.iter().position(|&t| t == base).unwrap_or(0);
                    let next_idx =
                        (cur_idx as isize + step).rem_euclid(THEMES.len() as isize) as usize;
                    self.focused_theme = Some(THEMES[next_idx]);
                    scroll_to_theme = true;
                } else if let Some(k) = activate_key {
                    ui.ctx().memory_mut(|m| m.request_focus(owner_id));
                    self.held_key = still_held.then_some(k);
                    let target = self
                        .focused_theme
                        .or(active_theme)
                        .unwrap_or(ThemeId::Classic);
                    self.focused_theme = Some(target);
                    if active_theme != Some(target) {
                        action = Some(AppearanceAction::Theme(target));
                    }
                }
            }
            FocusGroup::FontSize => {
                let mut step_key: Option<(Key, f32)> = None;
                let mut still_held = false;
                ui.input_mut(|i| {
                    for ev in &i.events {
                        if let egui::Event::Key {
                            key:
                                k @ (Key::ArrowLeft
                                | Key::ArrowDown
                                | Key::Minus
                                | Key::ArrowRight
                                | Key::ArrowUp
                                | Key::Plus
                                | Key::Equals),
                            pressed: true,
                            repeat: false,
                            modifiers,
                            ..
                        } = ev
                            && (modifiers.matches_logically(Modifiers::NONE)
                                || (matches!(*k, Key::Plus | Key::Equals)
                                    && modifiers.matches_logically(Modifiers::SHIFT)))
                            && held_before != Some(*k)
                        {
                            let delta = match k {
                                Key::ArrowLeft | Key::ArrowDown | Key::Minus => -1.0_f32,
                                _ => 1.0_f32,
                            };
                            step_key = Some((*k, delta));
                            break;
                        }
                    }
                    if let Some((k, _)) = step_key {
                        still_held = key_held_after_events(&i.events, k);
                    }
                    let _ = i.consume_key(Modifiers::NONE, Key::ArrowLeft);
                    let _ = i.consume_key(Modifiers::NONE, Key::ArrowDown);
                    let _ = i.consume_key(Modifiers::NONE, Key::Minus);
                    let _ = i.consume_key(Modifiers::NONE, Key::ArrowRight);
                    let _ = i.consume_key(Modifiers::NONE, Key::ArrowUp);
                    let _ = i.consume_key(Modifiers::NONE, Key::Plus);
                    let _ = i.consume_key(Modifiers::SHIFT, Key::Plus);
                    let _ = i.consume_key(Modifiers::NONE, Key::Equals);
                    let _ = i.consume_key(Modifiers::SHIFT, Key::Equals);
                });

                if let Some((k, delta)) = step_key {
                    ui.ctx().memory_mut(|m| m.request_focus(owner_id));
                    self.held_key = still_held.then_some(k);
                    let next = clamp_font_size(current_font_size + delta);
                    if (next - current_font_size).abs() > 1e-4_f32 {
                        action = Some(AppearanceAction::TerminalFontSize(next));
                    }
                }
            }
        }

        (action, scroll_to_theme)
    }

    /// Renders the Appearance settings page and returns any user-initiated [`AppearanceAction`].
    pub fn show(&mut self, ui: &mut egui::Ui, input: &AppearanceInput) -> Option<AppearanceAction> {
        let panel_id = ui.id().with("tessel-appearance-panel");
        let lang_group_id = panel_id.with("lang-group");
        let themes_group_id = panel_id.with("themes-group");
        let font_group_id = panel_id.with("font-group");

        let popup_open = egui::Popup::is_any_open(ui.ctx());
        let window_focused = ui.input(|i| i.focused);
        let interaction_blocked = !ui.is_enabled() || !window_focused || popup_open;
        self.update_key_releases(ui, interaction_blocked);

        let (any_down, any_released, primary_pressed, escape_pressed) = ui.input(|i| {
            (
                i.pointer.any_down(),
                i.pointer.any_released(),
                i.pointer.primary_pressed(),
                i.key_pressed(Key::Escape),
            )
        });
        if !any_down && !any_released {
            self.slider_gesture_cancelled = false;
            self.dragging_slider = false;
        }
        if !interaction_blocked && primary_pressed && !escape_pressed {
            self.slider_gesture_cancelled = false;
        }
        if interaction_blocked || escape_pressed {
            if self.dragging_slider || any_down || any_released {
                self.slider_gesture_cancelled = true;
            }
            self.dragging_slider = false;
        }

        let active_theme = effective_selected_theme(input);
        let is_legacy_light = input.theme.is_none() && !input.dark;
        let current_font_size = clamp_font_size(input.terminal_font_size);

        let focused_before = ui.ctx().memory(|m| m.focused());
        let focus_group = if interaction_blocked {
            self.focused_lang_french = None;
            self.focused_theme = None;
            None
        } else {
            let g = Self::classify_focus(
                focused_before,
                lang_group_id,
                themes_group_id,
                font_group_id,
            );
            if g != Some(FocusGroup::Language) {
                self.focused_lang_french = None;
            }
            if g != Some(FocusGroup::Themes) {
                self.focused_theme = None;
            }
            match g {
                Some(FocusGroup::Language) => {
                    if self.focused_lang_french.is_none() {
                        self.focused_lang_french = Some(input.french);
                    }
                }
                Some(FocusGroup::Themes) => {
                    if self.focused_theme.is_none() {
                        self.focused_theme = Some(active_theme.unwrap_or(ThemeId::Classic));
                    }
                }
                Some(FocusGroup::FontSize) => {}
                None => {
                    self.held_key = None;
                }
            }
            g
        };

        let (mut action, scroll_to_theme) = match focus_group {
            Some(g) => {
                let owner_id = match g {
                    FocusGroup::Language => lang_group_id,
                    FocusGroup::Themes => themes_group_id,
                    FocusGroup::FontSize => font_group_id,
                };
                self.handle_focused_keys(ui, g, owner_id, input, active_theme, current_font_size)
            }
            None => (None, false),
        };

        let avail_w = ui.available_width().max(120.0_f32);
        ui.spacing_mut().item_spacing.y = 0.0_f32;

        // `.set-page-head` (`style.css:3547-3558`)
        self.render_page_header(ui, avail_w, input.french);
        ui.add_space(20.0_f32);

        // 1. `.set-group`: Interface -> Language (`English` / `Français`).
        if let Some(lang_act) = self.render_language_section(
            ui,
            panel_id,
            lang_group_id,
            avail_w,
            input.french,
            interaction_blocked,
        ) {
            action = Some(lang_act);
        }
        ui.add_space(22.0_f32);

        // 2. `.set-group`: Theme -> Legacy light notice (if active) + 7 adaptive preview cards.
        if let Some(theme_act) = self.render_themes_section(
            ui,
            panel_id,
            themes_group_id,
            avail_w,
            input.french,
            active_theme,
            is_legacy_light,
            scroll_to_theme,
            interaction_blocked,
        ) {
            action = Some(theme_act);
        }
        ui.add_space(22.0_f32);

        // 3. `.set-group`: Terminal font -> `10..=28 px` stepper + range slider.
        if let Some(font_act) = self.render_font_section(
            ui,
            panel_id,
            font_group_id,
            avail_w,
            input.french,
            current_font_size,
            interaction_blocked,
        ) {
            action = Some(font_act);
        }

        if !interaction_blocked {
            ui.ctx().memory_mut(|m| {
                m.set_focus_lock_filter(
                    lang_group_id,
                    egui::EventFilter {
                        tab: false,
                        horizontal_arrows: true,
                        vertical_arrows: false,
                        escape: false,
                    },
                );
                m.set_focus_lock_filter(
                    themes_group_id,
                    egui::EventFilter {
                        tab: false,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: false,
                    },
                );
                m.set_focus_lock_filter(
                    font_group_id,
                    egui::EventFilter {
                        tab: false,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: false,
                    },
                );
            });
        }

        action
    }

    fn render_page_header(&self, ui: &mut egui::Ui, avail_w: f32, french: bool) {
        let strong = ui.visuals().strong_text_color();
        let dim = ui.visuals().weak_text_color();

        let title_galley = layout_wrapped_galley(
            ui,
            avail_w,
            page_title(french),
            FontId::proportional(21.0_f32),
            strong,
            2,
        );
        let desc_galley = layout_wrapped_galley(
            ui,
            avail_w,
            page_desc(french),
            FontId::proportional(13.0_f32),
            dim,
            3,
        );
        let total_h = title_galley.size().y + 5.0_f32 + desc_galley.size().y;
        let (_, rect) = ui.allocate_space(Vec2::new(avail_w, total_h));
        let p = ui.painter().with_clip_rect(rect);
        let desc_y = rect.min.y + title_galley.size().y + 5.0_f32;
        p.galley(rect.min, title_galley, strong);
        p.galley(Pos2::new(rect.min.x, desc_y), desc_galley, dim);
    }

    fn render_group_heading(ui: &mut egui::Ui, avail_w: f32, title: &str, desc: Option<&str>) {
        let strong = ui.visuals().strong_text_color();
        let dim = ui.visuals().weak_text_color();

        let title_g = layout_wrapped_galley(
            ui,
            avail_w,
            title,
            FontId::proportional(13.5_f32),
            strong,
            2,
        );
        let desc_g = desc
            .map(|d| layout_wrapped_galley(ui, avail_w, d, FontId::proportional(12.0_f32), dim, 3));
        let h = title_g.size().y + desc_g.as_ref().map_or(0.0_f32, |g| 3.0_f32 + g.size().y);
        let (_, rect) = ui.allocate_space(Vec2::new(avail_w, h));
        let p = ui.painter().with_clip_rect(rect);
        let next_y = rect.min.y + title_g.size().y + 3.0_f32;
        p.galley(rect.min, title_g, strong);
        if let Some(dg) = desc_g {
            p.galley(Pos2::new(rect.min.x, next_y), dg, dim);
        }
        ui.add_space(8.0_f32);
    }

    fn render_language_section(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        lang_group_id: Id,
        avail_w: f32,
        french: bool,
        interaction_blocked: bool,
    ) -> Option<AppearanceAction> {
        Self::render_group_heading(ui, avail_w, interface_group_title(french), None);

        let strong = ui.visuals().strong_text_color();
        let dim = ui.visuals().weak_text_color();
        let card_bg = ui.visuals().widgets.inactive.bg_fill;
        let card_border = ui.visuals().widgets.noninteractive.bg_stroke.color;
        let accent = ui.visuals().selection.stroke.color;

        let stacked = avail_w < 460.0_f32;
        let inner_pad_x = if avail_w < 340.0_f32 {
            12.0_f32
        } else {
            18.0_f32
        };
        let seg_w = (avail_w - inner_pad_x * 2.0_f32).clamp(132.0_f32, 204.0_f32);
        let seg_h = 28.0_f32;

        let label_max_w = if stacked {
            (avail_w - inner_pad_x * 2.0_f32).max(80.0_f32)
        } else {
            (avail_w - inner_pad_x * 2.0_f32 - seg_w - 16.0_f32).max(100.0_f32)
        };

        let title_g = layout_wrapped_galley(
            ui,
            label_max_w,
            language_title(french),
            FontId::proportional(13.0_f32),
            strong,
            2,
        );
        let hint_g = layout_wrapped_galley(
            ui,
            label_max_w,
            language_hint(french),
            FontId::proportional(11.5_f32),
            dim,
            3,
        );
        let label_h = title_g.size().y + 3.0_f32 + hint_g.size().y;

        let card_h = if stacked {
            14.0_f32 + label_h + 10.0_f32 + seg_h + 14.0_f32
        } else {
            (label_h + 24.0_f32).max(58.0_f32)
        };

        let (_, card_rect) = ui.allocate_space(Vec2::new(avail_w, card_h));
        let group_resp = ui.interact(card_rect, lang_group_id, Sense::focusable_noninteractive());
        let group_focused = ui.is_enabled() && ui.ctx().memory(|m| m.has_focus(lang_group_id));

        let seg_rect = if stacked {
            Rect::from_min_size(
                Pos2::new(
                    card_rect.min.x + inner_pad_x,
                    card_rect.max.y - 14.0_f32 - seg_h,
                ),
                Vec2::new(seg_w, seg_h),
            )
        } else {
            Rect::from_min_size(
                Pos2::new(
                    card_rect.max.x - inner_pad_x - seg_w,
                    card_rect.center().y - seg_h * 0.5_f32,
                ),
                Vec2::new(seg_w, seg_h),
            )
        };

        let half_w = (seg_rect.width() - 4.0_f32) * 0.5_f32;
        let en_rect = Rect::from_min_size(
            Pos2::new(seg_rect.min.x + 2.0_f32, seg_rect.min.y + 2.0_f32),
            Vec2::new(half_w, seg_h - 4.0_f32),
        );
        let fr_rect = Rect::from_min_size(
            Pos2::new(en_rect.max.x, seg_rect.min.y + 2.0_f32),
            Vec2::new(half_w, seg_h - 4.0_f32),
        );

        let en_resp = ui.interact(en_rect, panel_id.with("lang-en"), Sense::CLICK);
        en_resp.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::RadioButton,
                ui.is_enabled(),
                !french,
                "English",
            )
        });
        let fr_resp = ui.interact(fr_rect, panel_id.with("lang-fr"), Sense::CLICK);
        fr_resp.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::RadioButton,
                ui.is_enabled(),
                french,
                "Français",
            )
        });

        let cp = ui.painter().with_clip_rect(card_rect);
        let outer_stroke = if group_focused {
            Stroke::new(1.5_f32, accent)
        } else {
            Stroke::new(1.0_f32, card_border)
        };
        cp.rect(
            card_rect,
            10.0_f32,
            card_bg,
            outer_stroke,
            StrokeKind::Inside,
        );

        let label_y = if stacked {
            card_rect.min.y + 14.0_f32
        } else {
            card_rect.center().y - label_h * 0.5_f32
        };
        let hint_y = label_y + title_g.size().y + 3.0_f32;
        cp.galley(
            Pos2::new(card_rect.min.x + inner_pad_x, label_y),
            title_g,
            strong,
        );
        cp.galley(
            Pos2::new(card_rect.min.x + inner_pad_x, hint_y),
            hint_g,
            dim,
        );

        cp.rect(
            seg_rect,
            6.0_f32,
            ui.visuals().extreme_bg_color,
            Stroke::new(1.0_f32, card_border),
            StrokeKind::Inside,
        );

        for (opt_rect, label, active, hovered, key_focus) in [
            (
                en_rect,
                "English",
                !french,
                en_resp.hovered(),
                group_focused && self.focused_lang_french == Some(false),
            ),
            (
                fr_rect,
                "Français",
                french,
                fr_resp.hovered(),
                group_focused && self.focused_lang_french == Some(true),
            ),
        ] {
            let bg = if active {
                ui.visuals().selection.bg_fill
            } else if hovered {
                ui.visuals()
                    .widgets
                    .hovered
                    .bg_fill
                    .gamma_multiply(0.55_f32)
            } else {
                Color32::TRANSPARENT
            };
            let stroke = if active || key_focus {
                Stroke::new(1.0_f32, accent)
            } else {
                Stroke::NONE
            };
            cp.rect(opt_rect, 5.0_f32, bg, stroke, StrokeKind::Inside);
            let fg = if active || hovered {
                strong
            } else {
                ui.visuals().text_color()
            };
            let g = cp.layout_no_wrap(label.to_owned(), FontId::proportional(12.0_f32), fg);
            cp.galley(
                Pos2::new(
                    opt_rect.center().x - g.size().x * 0.5_f32,
                    opt_rect.center().y - g.size().y * 0.5_f32,
                ),
                g,
                fg,
            );
        }

        let en_resp = en_resp.on_hover_text("English");
        let fr_resp = fr_resp.on_hover_text("Français");

        let mut action = None;
        if !interaction_blocked && en_resp.clicked() {
            self.focused_lang_french = Some(false);
            ui.ctx().memory_mut(|m| m.request_focus(lang_group_id));
            if french {
                action = Some(AppearanceAction::Language { french: false });
            }
        } else if !interaction_blocked && fr_resp.clicked() {
            self.focused_lang_french = Some(true);
            ui.ctx().memory_mut(|m| m.request_focus(lang_group_id));
            if !french {
                action = Some(AppearanceAction::Language { french: true });
            }
        }

        let current_lang_name = if french { "Français" } else { "English" };
        let group_a11y = format!("{} — {current_lang_name}", language_title(french));
        group_resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), &group_a11y)
        });

        action
    }

    #[allow(clippy::too_many_arguments)]
    fn render_themes_section(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        themes_group_id: Id,
        avail_w: f32,
        french: bool,
        active_theme: Option<ThemeId>,
        is_legacy_light: bool,
        scroll_to_theme: bool,
        interaction_blocked: bool,
    ) -> Option<AppearanceAction> {
        Self::render_group_heading(
            ui,
            avail_w,
            theme_group_title(french),
            Some(theme_group_desc(french)),
        );

        let accent = ui.visuals().selection.stroke.color;
        let strong = ui.visuals().strong_text_color();

        if is_legacy_light {
            let callout_text = legacy_light_callout(french);
            let text_w = (avail_w - 42.0_f32).max(60.0_f32);
            let callout_g = layout_wrapped_galley(
                ui,
                text_w,
                callout_text,
                FontId::proportional(12.0_f32),
                ui.visuals().text_color(),
                4,
            );
            let callout_h = (callout_g.size().y + 18.0_f32).max(36.0_f32);
            let (_, callout_rect) = ui.allocate_space(Vec2::new(avail_w, callout_h));
            let callout_resp = ui.interact(
                callout_rect,
                panel_id.with("legacy-light-note"),
                Sense::hover(),
            );
            callout_resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), callout_text)
            });
            let cp = ui.painter().with_clip_rect(callout_rect);
            cp.rect(
                callout_rect,
                8.0_f32,
                accent.gamma_multiply(0.14_f32),
                Stroke::new(1.0_f32, accent.gamma_multiply(0.45_f32)),
                StrokeKind::Inside,
            );
            paint_info_badge(
                &cp,
                Pos2::new(callout_rect.min.x + 15.0_f32, callout_rect.center().y),
                accent,
            );
            cp.galley(
                Pos2::new(
                    callout_rect.min.x + 30.0_f32,
                    callout_rect.center().y - callout_g.size().y * 0.5_f32,
                ),
                callout_g,
                ui.visuals().text_color(),
            );
            ui.add_space(10.0_f32);
        }

        let cols = if avail_w >= 680.0_f32 {
            3usize
        } else if avail_w >= 440.0_f32 {
            2usize
        } else {
            1usize
        };
        let gap = 10.0_f32;
        let card_w =
            ((avail_w - gap * (cols.saturating_sub(1) as f32)) / (cols as f32)).max(110.0_f32);
        let card_h = 110.0_f32;
        let rows = THEMES.len().div_ceil(cols);
        let grid_h = (rows as f32) * card_h + (rows.saturating_sub(1) as f32) * gap;

        let (_, grid_rect) = ui.allocate_space(Vec2::new(avail_w, grid_h));
        let group_resp = ui.interact(
            grid_rect,
            themes_group_id,
            Sense::focusable_noninteractive(),
        );
        let group_focused = ui.is_enabled() && ui.ctx().memory(|m| m.has_focus(themes_group_id));

        let mut action = None;

        for (idx, &theme) in THEMES.iter().enumerate() {
            let row = idx / cols;
            let col = idx % cols;
            let x = grid_rect.min.x + (col as f32) * (card_w + gap);
            let y = grid_rect.min.y + (row as f32) * (card_h + gap);
            let card_rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(card_w, card_h));

            let is_selected = active_theme == Some(theme);
            let is_key_focused = group_focused && self.focused_theme == Some(theme);

            let label = theme.label(french);
            let desc = theme.description(french);
            let card_id = panel_id.with(("theme-card", theme.as_str()));
            let resp = ui.interact(card_rect, card_id, Sense::CLICK);
            let a11y = format!("{label} — {desc}");
            resp.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::RadioButton,
                    ui.is_enabled(),
                    is_selected,
                    &a11y,
                )
            });
            if scroll_to_theme && is_key_focused {
                resp.scroll_to_me(Some(egui::Align::Center));
            }

            Self::paint_theme_card(
                ui,
                card_rect,
                theme,
                label,
                desc,
                is_selected,
                is_key_focused,
                resp.hovered(),
            );
            let resp = resp.on_hover_text(desc);

            if !interaction_blocked && resp.clicked() {
                self.focused_theme = Some(theme);
                ui.ctx().memory_mut(|m| m.request_focus(themes_group_id));
                if !is_selected {
                    action = Some(AppearanceAction::Theme(theme));
                }
            }
        }

        let current_theme_name = match active_theme {
            Some(t) => t.label(french),
            None if french => "Thème clair hérité",
            None => "Legacy light theme",
        };
        let group_a11y = format!("{} — {current_theme_name}", theme_group_title(french));
        let _ = strong;
        group_resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), &group_a11y)
        });

        action
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_theme_card(
        ui: &egui::Ui,
        rect: Rect,
        theme: ThemeId,
        label: &str,
        desc: &str,
        is_selected: bool,
        is_key_focused: bool,
        hovered: bool,
    ) {
        let spec = theme.spec();
        let p = ui.painter().with_clip_rect(rect);
        let ui_accent = ui.visuals().selection.stroke.color;

        let card_bg = if is_selected {
            ui.visuals().selection.bg_fill.gamma_multiply(0.85_f32)
        } else if hovered {
            ui.visuals().widgets.hovered.bg_fill.gamma_multiply(0.6_f32)
        } else {
            ui.visuals().widgets.inactive.bg_fill
        };
        let border_stroke = if is_selected || is_key_focused {
            Stroke::new(1.6_f32, ui_accent)
        } else if hovered {
            Stroke::new(1.0_f32, ui.visuals().widgets.hovered.bg_stroke.color)
        } else {
            Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color)
        };
        p.rect(rect, 9.0_f32, card_bg, border_stroke, StrokeKind::Inside);

        // Top miniature 4-zone Tessel shell preview (`chrome` + `surface` + `term` + swatches).
        let preview_rect = Rect::from_min_max(
            Pos2::new(rect.min.x + 8.0_f32, rect.min.y + 8.0_f32),
            Pos2::new(rect.max.x - 8.0_f32, rect.min.y + 46.0_f32),
        );
        p.rect(
            preview_rect,
            6.0_f32,
            spec.term.background,
            Stroke::new(1.0_f32, spec.ui.border),
            StrokeKind::Inside,
        );

        let top_bar_rect = Rect::from_min_max(
            preview_rect.min,
            Pos2::new(preview_rect.max.x, preview_rect.min.y + 8.0_f32),
        );
        p.rect_filled(top_bar_rect, 5.0_f32, spec.ui.chrome);

        let body_top = top_bar_rect.max.y;
        let sidebar_w = (preview_rect.width() * 0.22_f32).clamp(18.0_f32, 44.0_f32);
        let sidebar_rect = Rect::from_min_max(
            Pos2::new(preview_rect.min.x + 1.0_f32, body_top),
            Pos2::new(preview_rect.min.x + sidebar_w, preview_rect.max.y - 1.0_f32),
        );
        p.rect_filled(sidebar_rect, 2.0_f32, spec.ui.surface);

        // Mini active accent bar + terminal prompt lines inside the preview pane.
        let accent_pill = Rect::from_min_size(
            Pos2::new(sidebar_rect.min.x + 3.0_f32, sidebar_rect.min.y + 5.0_f32),
            Vec2::new((sidebar_w - 7.0_f32).max(6.0_f32), 3.0_f32),
        );
        p.rect_filled(accent_pill, 1.5_f32, spec.ui.accent);

        let term_x = sidebar_rect.max.x + 6.0_f32;
        let term_y = body_top + 7.0_f32;
        if term_x + 28.0_f32 < preview_rect.max.x {
            p.rect_filled(
                Rect::from_min_size(Pos2::new(term_x, term_y), Vec2::new(16.0_f32, 3.0_f32)),
                1.5_f32,
                spec.ui.accent,
            );
            p.rect_filled(
                Rect::from_min_size(
                    Pos2::new(term_x + 19.0_f32, term_y),
                    Vec2::new(22.0_f32, 3.0_f32),
                ),
                1.5_f32,
                spec.term.foreground.gamma_multiply(0.8_f32),
            );
            p.rect_filled(
                Rect::from_min_size(
                    Pos2::new(term_x, term_y + 7.0_f32),
                    Vec2::new(5.0_f32, 6.0_f32),
                ),
                1.0_f32,
                spec.term.cursor,
            );
        }

        // 5 palette swatch dots in the bottom-right corner of the miniature shell.
        let swatches = theme_swatches(theme);
        let dots_right = preview_rect.max.x - 7.0_f32;
        let dots_y = preview_rect.max.y - 8.0_f32;
        for (i, &swatch) in swatches.iter().rev().enumerate() {
            let cx = dots_right - (i as f32) * 9.5_f32;
            if cx - 4.0_f32 > sidebar_rect.max.x + 4.0_f32 {
                p.circle_filled(Pos2::new(cx, dots_y), 3.2_f32, swatch);
            }
        }

        // Theme title + selected checkmark badge.
        let text_left = rect.min.x + 10.0_f32;
        let check_reserve = if is_selected { 22.0_f32 } else { 0.0_f32 };
        let title_max_w = (rect.max.x - 10.0_f32 - text_left - check_reserve).max(32.0_f32);
        let title_y = preview_rect.max.y + 7.0_f32;
        let title_h = paint_wrapped_text(
            ui,
            &p,
            Pos2::new(text_left, title_y),
            title_max_w,
            label,
            FontId::proportional(13.0_f32),
            ui.visuals().strong_text_color(),
            1,
        );

        if is_selected {
            let badge_center = Pos2::new(rect.max.x - 16.0_f32, title_y + title_h * 0.5_f32);
            p.circle_filled(badge_center, 7.5_f32, ui_accent.gamma_multiply(0.22_f32));
            paint_checkmark(
                &p,
                Rect::from_center_size(badge_center, Vec2::splat(11.0_f32)),
                ui_accent,
            );
        }

        let desc_y = title_y + title_h + 2.5_f32;
        let desc_max_w = (rect.max.x - 10.0_f32 - text_left).max(32.0_f32);
        let _ = paint_wrapped_text(
            ui,
            &p,
            Pos2::new(text_left, desc_y),
            desc_max_w,
            desc,
            FontId::proportional(11.0_f32),
            ui.visuals().weak_text_color(),
            2,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn render_font_section(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        font_group_id: Id,
        avail_w: f32,
        french: bool,
        current_font_size: f32,
        interaction_blocked: bool,
    ) -> Option<AppearanceAction> {
        Self::render_group_heading(
            ui,
            avail_w,
            font_group_title(french),
            Some(font_group_desc(french)),
        );

        let strong = ui.visuals().strong_text_color();
        let dim = ui.visuals().weak_text_color();
        let card_bg = ui.visuals().widgets.inactive.bg_fill;
        let card_border = ui.visuals().widgets.noninteractive.bg_stroke.color;
        let accent = ui.visuals().selection.stroke.color;

        let inner_pad_x = if avail_w < 340.0_f32 {
            12.0_f32
        } else {
            18.0_f32
        };
        let content_w = (avail_w - inner_pad_x * 2.0_f32).max(96.0_f32);

        let stepper_w = 108.0_f32;
        let show_slider = content_w >= 220.0_f32;
        let slider_w = if show_slider {
            (content_w - stepper_w - 10.0_f32).clamp(84.0_f32, 150.0_f32)
        } else {
            0.0_f32
        };
        let controls_w = if show_slider {
            slider_w + 10.0_f32 + stepper_w
        } else {
            stepper_w
        };
        let controls_h = 28.0_f32;

        let stacked = avail_w < 540.0_f32;
        let label_max_w = if stacked {
            content_w
        } else {
            (content_w - controls_w - 16.0_f32).max(110.0_f32)
        };

        let title_g = layout_wrapped_galley(
            ui,
            label_max_w,
            font_row_title(french),
            FontId::proportional(13.0_f32),
            strong,
            2,
        );
        let hint_g = layout_wrapped_galley(
            ui,
            label_max_w,
            font_row_hint(french),
            FontId::proportional(11.5_f32),
            dim,
            2,
        );
        let label_h = title_g.size().y + 3.0_f32 + hint_g.size().y;

        let card_h = if stacked {
            14.0_f32 + label_h + 10.0_f32 + controls_h + 14.0_f32
        } else {
            (label_h + 24.0_f32).max(58.0_f32)
        };

        let (_, card_rect) = ui.allocate_space(Vec2::new(avail_w, card_h));
        let group_resp = ui.interact(card_rect, font_group_id, Sense::focusable_noninteractive());
        let group_focused = ui.is_enabled() && ui.ctx().memory(|m| m.has_focus(font_group_id));

        let controls_origin = if stacked {
            Pos2::new(
                card_rect.min.x + inner_pad_x,
                card_rect.max.y - 14.0_f32 - controls_h,
            )
        } else {
            Pos2::new(
                card_rect.max.x - inner_pad_x - controls_w,
                card_rect.center().y - controls_h * 0.5_f32,
            )
        };

        let slider_rect = Rect::from_min_size(controls_origin, Vec2::new(slider_w, controls_h));
        let stepper_x = if show_slider {
            slider_rect.max.x + 10.0_f32
        } else {
            controls_origin.x
        };
        let stepper_rect = Rect::from_min_size(
            Pos2::new(stepper_x, controls_origin.y),
            Vec2::new(stepper_w, controls_h),
        );

        let minus_rect = Rect::from_min_size(stepper_rect.min, Vec2::new(30.0_f32, controls_h));
        let plus_rect = Rect::from_min_size(
            Pos2::new(stepper_rect.max.x - 30.0_f32, stepper_rect.min.y),
            Vec2::new(30.0_f32, controls_h),
        );
        let val_rect = Rect::from_min_max(
            Pos2::new(minus_rect.max.x, stepper_rect.min.y),
            Pos2::new(plus_rect.min.x, stepper_rect.max.y),
        );

        let can_shrink = ui.is_enabled() && current_font_size > MIN_TERMINAL_FONT_SIZE + 1e-4_f32;
        let can_grow = ui.is_enabled() && current_font_size < MAX_TERMINAL_FONT_SIZE - 1e-4_f32;

        let minus_resp = ui.interact(minus_rect, panel_id.with("font-minus"), Sense::CLICK);
        minus_resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, can_shrink, smaller_label(french))
        });
        let plus_resp = ui.interact(plus_rect, panel_id.with("font-plus"), Sense::CLICK);
        plus_resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, can_grow, bigger_label(french))
        });

        let slider_resp = if show_slider {
            let r = ui.interact(
                slider_rect,
                panel_id.with("font-slider"),
                Sense::CLICK | Sense::DRAG,
            );
            let slider_lbl = format!(
                "{} ({})",
                font_row_title(french),
                format_font_px(current_font_size)
            );
            r.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Slider, ui.is_enabled(), &slider_lbl)
            });
            Some(r)
        } else {
            self.dragging_slider = false;
            None
        };

        let cp = ui.painter().with_clip_rect(card_rect);
        let outer_stroke = if group_focused {
            Stroke::new(1.5_f32, accent)
        } else {
            Stroke::new(1.0_f32, card_border)
        };
        cp.rect(
            card_rect,
            10.0_f32,
            card_bg,
            outer_stroke,
            StrokeKind::Inside,
        );

        let label_y = if stacked {
            card_rect.min.y + 14.0_f32
        } else {
            card_rect.center().y - label_h * 0.5_f32
        };
        let hint_y = label_y + title_g.size().y + 3.0_f32;
        cp.galley(
            Pos2::new(card_rect.min.x + inner_pad_x, label_y),
            title_g,
            strong,
        );
        cp.galley(
            Pos2::new(card_rect.min.x + inner_pad_x, hint_y),
            hint_g,
            dim,
        );

        // Paint `.set-range` slider (`style.css:3868-3871`).
        let mut action = None;
        if let Some(s_resp) = &slider_resp {
            let slider_allowed = !interaction_blocked && !self.slider_gesture_cancelled;
            if slider_allowed
                && ((s_resp.is_pointer_button_down_on()
                    && ui.input(|i| i.pointer.primary_pressed()))
                    || s_resp.drag_started())
            {
                self.dragging_slider = true;
                ui.ctx().memory_mut(|m| m.request_focus(font_group_id));
            }
            let pointer_active =
                slider_allowed && (s_resp.clicked() || self.dragging_slider || s_resp.dragged());
            if pointer_active && let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                ui.ctx().memory_mut(|m| m.request_focus(font_group_id));
                let pad = 7.0_f32;
                let usable_w = (slider_rect.width() - pad * 2.0_f32).max(1.0_f32);
                let ratio =
                    ((pos.x - (slider_rect.min.x + pad)) / usable_w).clamp(0.0_f32, 1.0_f32);
                let raw = MIN_TERMINAL_FONT_SIZE
                    + ratio * (MAX_TERMINAL_FONT_SIZE - MIN_TERMINAL_FONT_SIZE);
                let snapped = clamp_font_size(raw.round());
                if (snapped - current_font_size).abs() > 1e-4_f32 {
                    action = Some(AppearanceAction::TerminalFontSize(snapped));
                }
            }
            if ui.input(|i| i.pointer.any_released() || !i.pointer.primary_down()) {
                self.dragging_slider = false;
            }

            let pad = 7.0_f32;
            let track_left = slider_rect.min.x + pad;
            let track_right = (slider_rect.max.x - pad).max(track_left + 4.0_f32);
            let cy = slider_rect.center().y;
            let ratio = ((current_font_size - MIN_TERMINAL_FONT_SIZE)
                / (MAX_TERMINAL_FONT_SIZE - MIN_TERMINAL_FONT_SIZE))
                .clamp(0.0_f32, 1.0_f32);
            let thumb_x = track_left + ratio * (track_right - track_left);

            let full_track = Rect::from_min_max(
                Pos2::new(track_left, cy - 2.0_f32),
                Pos2::new(track_right, cy + 2.0_f32),
            );
            let active_track = Rect::from_min_max(
                Pos2::new(track_left, cy - 2.0_f32),
                Pos2::new(thumb_x.max(track_left + 1.0_f32), cy + 2.0_f32),
            );
            cp.rect_filled(full_track, 2.0_f32, ui.visuals().extreme_bg_color);
            cp.rect_filled(active_track, 2.0_f32, accent);
            let thumb_r = if s_resp.hovered() || self.dragging_slider {
                6.5_f32
            } else {
                5.5_f32
            };
            cp.circle_filled(Pos2::new(thumb_x, cy), thumb_r, accent);
        }

        // Paint `.set-stepper` (`style.css:3820-3846`).
        cp.rect(
            stepper_rect,
            6.0_f32,
            ui.visuals().extreme_bg_color,
            Stroke::new(1.0_f32, card_border),
            StrokeKind::Inside,
        );
        if can_shrink && minus_resp.hovered() {
            cp.rect_filled(
                minus_rect.shrink(1.0_f32),
                5.0_f32,
                ui.visuals().widgets.hovered.bg_fill,
            );
        }
        if can_grow && plus_resp.hovered() {
            cp.rect_filled(
                plus_rect.shrink(1.0_f32),
                5.0_f32,
                ui.visuals().widgets.hovered.bg_fill,
            );
        }
        cp.line_segment(
            [
                Pos2::new(minus_rect.max.x, stepper_rect.min.y + 1.0_f32),
                Pos2::new(minus_rect.max.x, stepper_rect.max.y - 1.0_f32),
            ],
            Stroke::new(1.0_f32, card_border),
        );
        cp.line_segment(
            [
                Pos2::new(plus_rect.min.x, stepper_rect.min.y + 1.0_f32),
                Pos2::new(plus_rect.min.x, stepper_rect.max.y - 1.0_f32),
            ],
            Stroke::new(1.0_f32, card_border),
        );

        let minus_fg = if !can_shrink {
            dim.gamma_multiply(0.4_f32)
        } else if minus_resp.hovered() {
            strong
        } else {
            ui.visuals().text_color()
        };
        let plus_fg = if !can_grow {
            dim.gamma_multiply(0.4_f32)
        } else if plus_resp.hovered() {
            strong
        } else {
            ui.visuals().text_color()
        };
        paint_plus_or_minus(
            &cp,
            Rect::from_center_size(minus_rect.center(), Vec2::splat(14.0_f32)),
            minus_fg,
            false,
        );
        paint_plus_or_minus(
            &cp,
            Rect::from_center_size(plus_rect.center(), Vec2::splat(14.0_f32)),
            plus_fg,
            true,
        );

        let val_str = format_font_px(current_font_size);
        let val_g = cp.layout_no_wrap(val_str.clone(), FontId::proportional(12.0_f32), strong);
        cp.galley(
            Pos2::new(
                val_rect.center().x - val_g.size().x * 0.5_f32,
                val_rect.center().y - val_g.size().y * 0.5_f32,
            ),
            val_g,
            strong,
        );

        let minus_resp = if can_shrink {
            minus_resp.on_hover_text(smaller_label(french))
        } else {
            minus_resp
        };
        let plus_resp = if can_grow {
            plus_resp.on_hover_text(bigger_label(french))
        } else {
            plus_resp
        };

        if !interaction_blocked && can_shrink && minus_resp.clicked() {
            ui.ctx().memory_mut(|m| m.request_focus(font_group_id));
            let next = clamp_font_size(current_font_size - 1.0_f32);
            if (next - current_font_size).abs() > 1e-4_f32 {
                action = Some(AppearanceAction::TerminalFontSize(next));
            }
        } else if !interaction_blocked && can_grow && plus_resp.clicked() {
            ui.ctx().memory_mut(|m| m.request_focus(font_group_id));
            let next = clamp_font_size(current_font_size + 1.0_f32);
            if (next - current_font_size).abs() > 1e-4_f32 {
                action = Some(AppearanceAction::TerminalFontSize(next));
            }
        }

        let group_a11y = format!("{} — {val_str}", font_row_title(french));
        group_resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), &group_a11y)
        });

        action
    }
}
