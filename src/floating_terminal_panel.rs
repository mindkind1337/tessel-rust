//! Pure `egui` top-overlay `FloatingTerminal` panel (`Ctrl+\`` / toolbar toggle).
//!
//! Transcribed directly from:
//! - `src/renderer/src/floatingTerminal.js` (`MIN_HEIGHT`, `MAX_HEIGHT`, `DEFAULT_HEIGHT`,
//!   `clampHeight`, `isFloatingToggleKey`, `createFloatingTerminal`)
//! - `src/renderer/src/components/FloatingTerminal.vue` (top workspace overlay — instant mount/unmount
//!   without Vue's `.16s` CSS slide transition — `10px` bottom drag handle with `.10..=.95` live
//!   preview before `.15..=.90` commit, `±5%` keyboard step, `Hide` chevron button, and focus
//!   capture/restoration lifecycle)
//! - `src/renderer/src/App.vue` (`14189-14217`, `15014-15029`, `15203`: toolbar toggle SVG,
//!   `floatingPaneKey` shortcut isolation, and `Escape` passthrough to the shell)
//!
//! # Architectural Boundaries
//! - **Zero Process / PTY Ownership:** Never spawns, owns, writes to, or kills a `Terminal` or
//!   daemon session. Renders caller-supplied terminal UI inside [`FloatingBodyContext`] and emits
//!   explicit [`FloatingTerminalAction`] intents (`Show`, `Hide`, `Close`, `Restart`,
//!   `CopyOutput`, `HeightChanged`, `FocusTerminal`, `RestoreFocus`).
//! - **Outside the Workspace Pane Tree:** Lives in its own overlay state so workspace split trees,
//!   sidebar lists, and saved grid layouts never include the floating terminal.

use crate::icons::{self, Icon};
use crate::palette;
use crate::pane_menu_model::{
    PaneActionsMenuInput, PaneMenuAction, PaneMenuEntry, PaneMenuLang, build_pane_actions_menu,
};
use crate::shortcuts::{
    AppKeyContext, AppShortcut, KeyInput, is_floating_toggle_key, route_app_key,
};
use crate::theme_palettes::UiPalette;
use eframe::egui::{
    self, Align, Area, Color32, CursorIcon, Event, FontId, Id, Key, Layout, Modifiers, Order,
    Painter, PointerButton, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo,
    WidgetType,
};
use serde::{Deserialize, Serialize};
use std::hash::Hash;

pub use crate::palette::{
    FLOATING_DEFAULT_HEIGHT, FLOATING_MAX_HEIGHT, FLOATING_MIN_HEIGHT, clamp_floating_height,
};

/// Keyboard resize step on the bottom handle (`FloatingTerminal.vue:88`: `±0.05`).
pub const FLOATING_KEYBOARD_STEP: f64 = 0.05;
/// Height of `.floating-term-handle` (`FloatingTerminal.vue:244`: `10px`).
pub const FLOATING_HANDLE_HEIGHT: f32 = 10.0;
/// Height of the floating terminal's own pane header bar (`28px`).
pub const FLOATING_HEADER_HEIGHT: f32 = 28.0;

/// Live pointer-drag preview bounds before release clamp (`FloatingTerminal.vue:74`: `0.1..0.95`).
const FLOATING_DRAG_PREVIEW_MIN: f64 = 0.10;
const FLOATING_DRAG_PREVIEW_MAX: f64 = 0.95;
const MAX_TITLE_DISPLAY_CHARS: usize = 96;
const TOOLBAR_TOGGLE_ID_SALT: &str = "floating_terminal_toolbar_toggle";

#[derive(Clone, Debug, Deserialize)]
struct RawFloatingTerminalState {
    #[serde(default)]
    open: bool,
    #[serde(default = "default_floating_height")]
    height: f64,
}

const fn default_floating_height() -> f64 {
    FLOATING_DEFAULT_HEIGHT
}

fn clamp_drag_preview(height: f64) -> f64 {
    if !height.is_finite() {
        return FLOATING_DEFAULT_HEIGHT;
    }
    height.clamp(FLOATING_DRAG_PREVIEW_MIN, FLOATING_DRAG_PREVIEW_MAX)
}

/// Pure UI and focus-lifecycle state for the slide-down floating terminal.
///
/// Persists only `{ open, height }` when serialized; runtime drag and focus-restoration state is
/// skipped and rebuilt deterministically (`keyboard = open` on reload, matching
/// `floatingTerminal.js:70`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawFloatingTerminalState")]
pub struct FloatingTerminalState {
    pub open: bool,
    pub height: f64,
    #[serde(skip)]
    pub keyboard: bool,
    #[serde(skip)]
    drag_height: Option<f64>,
    #[serde(skip)]
    resizing_drag_active: bool,
    #[serde(skip)]
    saved_focus: Option<Id>,
    #[serde(skip)]
    pre_toggle_focus: Option<Option<Id>>,
    #[serde(skip)]
    keyboard_before_hide: Option<bool>,
    #[serde(skip)]
    last_observed_focus: Option<Id>,
    #[serde(skip)]
    body_child_focus: Option<Id>,
    #[serde(skip)]
    was_open: bool,
    #[serde(skip)]
    had_terminal: bool,
    #[serde(skip)]
    was_transient_overlay_open: bool,
    #[serde(skip)]
    pending_focus_terminal: bool,
}

impl From<RawFloatingTerminalState> for FloatingTerminalState {
    fn from(raw: RawFloatingTerminalState) -> Self {
        Self::from_saved(raw.open, raw.height)
    }
}

impl Default for FloatingTerminalState {
    fn default() -> Self {
        Self::new()
    }
}

impl FloatingTerminalState {
    #[must_use]
    pub fn new() -> Self {
        Self::from_saved(false, FLOATING_DEFAULT_HEIGHT)
    }

    /// Restores `{ open, height }` from saved preferences (`floatingTerminal.js:60-71`).
    ///
    /// Clamps `height` into `[0.15, 0.90]` (or `0.45` if non-finite) and gives keyboard ownership
    /// to the floating terminal when it starts open after a reload (`keyboard = open`).
    #[must_use]
    pub fn from_saved(open: bool, height: f64) -> Self {
        Self {
            open,
            height: palette::clamp_floating_height(height),
            keyboard: open,
            drag_height: None,
            resizing_drag_active: false,
            saved_focus: None,
            pre_toggle_focus: None,
            keyboard_before_hide: None,
            last_observed_focus: None,
            body_child_focus: None,
            was_open: open,
            had_terminal: false,
            was_transient_overlay_open: false,
            pending_focus_terminal: open,
        }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Share of workspace height currently displayed (`0.10..=0.95` while dragging the handle,
    /// otherwise committed `0.15..=0.90`).
    #[must_use]
    pub fn shown_height(&self) -> f64 {
        self.drag_height
            .map(clamp_drag_preview)
            .unwrap_or_else(|| clamp_floating_height(self.height))
    }

    #[must_use]
    pub fn is_resizing(&self) -> bool {
        self.resizing_drag_active || self.drag_height.is_some()
    }

    /// `true` when the floating terminal is shown and owns keyboard routing
    /// (`floatingTerminal.js:hasKeyboard`).
    #[must_use]
    pub fn has_keyboard(&self) -> bool {
        self.open && self.keyboard
    }

    pub fn show(&mut self) {
        self.open = true;
        self.keyboard = true;
        self.keyboard_before_hide = None;
        self.pending_focus_terminal = true;
    }

    pub fn hide(&mut self) {
        if self.keyboard_before_hide.is_none() {
            self.keyboard_before_hide = Some(self.keyboard);
        }
        self.open = false;
        self.keyboard = false;
        self.drag_height = None;
        self.resizing_drag_active = false;
        self.pending_focus_terminal = false;
        self.body_child_focus = None;
    }

    pub fn toggle(&mut self) -> bool {
        if self.open {
            self.hide();
        } else {
            self.show();
        }
        self.open
    }

    /// Closes the floating terminal (`Ctrl+Shift+W` or header `×`), hiding the panel so the caller
    /// stops its PTY (`floatingTerminal.js:close`).
    pub fn close(&mut self) {
        if self.keyboard_before_hide.is_none() {
            self.keyboard_before_hide = Some(self.keyboard);
        }
        self.open = false;
        self.keyboard = false;
        self.drag_height = None;
        self.resizing_drag_active = false;
        self.pending_focus_terminal = false;
        self.body_child_focus = None;
        self.had_terminal = false;
    }

    pub fn set_height(&mut self, height: f64) -> f64 {
        let clamped = clamp_floating_height(height);
        self.height = clamped;
        clamped
    }

    pub fn step_height(&mut self, delta: f64) -> f64 {
        let base = clamp_floating_height(self.height);
        self.set_height(base + delta)
    }

    /// Give focus to another pane without hiding or restarting this terminal.
    pub fn yield_keyboard(&mut self) {
        self.keyboard = false;
        self.pending_focus_terminal = false;
    }
    pub fn set_keyboard(&mut self, has_keyboard: bool) {
        self.keyboard = has_keyboard;
    }
}

/// Frame inputs supplied by `App` when rendering [`FloatingTerminalPanel`].
#[derive(Clone, Copy, Debug)]
pub struct FloatingTerminalInput<'a> {
    pub workspace_rect: Rect,
    pub title: &'a str,
    pub has_terminal: bool,
    pub starting: bool,
    pub can_restart: bool,
    pub has_output: bool,
    pub dialog_open: bool,
    pub typing_in_field: bool,
    pub french: bool,
    pub palette: UiPalette,
    pub content_focus_id: Option<Id>,
    pub toggle_button_id: Option<Id>,
}

/// Geometry and input-permission context passed to the caller's terminal body closure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatingBodyContext {
    pub rect: Rect,
    pub accepts_input: bool,
    pub has_keyboard: bool,
    pub focus_id: Id,
}

/// Pure intents emitted by [`FloatingTerminalPanel`] for `App` to execute.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FloatingTerminalAction {
    Show,
    Hide,
    Close,
    Restart,
    CopyOutput,
    HeightChanged(f64),
    FocusTerminal,
    RestoreFocus(Option<Id>),
}

/// Per-frame output returned by [`FloatingTerminalPanel::show`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FloatingTerminalOutput {
    pub actions: Vec<FloatingTerminalAction>,
    pub panel_rect: Option<Rect>,
    pub body_rect: Option<Rect>,
    pub has_keyboard: bool,
}

/// Stateless renderer and shortcut filter for Tessel's slide-down floating terminal.
pub struct FloatingTerminalPanel;

impl FloatingTerminalPanel {
    /// Renders the exact `16x16` toolbar toggle button (`App.vue:15014-15029`).
    pub fn toolbar_button(ui: &mut Ui, open: bool, french: bool) -> Response {
        let label = if french {
            "Terminal flottant"
        } else {
            "Floating terminal"
        };
        let hint = if french {
            "Terminal flottant (Ctrl+`)"
        } else {
            "Floating terminal (Ctrl+`)"
        };
        let id = Id::new(TOOLBAR_TOGGLE_ID_SALT);
        let (_, rect) = ui.allocate_space(Vec2::new(30.0, 28.0));
        let response = ui.interact(rect, id, Sense::click());
        response
            .widget_info(|| WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), open, label));

        let visuals = ui.style().interact_selectable(&response, open);
        if open || response.hovered() {
            ui.painter().rect_filled(rect, 6.0, visuals.bg_fill);
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.shrink(0.5),
                6.0,
                ui.visuals().selection.stroke,
                StrokeKind::Inside,
            );
        }
        let color = if !ui.is_enabled() {
            ui.visuals().weak_text_color().gamma_multiply(0.5)
        } else if response.hovered() || open {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        Self::paint_toolbar_icon(
            ui.painter(),
            Rect::from_center_size(rect.center(), Vec2::splat(16.0)),
            open,
            color,
        );
        response.on_hover_text(hint)
    }

    /// Paints the exact `16x16` floating-terminal icon (`App.vue:15023-15028`):
    /// filled top drawer when `open`, horizontal split + `>` prompt chevron when closed.
    pub fn paint_toolbar_icon(painter: &Painter, rect: Rect, open: bool, color: Color32) {
        if !rect.is_positive() || !rect.is_finite() {
            return;
        }
        let side = rect.width().min(rect.height());
        if side <= 0.0 {
            return;
        }
        let sq = Rect::from_center_size(rect.center(), Vec2::splat(side));
        let s = side / 16.0;
        let pt = |x: f32, y: f32| Pos2::new(sq.min.x + x * s, sq.min.y + y * s);
        let r_xywh =
            |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pt(x, y), Vec2::new(w * s, h * s));

        let outer_stroke = Stroke::new(1.3_f32 * s, color);
        painter.rect_stroke(
            r_xywh(1.75, 2.25, 12.5, 11.5),
            2.25 * s,
            outer_stroke,
            StrokeKind::Middle,
        );
        if open {
            painter.rect_filled(r_xywh(2.4, 2.9, 11.2, 5.2), 1.4 * s, color);
        } else {
            painter.line_segment([pt(2.5, 8.5), pt(13.5, 8.5)], outer_stroke);
            let prompt_stroke = Stroke::new(1.2_f32 * s, color);
            let p0 = pt(4.6, 4.6);
            let p1 = pt(6.1, 5.9);
            let p2 = pt(4.6, 7.2);
            painter.add(egui::Shape::line(vec![p0, p1, p2], prompt_stroke));
            let cap_r = (0.6 * s).max(0.4);
            painter.circle_filled(p0, cap_r, color);
            painter.circle_filled(p1, cap_r, color);
            painter.circle_filled(p2, cap_r, color);
        }
    }

    /// Converts an `egui` key event into [`KeyInput`] for shared [`crate::shortcuts`] routing,
    /// preserving physical `Backquote` on French AZERTY (`²`) and Canadian French (`#`) layouts.
    #[must_use]
    pub fn key_input_from_egui(
        key: Key,
        physical_key: Option<Key>,
        modifiers: Modifiers,
        repeat: bool,
    ) -> KeyInput {
        let logical = match key {
            Key::Backtick => "`".to_owned(),
            Key::Escape => "Escape".to_owned(),
            Key::F1 => "F1".to_owned(),
            Key::Space => " ".to_owned(),
            Key::Comma => ",".to_owned(),
            Key::Equals => "=".to_owned(),
            Key::Plus => "+".to_owned(),
            Key::Minus => "-".to_owned(),
            Key::Num0 => "0".to_owned(),
            Key::PageUp => "PageUp".to_owned(),
            Key::PageDown => "PageDown".to_owned(),
            Key::ArrowLeft => "ArrowLeft".to_owned(),
            Key::ArrowRight => "ArrowRight".to_owned(),
            Key::ArrowUp => "ArrowUp".to_owned(),
            Key::ArrowDown => "ArrowDown".to_owned(),
            _ => {
                let name = key.name();
                if name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic() {
                    name.to_ascii_lowercase()
                } else {
                    name.to_owned()
                }
            }
        };
        let code = match physical_key {
            Some(Key::Backtick) => "Backquote".to_owned(),
            Some(Key::Num0) => "Digit0".to_owned(),
            Some(Key::Plus) => "NumpadAdd".to_owned(),
            Some(Key::Minus) => "NumpadSubtract".to_owned(),
            Some(Key::Z) => "KeyZ".to_owned(),
            Some(pk) => pk.name().to_owned(),
            None => String::new(),
        };
        KeyInput {
            key: logical,
            code,
            ctrl: modifiers.ctrl,
            shift: modifiers.shift,
            alt: modifiers.alt,
            meta: modifiers.mac_cmd,
            repeat,
        }
    }

    /// Intercepts `Ctrl+\`` and floating-scoped shortcuts in `ctx.input_mut` BEFORE `App`
    /// processes workspace pane-grid shortcuts (`App.vue:14189-14217`).
    ///
    /// - `Ctrl+\`` (when `!dialog_open`): consumed; toggles panel on non-repeat (`Show` / `Hide`).
    /// - When `state.has_keyboard() && !dialog_open && !typing_in_field`:
    ///   - `Ctrl+Shift+W`: consumed; closes floating terminal (`Close`), never grid active pane.
    ///   - `Ctrl+Shift+R`: consumed; restarts floating terminal (`Restart`), never grid active pane.
    ///   - `Ctrl+Shift+{E,O,B}` and `Alt+Arrows`: consumed no-op so background split tree and
    ///     broadcast state are untouched and the PTY receives no stray sequences.
    ///   - `Escape`: **never** consumed here, leaving `Key::Escape` intact for `vi`/`less`/shell.
    pub fn handle_shortcuts(
        ctx: &egui::Context,
        state: &mut FloatingTerminalState,
        dialog_open: bool,
        typing_in_field: bool,
    ) -> Vec<FloatingTerminalAction> {
        let current_focus = ctx.memory(|m| m.focused());
        let mut actions = Vec::new();
        let mut close_popups = false;

        ctx.input_mut(|input| {
            input.events.retain(|event| {
                let Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    repeat,
                    modifiers,
                } = event
                else {
                    return true;
                };

                let ki = Self::key_input_from_egui(*key, *physical_key, *modifiers, *repeat);
                if is_floating_toggle_key(&ki) && !dialog_open {
                    if !*repeat {
                        if state.pre_toggle_focus.is_none() {
                            state.pre_toggle_focus = Some(current_focus);
                        }
                        close_popups = true;
                        let now_open = state.toggle();
                        actions.push(if now_open {
                            FloatingTerminalAction::Show
                        } else {
                            FloatingTerminalAction::Hide
                        });
                    }
                    return false;
                }

                if !dialog_open && !typing_in_field && state.has_keyboard() {
                    let routed = route_app_key(
                        &ki,
                        AppKeyContext {
                            in_floating_terminal: true,
                            dialog_open: false,
                            typing_in_field: false,
                            ..Default::default()
                        },
                    );
                    match routed {
                        AppShortcut::FloatingClose => {
                            if !*repeat {
                                if state.pre_toggle_focus.is_none() {
                                    state.pre_toggle_focus = Some(current_focus);
                                }
                                state.close();
                                actions.push(FloatingTerminalAction::Close);
                            }
                            return false;
                        }
                        AppShortcut::FloatingRestart => {
                            if !*repeat {
                                state.set_keyboard(true);
                                state.pending_focus_terminal = true;
                                actions.push(FloatingTerminalAction::Restart);
                            }
                            return false;
                        }
                        AppShortcut::ConsumedNoop => {
                            return false;
                        }
                        _ => {}
                    }
                }

                true
            });
        });

        if close_popups {
            egui::Popup::close_all(ctx);
        }

        actions
    }

    /// Renders the slide-down floating terminal overlay over `input.workspace_rect` and
    /// synchronizes its focus capture/restoration state machine.
    ///
    /// Call every frame (even when `!state.is_open()`) so hiding via `Ctrl+\`` or toolbar toggle
    /// restores focus immediately without rendering any off-screen geometry.
    pub fn show(
        ctx: &egui::Context,
        id_source: impl Hash,
        state: &mut FloatingTerminalState,
        input: FloatingTerminalInput<'_>,
        render_body: impl FnOnce(&mut Ui, FloatingBodyContext),
    ) -> FloatingTerminalOutput {
        let base_id = Id::new(id_source);
        let ids = PanelIds::new(base_id, input.content_focus_id, input.toggle_button_id);
        let mut out = FloatingTerminalOutput::default();

        // Reconcile external open/close transitions before painting.
        Self::sync_focus_transitions(ctx, state, &ids, &mut out.actions);

        if !state.is_open() {
            state.was_transient_overlay_open = input.dialog_open || egui::Popup::is_any_open(ctx);
            state.had_terminal = input.has_terminal;
            state.last_observed_focus = ctx.memory(|m| m.focused());
            out.has_keyboard = false;
            return out;
        }

        let ws = input.workspace_rect;
        if !ws.is_positive() || !ws.is_finite() {
            out.has_keyboard = state.has_keyboard();
            return out;
        }

        // Update ongoing pointer drag before computing `panel_rect` for zero-frame-lag preview.
        if state.resizing_drag_active {
            if input.dialog_open {
                state.resizing_drag_active = false;
                state.drag_height = None;
            } else {
                let (primary_down, pos) = ctx.input(|i| {
                    (
                        i.pointer.primary_down(),
                        i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()),
                    )
                });
                if let Some(p) = pos {
                    let raw = f64::from((p.y - ws.top()) / ws.height());
                    state.drag_height = Some(clamp_drag_preview(raw));
                }
                if !primary_down {
                    state.resizing_drag_active = false;
                    let final_raw = state.drag_height.take().unwrap_or(state.height);
                    let committed = state.set_height(final_raw);
                    out.actions
                        .push(FloatingTerminalAction::HeightChanged(committed));
                }
            }
        }

        let share = state.shown_height() as f32;
        let panel_h = (ws.height() * share)
            .round()
            .clamp(1.0_f32.min(ws.height()), ws.height());
        let panel_rect = Rect::from_min_size(ws.min, Vec2::new(ws.width(), panel_h));

        let handle_h = FLOATING_HANDLE_HEIGHT.min(panel_rect.height());
        let rem_after_handle = (panel_rect.height() - handle_h).max(0.0);
        let header_h = FLOATING_HEADER_HEIGHT.min(rem_after_handle);
        let body_h = (rem_after_handle - header_h).max(0.0);

        let header_rect =
            Rect::from_min_size(panel_rect.min, Vec2::new(panel_rect.width(), header_h));
        let body_rect = Rect::from_min_size(
            Pos2::new(panel_rect.left(), header_rect.bottom()),
            Vec2::new(panel_rect.width(), body_h),
        );
        let handle_rect = Rect::from_min_size(
            Pos2::new(panel_rect.left(), body_rect.bottom()),
            Vec2::new(panel_rect.width(), handle_h),
        );

        let mut request_hide = false;
        let mut request_close = false;
        let mut request_restart = false;
        let mut request_copy_output = false;
        let mut clicked_inside_panel = false;
        let mut menu_open = false;
        let mut area_was_sizing_pass = false;

        Area::new(ids.area)
            .order(Order::Middle)
            .fixed_pos(panel_rect.min)
            .constrain(false)
            .interactable(!input.dialog_open)
            .show(ctx, |ui| {
                area_was_sizing_pass = ui.is_sizing_pass();
                if input.dialog_open {
                    ui.disable();
                }
                ui.set_clip_rect(ws);
                ui.set_min_size(panel_rect.size());
                ui.set_max_size(panel_rect.size());
                let _ = ui.allocate_rect(panel_rect, Sense::hover());

                // Subtle drop shadow below `.floating-term` (`0 10px 28px rgba(0, 0, 0, 0.35)`).
                let shadow_painter = ui.painter().with_clip_rect(ws);
                for (dy, alpha) in [(3.0_f32, 56_u8), (8.0_f32, 32_u8), (16.0_f32, 14_u8)] {
                    let y0 = panel_rect.bottom();
                    let y1 = (panel_rect.bottom() + dy).min(ws.bottom());
                    if y1 > y0 {
                        shadow_painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(panel_rect.left(), y0),
                                Pos2::new(panel_rect.right(), y1),
                            ),
                            0.0,
                            Color32::from_rgba_unmultiplied(0, 0, 0, alpha),
                        );
                    }
                }

                // Panel background (`var(--term)`) & bottom border (`var(--border-strong)`).
                let bg_resp = ui.interact(panel_rect, ids.bg, Sense::click());
                let region_label = if input.french {
                    "Terminal flottant"
                } else {
                    "Floating terminal"
                };
                bg_resp.widget_info(|| {
                    WidgetInfo::labeled(WidgetType::Panel, ui.is_enabled(), region_label)
                });
                if bg_resp.clicked() {
                    clicked_inside_panel = true;
                }

                let p = ui.painter().with_clip_rect(panel_rect);
                p.rect_filled(panel_rect, 0.0, input.palette.term);

                // 1. Floating Pane Header (no Maximize, no Split, no Broadcast write pill).
                if header_rect.height() >= 16.0 {
                    p.rect_filled(header_rect, 0.0, input.palette.surface);
                    p.line_segment(
                        [
                            Pos2::new(header_rect.left(), header_rect.bottom() - 0.5),
                            Pos2::new(header_rect.right(), header_rect.bottom() - 0.5),
                        ],
                        Stroke::new(1.0_f32, input.palette.border),
                    );

                    let header_click =
                        ui.interact(header_rect, ids.header_bar, Sense::click());
                    if header_click.clicked() {
                        clicked_inside_panel = true;
                    }

                    let default_title = if input.french {
                        "Terminal flottant"
                    } else {
                        "Floating terminal"
                    };
                    let clean_title = sanitize_single_line(input.title, default_title);
                    let buttons_w = 56.0_f32.min(header_rect.width());
                    let close_rect = Rect::from_center_size(
                        Pos2::new(header_rect.right() - 16.0, header_rect.center().y),
                        Vec2::new(24.0, 22.0),
                    )
                    .intersect(header_rect);
                    let more_rect = Rect::from_center_size(
                        Pos2::new(header_rect.right() - 42.0, header_rect.center().y),
                        Vec2::new(24.0, 22.0),
                    )
                    .intersect(header_rect);

                    let title_right = (header_rect.right() - buttons_w - 6.0)
                        .max(header_rect.left() + 10.0);
                    let title_rect = Rect::from_min_max(
                        Pos2::new(header_rect.left() + 10.0, header_rect.top()),
                        Pos2::new(title_right, header_rect.bottom()),
                    );
                    if title_rect.width() > 8.0 {
                        let dot_color = if state.has_keyboard() {
                            input.palette.accent
                        } else {
                            input.palette.border_strong
                        };
                        let dot_center = Pos2::new(title_rect.left() + 3.5, title_rect.center().y);
                        p.circle_filled(dot_center, 3.0, dot_color);

                        let text_left = (title_rect.left() + 12.0).min(title_rect.right());
                        let text_clip = Rect::from_min_max(
                            Pos2::new(text_left, title_rect.top()),
                            title_rect.max,
                        );
                        if text_clip.width() > 4.0 {
                            let title_color = if state.has_keyboard() {
                                input.palette.text_strong
                            } else {
                                input.palette.text_dim
                            };
                            let mut job = egui::text::LayoutJob::simple(
                                clean_title.clone(),
                                FontId::proportional(12.0),
                                title_color,
                                text_clip.width(),
                            );
                            job.wrap.max_rows = 1;
                            job.wrap.break_anywhere = true;
                            job.wrap.overflow_character = Some('…');
                            let galley = p.layout_job(job);
                            let ty = header_rect.center().y - galley.size().y * 0.5;
                            p.with_clip_rect(text_clip)
                                .galley(Pos2::new(text_left, ty), galley, title_color);
                        }
                    }

                    if more_rect.width() >= 14.0 {
                        let more_label = if input.french {
                            "Plus d'options"
                        } else {
                            "More options"
                        };
                        let more_hint = if input.french {
                            "Plus d'options (ou Maj+clic droit dans le panneau)"
                        } else {
                            "More options (or Shift+right-click in the pane)"
                        };
                        let more_resp =
                            paint_header_icon_btn(ui, more_rect, ids.header_more, Icon::PaneMore, more_label, more_hint, input.palette, false);
                        if more_resp.clicked() {
                            clicked_inside_panel = true;
                        }
                        let popup = egui::Popup::menu(&more_resp)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
                        let shown = popup.show(|menu_ui| {
                            if let Some(menu_action) = show_floating_more_menu(
                                menu_ui,
                                &clean_title,
                                input.has_output,
                                input.can_restart,
                                input.french,
                            ) {
                                match menu_action {
                                    PaneMenuAction::CopyOutput => request_copy_output = true,
                                    PaneMenuAction::Restart => request_restart = true,
                                    PaneMenuAction::ClosePane => request_close = true,
                                    _ => {}
                                }
                            }
                        });
                        menu_open |= shown.is_some();
                    }

                    if close_rect.width() >= 14.0 {
                        let close_label = if input.french {
                            "Fermer le panneau"
                        } else {
                            "Close pane"
                        };
                        let close_hint = if input.french {
                            "Fermer le panneau (Ctrl+Maj+W)"
                        } else {
                            "Close pane (Ctrl+Shift+W)"
                        };
                        let close_resp = paint_header_icon_btn(
                            ui,
                            close_rect,
                            ids.header_close,
                            Icon::PaneClose,
                            close_label,
                            close_hint,
                            input.palette,
                            true,
                        );
                        if close_resp.clicked() {
                            request_close = true;
                        }
                    }
                }

                // 2. Floating Body (`.floating-term-body`) — invoked across all open phases.
                if body_rect.height() > 0.0 && body_rect.width() > 0.0 {
                    let popup_open_now = menu_open || egui::Popup::is_any_open(ctx);
                    let accepts_input = ui.is_enabled() && state.has_keyboard()
                        && !input.dialog_open
                        && !input.typing_in_field
                        && !popup_open_now;
                    let content_rect = if input.has_terminal {
                        body_rect
                    } else {
                        let left = (body_rect.left() + 14.0).min(body_rect.right());
                        let top = (body_rect.top() + 12.0).min(body_rect.bottom());
                        let right = (body_rect.right() - 14.0).max(left);
                        let bottom = (body_rect.bottom() - 12.0).max(top);
                        Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom))
                    };
                    let mut body_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(content_rect)
                            .layout(Layout::top_down(Align::Min)),
                    );
                    body_ui.set_clip_rect(body_rect.intersect(ws));
                    if !input.has_terminal && input.starting {
                        let starting_text = if input.french {
                            "Démarrage du terminal..."
                        } else {
                            "Starting the terminal..."
                        };
                        body_ui.label(
                            egui::RichText::new(starting_text)
                                .size(12.0)
                                .color(input.palette.text_dim),
                        );
                        body_ui.add_space(6.0);
                    }
                    let focus_before_body = body_ui.ctx().memory(|m| m.focused());
                    render_body(
                        &mut body_ui,
                        FloatingBodyContext {
                            rect: content_rect,
                            accepts_input,
                            has_keyboard: state.has_keyboard(),
                            focus_id: ids.content,
                        },
                    );
                    let focus_after_body = body_ui.ctx().memory(|m| m.focused());
                    if focus_after_body != focus_before_body
                        && let Some(bfid) = focus_after_body
                    {
                        state.body_child_focus = Some(bfid);
                    }
                    if !popup_open_now
                        && !input.dialog_open
                        && ctx.input(|i| {
                            i.pointer.button_pressed(PointerButton::Primary)
                                && i.pointer
                                    .interact_pos()
                                    .is_some_and(|pos| body_rect.contains(pos))
                        })
                    {
                        clicked_inside_panel = true;
                    }
                }

                // 3. Bottom Resize Handle (`.floating-term-handle`, `10px`) + `Hide` chevron button.
                if handle_rect.height() > 0.0 {
                    let hide_w = 20.0_f32.min((handle_rect.width() - 12.0).max(0.0));
                    let hide_h = 14.0_f32.min(panel_rect.height());
                    let hide_rect = Rect::from_center_size(
                        Pos2::new(handle_rect.right() - 6.0 - hide_w * 0.5, handle_rect.center().y),
                        Vec2::new(hide_w, hide_h),
                    )
                    .intersect(panel_rect);

                    let resize_label = if input.french {
                        "Redimensionner le terminal flottant"
                    } else {
                        "Resize the floating terminal"
                    };
                    let resize_hint = if input.french {
                        "Faites glisser pour redimensionner. Ctrl+` le masque, son terminal continue de tourner."
                    } else {
                        "Drag to resize. Ctrl+` hides it, its terminal keeps running."
                    };

                    let handle_resp =
                        ui.interact(handle_rect, ids.handle, Sense::click_and_drag());
                    handle_resp.widget_info(|| {
                        WidgetInfo::labeled(WidgetType::Slider, ui.is_enabled(), resize_label)
                    });

                    let press_on_hide = ctx
                        .input(|i| i.pointer.press_origin())
                        .is_some_and(|origin| hide_rect.contains(origin));

                    if !press_on_hide
                        && !input.dialog_open
                        && (handle_resp.drag_started_by(PointerButton::Primary)
                            || handle_resp.dragged_by(PointerButton::Primary)
                            || (handle_resp.is_pointer_button_down_on()
                                && ctx.input(|i| i.pointer.primary_down())))
                    {
                        state.resizing_drag_active = true;
                        clicked_inside_panel = true;
                        if let Some(pos) = ctx
                            .input(|i| i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()))
                        {
                            let raw = f64::from((pos.y - ws.top()) / ws.height());
                            state.drag_height = Some(clamp_drag_preview(raw));
                        }
                    }

                    let handle_hovered = (handle_resp.hovered() && !press_on_hide) || state.is_resizing();
                    if handle_hovered && !input.dialog_open {
                        ctx.set_cursor_icon(CursorIcon::ResizeVertical);
                    }

                    // Keyboard `ArrowUp` / `ArrowDown` `±5%` step when handle holds focus.
                    if handle_resp.has_focus() && !input.dialog_open {
                        ctx.memory_mut(|m| {
                            m.set_focus_lock_filter(
                                ids.handle,
                                egui::EventFilter {
                                    vertical_arrows: true,
                                    ..Default::default()
                                },
                            );
                        });
                        let mut step_delta = 0.0_f64;
                        ctx.input_mut(|inp| {
                            if inp.consume_key(Modifiers::NONE, Key::ArrowDown) {
                                step_delta += FLOATING_KEYBOARD_STEP;
                            }
                            if inp.consume_key(Modifiers::NONE, Key::ArrowUp) {
                                step_delta -= FLOATING_KEYBOARD_STEP;
                            }
                        });
                        if step_delta != 0.0 {
                            // Navigation may already be queued before a newly focused
                            // handle installs its arrow-key filter.
                            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
                            let before = clamp_floating_height(state.height);
                            let after = state.step_height(step_delta);
                            if (after - before).abs() > f64::EPSILON {
                                out.actions.push(FloatingTerminalAction::HeightChanged(after));
                            }
                        }
                    }

                    let hp = ui.painter().with_clip_rect(panel_rect);
                    hp.rect_filled(handle_rect, 0.0, input.palette.surface);
                    hp.line_segment(
                        [
                            Pos2::new(handle_rect.left(), handle_rect.top() + 0.5),
                            Pos2::new(handle_rect.right(), handle_rect.top() + 0.5),
                        ],
                        Stroke::new(1.0_f32, input.palette.border),
                    );
                    if handle_resp.has_focus() {
                        hp.rect_stroke(
                            handle_rect.shrink(1.0),
                            0.0,
                            Stroke::new(2.0_f32, input.palette.accent),
                            StrokeKind::Inside,
                        );
                    }

                    let grip_w = 36.0_f32.min((handle_rect.width() - 56.0).max(12.0));
                    let grip_rect = Rect::from_center_size(
                        handle_rect.center(),
                        Vec2::new(grip_w, 3.0_f32.min(handle_rect.height())),
                    );
                    let grip_color = if handle_hovered {
                        input.palette.accent
                    } else {
                        input.palette.border_strong
                    };
                    hp.rect_filled(grip_rect, 2.0, grip_color);

                    let _ = handle_resp.on_hover_text(resize_hint);

                    // `Hide` chevron-up button (`right: 6px`, `@pointerdown.stop`, `@click="ctl.hide()"`).
                    if hide_rect.width() >= 10.0 {
                        let hide_label = if input.french {
                            "Masquer le terminal flottant"
                        } else {
                            "Hide the floating terminal"
                        };
                        let hide_hint = if input.french {
                            "Masquer (Ctrl+`). Son terminal continue de tourner."
                        } else {
                            "Hide (Ctrl+`). Its terminal keeps running."
                        };
                        let hide_resp = ui.interact(hide_rect, ids.hide, Sense::click());
                        hide_resp.widget_info(|| {
                            WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), hide_label)
                        });
                        if hide_resp.hovered() && ui.is_enabled() {
                            hp.rect_filled(hide_rect, 4.0, input.palette.border);
                        }
                        if hide_resp.has_focus() {
                            hp.rect_stroke(
                                hide_rect,
                                4.0,
                                Stroke::new(1.5_f32, input.palette.accent),
                                StrokeKind::Inside,
                            );
                        }
                        let chevron_color = if hide_resp.hovered() && ui.is_enabled() {
                            input.palette.text_strong
                        } else {
                            input.palette.text_dim
                        };
                        paint_chevron_up(
                            &hp,
                            Rect::from_center_size(hide_rect.center(), Vec2::splat(12.0)),
                            chevron_color,
                        );
                        if hide_resp.clicked() {
                            request_hide = true;
                        }
                        let _ = hide_resp.on_hover_text(hide_hint);
                    }
                }

                // Bottom border (`1px solid var(--border-strong)`).
                ui.painter().with_clip_rect(panel_rect).line_segment(
                    [
                        Pos2::new(panel_rect.left(), panel_rect.bottom() - 0.5),
                        Pos2::new(panel_rect.right(), panel_rect.bottom() - 0.5),
                    ],
                    Stroke::new(1.0_f32, input.palette.border_strong),
                );
            });

        let transient_overlay_open =
            input.dialog_open || menu_open || egui::Popup::is_any_open(ctx);

        if request_copy_output {
            out.actions.push(FloatingTerminalAction::CopyOutput);
        }
        if request_restart {
            state.set_keyboard(true);
            state.pending_focus_terminal = true;
            out.actions.push(FloatingTerminalAction::Restart);
        }
        if request_close {
            state.pre_toggle_focus = Some(ctx.memory(|m| m.focused()));
            state.close();
            out.actions.push(FloatingTerminalAction::Close);
            Self::sync_focus_transitions(ctx, state, &ids, &mut out.actions);
            state.was_transient_overlay_open = transient_overlay_open;
            state.had_terminal = false;
            state.last_observed_focus = ctx.memory(|m| m.focused());
            out.has_keyboard = false;
            return out;
        }
        if request_hide {
            state.pre_toggle_focus = Some(ctx.memory(|m| m.focused()));
            state.hide();
            out.actions.push(FloatingTerminalAction::Hide);
            Self::sync_focus_transitions(ctx, state, &ids, &mut out.actions);
            state.was_transient_overlay_open = transient_overlay_open;
            state.had_terminal = input.has_terminal;
            state.last_observed_focus = ctx.memory(|m| m.focused());
            out.has_keyboard = false;
            return out;
        }

        // During `egui::Area`'s initial invisible sizing pass, widgets inside `Area` cannot yet
        // retain focus; preserve `pending_focus_terminal` for the immediately requested repaint.
        if area_was_sizing_pass {
            out.panel_rect = Some(panel_rect);
            out.body_rect = Some(body_rect);
            out.has_keyboard = state.has_keyboard();
            return out;
        }

        if clicked_inside_panel && !input.dialog_open {
            state.set_keyboard(true);
            if !transient_overlay_open {
                let current_focus = ctx.memory(|m| m.focused());
                let keep_widget_focus = current_focus == Some(ids.handle)
                    || current_focus == Some(ids.hide)
                    || current_focus == Some(ids.header_more)
                    || current_focus == Some(ids.header_close)
                    || (state.body_child_focus.is_some()
                        && current_focus == state.body_child_focus
                        && !input.has_terminal);
                if input.has_terminal && !keep_widget_focus && current_focus != Some(ids.content) {
                    state.pending_focus_terminal = true;
                }
            }
        }

        // Observe focus transfers FIRST (`FloatingTerminal.vue:137-141`) so clicking a live
        // workspace widget (even to dismiss a popup) yields `keyboard = false` before popup-close
        // focus restoration runs.
        let observed = ctx.memory(|m| m.focused());
        if observed != state.last_observed_focus {
            if let Some(fid) = observed {
                let inside_panel = ids.is_inside(fid) || state.body_child_focus == Some(fid);
                if inside_panel {
                    state.set_keyboard(true);
                } else if !transient_overlay_open
                    && !ids.is_nowhere(Some(fid))
                    && ctx.read_response(fid).is_some()
                {
                    state.set_keyboard(false);
                }
            }
            state.last_observed_focus = observed;
        }

        // Apply pending terminal focus (`watch(open)` / `watch(leaf)` / dialog-or-popup-close return).
        if (!state.had_terminal && input.has_terminal && state.is_open() && state.has_keyboard())
            || (state.was_transient_overlay_open
                && !transient_overlay_open
                && state.is_open()
                && state.has_keyboard())
        {
            state.pending_focus_terminal = true;
        }

        if state.pending_focus_terminal
            && state.is_open()
            && state.has_keyboard()
            && input.has_terminal
            && !transient_overlay_open
        {
            ctx.memory_mut(|m| m.request_focus(ids.content));
            state.last_observed_focus = Some(ids.content);
            state.pending_focus_terminal = false;
            if !out.actions.contains(&FloatingTerminalAction::FocusTerminal) {
                out.actions.push(FloatingTerminalAction::FocusTerminal);
            }
        }

        state.was_transient_overlay_open = transient_overlay_open;
        state.had_terminal = input.has_terminal;
        out.panel_rect = Some(panel_rect);
        out.body_rect = Some(body_rect);
        out.has_keyboard = state.has_keyboard();
        out
    }

    fn sync_focus_transitions(
        ctx: &egui::Context,
        state: &mut FloatingTerminalState,
        ids: &PanelIds,
        actions: &mut Vec<FloatingTerminalAction>,
    ) {
        if state.was_open == state.open {
            state.keyboard_before_hide = None;
            return;
        }

        let current_focus = ctx.memory(|m| m.focused());
        let reference_focus = state.pre_toggle_focus.take().unwrap_or(current_focus);

        let child_focus = state.body_child_focus;
        let is_inside_panel = |fid: Id| ids.is_inside(fid) || child_focus == Some(fid);

        if state.open {
            state.keyboard_before_hide = None;
            state.body_child_focus = None;
            // `false -> true`: capture outside focus (ignoring body/nowhere and toolbar toggle).
            // Defer `request_focus(ids.content)` until after `Area::show` finishes a visible pass.
            state.saved_focus =
                reference_focus.filter(|&fid| !ids.is_nowhere(Some(fid)) && !is_inside_panel(fid));
            if let Some(prev) = reference_focus
                && !is_inside_panel(prev)
            {
                ctx.memory_mut(|m| m.surrender_focus(prev));
            }
            state.set_keyboard(true);
            state.pending_focus_terminal = true;
            state.was_open = true;
            state.last_observed_focus = ctx.memory(|m| m.focused());
        } else {
            // `true -> false`: restore previous focus only if keyboard was inside floating or nowhere.
            let had_kb = state.keyboard_before_hide.take().unwrap_or(state.keyboard);
            let was_inside_or_nowhere = had_kb
                || reference_focus.is_some_and(is_inside_panel)
                || ids.is_nowhere(reference_focus)
                || current_focus.is_some_and(is_inside_panel)
                || ids.is_nowhere(current_focus);

            state.set_keyboard(false);
            state.pending_focus_terminal = false;
            state.body_child_focus = None;
            state.drag_height = None;
            state.resizing_drag_active = false;
            state.was_open = false;

            if let Some(fid) = current_focus
                && (is_inside_panel(fid) || ids.is_nowhere(Some(fid)))
            {
                ctx.memory_mut(|m| m.surrender_focus(fid));
            }

            let saved = state.saved_focus.take();
            if was_inside_or_nowhere {
                if let Some(prev) = saved
                    && ctx.read_response(prev).is_some()
                {
                    ctx.memory_mut(|m| m.request_focus(prev));
                    actions.push(FloatingTerminalAction::RestoreFocus(Some(prev)));
                } else {
                    actions.push(FloatingTerminalAction::RestoreFocus(None));
                }
            }
            state.last_observed_focus = ctx.memory(|m| m.focused());
        }
    }
}

#[derive(Clone, Copy)]
struct PanelIds {
    area: Id,
    bg: Id,
    header_bar: Id,
    header_more: Id,
    header_close: Id,
    default_content: Id,
    content: Id,
    handle: Id,
    hide: Id,
    default_toggle: Id,
    custom_toggle: Option<Id>,
}

impl PanelIds {
    fn new(base: Id, content_focus_id: Option<Id>, toggle_button_id: Option<Id>) -> Self {
        let default_content = base.with("content");
        Self {
            area: base.with("area"),
            bg: base.with("bg"),
            header_bar: base.with("header_bar"),
            header_more: base.with("header_more"),
            header_close: base.with("header_close"),
            default_content,
            content: content_focus_id.unwrap_or(default_content),
            handle: base.with("resize_handle"),
            hide: base.with("hide_btn"),
            default_toggle: Id::new(TOOLBAR_TOGGLE_ID_SALT),
            custom_toggle: toggle_button_id,
        }
    }

    fn is_inside(&self, id: Id) -> bool {
        id == self.area
            || id == self.bg
            || id == self.header_bar
            || id == self.header_more
            || id == self.header_close
            || id == self.default_content
            || id == self.content
            || id == self.handle
            || id == self.hide
    }

    fn is_nowhere(&self, id: Option<Id>) -> bool {
        match id {
            None => true,
            Some(fid) => fid == self.default_toggle || self.custom_toggle == Some(fid),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_header_icon_btn(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    icon: Icon,
    label: &str,
    hint: &str,
    palette: UiPalette,
    danger_hover: bool,
) -> Response {
    let resp = ui.interact(rect, id, Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    let hovered = resp.hovered() && ui.is_enabled();
    let painter = ui.painter().with_clip_rect(rect);
    if hovered {
        let fill = if danger_hover {
            palette.danger
        } else {
            palette.border
        };
        painter.rect_filled(rect, 5.0, fill);
    }
    if resp.has_focus() {
        painter.rect_stroke(
            rect.shrink(0.5),
            5.0,
            Stroke::new(1.5_f32, palette.accent),
            StrokeKind::Inside,
        );
    }
    let color = if hovered && danger_hover {
        Color32::WHITE
    } else if hovered {
        palette.text_strong
    } else {
        palette.text_dim
    };
    icons::paint(
        &painter,
        Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
        icon,
        color,
    );
    resp.on_hover_text(hint)
}

fn paint_chevron_up(painter: &Painter, rect: Rect, color: Color32) {
    if !rect.is_positive() || !rect.is_finite() {
        return;
    }
    let side = rect.width().min(rect.height());
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let s = side / 16.0;
    let pt = |x: f32, y: f32| Pos2::new(sq.min.x + x * s, sq.min.y + y * s);
    let p0 = pt(3.5, 10.0);
    let p1 = pt(8.0, 5.5);
    let p2 = pt(12.5, 10.0);
    let stroke = Stroke::new(1.6_f32 * s, color);
    painter.add(egui::Shape::line(vec![p0, p1, p2], stroke));
    let cap_r = (0.8 * s).max(0.4);
    painter.circle_filled(p0, cap_r, color);
    painter.circle_filled(p1, cap_r, color);
    painter.circle_filled(p2, cap_r, color);
}

fn show_floating_more_menu(
    ui: &mut Ui,
    title: &str,
    has_output: bool,
    can_restart: bool,
    french: bool,
) -> Option<PaneMenuAction> {
    let lang = if french {
        PaneMenuLang::Fr
    } else {
        PaneMenuLang::En
    };
    let built = build_pane_actions_menu(
        &PaneActionsMenuInput {
            pane_title: title.to_owned(),
            is_floating: true,
            can_arrange_panes: true,
            ..Default::default()
        },
        lang,
    );

    ui.strong(title);
    ui.separator();

    let mut chosen = None;
    for entry in built.entries {
        let PaneMenuEntry::Item {
            action,
            label,
            enabled,
            danger,
            shortcut,
            tooltip,
        } = entry
        else {
            continue;
        };

        let keep = match action {
            PaneMenuAction::CopyOutput => has_output,
            PaneMenuAction::Restart => can_restart,
            PaneMenuAction::ClosePane => true,
            _ => false,
        };
        if !keep {
            continue;
        }

        if danger {
            ui.separator();
        }
        let text = if danger {
            egui::RichText::new(label).color(ui.visuals().error_fg_color)
        } else {
            egui::RichText::new(label)
        };
        let mut btn = egui::Button::new(text);
        if let Some(sc) = shortcut {
            btn = btn.shortcut_text(sc);
        }
        let resp = ui.add_enabled(enabled, btn);
        let resp = if let Some(tip) = tooltip {
            if enabled {
                resp.on_hover_text(tip)
            } else {
                resp.on_disabled_hover_text(tip)
            }
        } else {
            resp
        };
        if resp.clicked() {
            chosen = Some(action);
            ui.close();
        }
    }
    chosen
}

fn sanitize_single_line(raw: &str, fallback: &str) -> String {
    let mut cleaned = String::new();
    for ch in raw.chars() {
        if ch.is_control() {
            if matches!(ch, '\n' | '\r' | '\t') && !cleaned.ends_with(' ') && !cleaned.is_empty() {
                cleaned.push(' ');
            }
            continue;
        }
        if cleaned.chars().count() >= MAX_TITLE_DISPLAY_CHARS {
            cleaned.push('…');
            break;
        }
        cleaned.push(ch);
    }
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed.to_owned()
    }
}
