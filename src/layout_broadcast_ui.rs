//! Pure `egui` UI, layout preset planning, and multi-write reply filtering for the toolbar
//! `Layout` menu and `Broadcast` multi-write mode.
//!
//! Transcribed directly from:
//! - `src/renderer/src/App.vue` (`2891-2963`, `4269-4331`, `14919-15012`, `15079-15081`)
//! - `src/renderer/src/components/TerminalPane.vue` (`4506-4513`)
//! - `src/renderer/src/components/StatusBar.vue` (`broadcast` status target)
//! - `src/renderer/src/style.css` (`335-421`, `531-537`, `1078-1095`, `5256-5333`, `5521-5534`)
//! - `src/renderer/src/i18n/locales/fr/{app,panes}.json`

use crate::icons::{self, Icon};
use crate::pane_grid::GridPreset;
use crate::theme_palettes::UiPalette;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Frame, Id, Margin, Painter, Pos2, Rect, Response,
    Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo, WidgetType,
};
use std::f32::consts::FRAC_PI_4;
use std::hash::Hash;

/// Outer width of `.layout-menu` (`style.css:5256`: `width: 290px`).
pub const LAYOUT_MENU_WIDTH: f32 = 290.0;
/// Inner padding of `.toolbar-menu` (`style.css:351`: `padding: 5px`).
pub const LAYOUT_MENU_PADDING: i8 = 5;
/// Row height of `.toolbar-menu-item` (`style.css:378`: `height: 30px`).
pub const LAYOUT_MENU_ITEM_HEIGHT: f32 = 30.0;
/// Height of `.grid-chip` (`style.css:5320`: `height: 26px`).
pub const GRID_CHIP_HEIGHT: f32 = 26.0;
/// Exact height of `.broadcast-banner` (`style.css:415`: `height: 24px`).
pub const BROADCAST_BANNER_HEIGHT: f32 = 24.0;

/// `.tb-icon.on.warn` text/icon colour (`style.css:5533`: `#ffe0a8`).
pub const BROADCAST_TOOLBAR_ON_TEXT: Color32 = Color32::from_rgb(0xff, 0xe0, 0xa8);
/// `.broadcast-banner` and `.bc-toggle.member` text colour (`style.css:418, 1094`: `#ffd98f`).
pub const BROADCAST_MEMBER_TEXT: Color32 = Color32::from_rgb(0xff, 0xd9, 0x8f);
/// `.toolbar-menu-item.danger:hover` text colour (`style.css:5271`: `#ffb4b6`).
pub const DANGER_HOVER_TEXT: Color32 = Color32::from_rgb(0xff, 0xb4, 0xb6);

#[must_use]
pub fn with_alpha(color: Color32, alpha: f32) -> Color32 {
    let [r, g, b, _] = color.to_srgba_unmultiplied();
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

#[must_use]
pub fn srgb_mix(base: Color32, tint: Color32, tint_weight: f32) -> Color32 {
    let w = tint_weight.clamp(0.0, 1.0);
    let inv = 1.0 - w;
    let [br, bg, bb, _] = base.to_srgba_unmultiplied();
    let [tr, tg, tb, _] = tint.to_srgba_unmultiplied();
    Color32::from_rgb(
        (f32::from(br) * inv + f32::from(tr) * w).round() as u8,
        (f32::from(bg) * inv + f32::from(tg) * w).round() as u8,
        (f32::from(bb) * inv + f32::from(tb) * w).round() as u8,
    )
}

/// `(cols, rows)` dimensions for each `Even grid` preset (`App.vue` `gridOptions`).
#[must_use]
pub const fn preset_dims(preset: GridPreset) -> (usize, usize) {
    match preset {
        GridPreset::OneByTwo => (1, 2),
        GridPreset::TwoByOne => (2, 1),
        GridPreset::TwoByTwo => (2, 2),
        GridPreset::ThreeByTwo => (3, 2),
        GridPreset::ThreeByThree => (3, 3),
    }
}

/// Spaced display label (`"1 x 2"` .. `"3 x 3"`) matching `App.vue` `gridOptions`.
#[must_use]
pub const fn preset_display_label(preset: GridPreset) -> &'static str {
    match preset {
        GridPreset::OneByTwo => "1 x 2",
        GridPreset::TwoByOne => "2 x 1",
        GridPreset::TwoByTwo => "2 x 2",
        GridPreset::ThreeByTwo => "3 x 2",
        GridPreset::ThreeByThree => "3 x 3",
    }
}

/// Pure plan for `buildGrid(cols, rows)` (`App.vue:2891-2941`):
/// keeps all open workspace panes in leaf order, adds extra rows if more panes are already open
/// than `cols * rows`, and spawns new default shells only for empty slots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetGridPlan {
    pub cols: usize,
    pub effective_rows: usize,
    pub kept_ids: Vec<u64>,
    pub shells_to_spawn: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalizedPresetGrid {
    pub ordered_ids: Vec<u64>,
    pub row_counts: Vec<usize>,
    pub active_id: Option<u64>,
}

#[must_use]
pub fn plan_preset_grid(
    preset: GridPreset,
    existing_pane_ids: &[u64],
    available_new_slots: usize,
) -> PresetGridPlan {
    let (cols, min_rows) = preset_dims(preset);
    let needed_rows_for_existing = existing_pane_ids.len().div_ceil(cols);
    let effective_rows = min_rows.max(needed_rows_for_existing);
    let target_slots = cols.saturating_mul(effective_rows);
    let shells_to_spawn = target_slots
        .saturating_sub(existing_pane_ids.len())
        .min(available_new_slots);
    PresetGridPlan {
        cols,
        effective_rows,
        kept_ids: existing_pane_ids.to_vec(),
        shells_to_spawn,
    }
}

impl PresetGridPlan {
    #[must_use]
    pub fn finalize(
        &self,
        spawned_ids: &[u64],
        current_active_id: Option<u64>,
    ) -> FinalizedPresetGrid {
        let mut ordered_ids = Vec::with_capacity(self.kept_ids.len() + spawned_ids.len());
        ordered_ids.extend_from_slice(&self.kept_ids);
        ordered_ids.extend_from_slice(spawned_ids);
        let row_counts: Vec<usize> = ordered_ids
            .chunks(self.cols.max(1))
            .map(<[u64]>::len)
            .collect();
        let active_id = match current_active_id {
            Some(id) if ordered_ids.contains(&id) => Some(id),
            _ => ordered_ids.first().copied(),
        };
        FinalizedPresetGrid {
            ordered_ids,
            row_counts,
            active_id,
        }
    }
}

#[must_use]
pub const fn layout_toolbar_label(french: bool) -> &'static str {
    if french { "Disposition" } else { "Layout" }
}

#[must_use]
pub const fn layout_toolbar_tooltip(french: bool) -> &'static str {
    if french {
        "Disposition : scinder et organiser les panneaux"
    } else {
        "Layout: split and arrange panes"
    }
}

#[must_use]
pub const fn split_right_label(french: bool) -> &'static str {
    if french {
        "Scinder vers la droite"
    } else {
        "Split right"
    }
}

#[must_use]
pub const fn split_down_label(french: bool) -> &'static str {
    if french {
        "Scinder vers le bas"
    } else {
        "Split down"
    }
}

#[must_use]
pub const fn arrange_grid_label(french: bool) -> &'static str {
    if french {
        "Organiser les panneaux en grille"
    } else {
        "Arrange panes in a grid"
    }
}

#[must_use]
pub const fn arrange_grid_hint(french: bool) -> &'static str {
    if french {
        "Panneaux de taille égale, aussi carrés que la place le permet ; les nouveaux panneaux s'y rangent aussi"
    } else {
        "Equal panes, as square as the room allows; new panes then tile too"
    }
}

#[must_use]
pub const fn even_grid_heading(french: bool) -> &'static str {
    if french {
        "Grille régulière"
    } else {
        "Even grid"
    }
}

#[must_use]
pub fn arrange_preset_tooltip(french: bool, preset: GridPreset) -> String {
    let label = preset_display_label(preset);
    if french {
        format!("Organiser cet espace de travail en {label}")
    } else {
        format!("Arrange this workspace into {label}")
    }
}

#[must_use]
pub fn even_grid_command_title(french: bool, preset: GridPreset) -> String {
    let label = preset_display_label(preset);
    if french {
        format!("Grille régulière {label}")
    } else {
        format!("Even grid {label}")
    }
}

#[must_use]
pub const fn close_active_menu_label(french: bool) -> &'static str {
    if french {
        "Fermer le panneau actif"
    } else {
        "Close active pane"
    }
}

#[must_use]
pub const fn close_active_command_title(french: bool) -> &'static str {
    if french {
        "Fermer le panneau actif"
    } else {
        "Close the active pane"
    }
}

#[must_use]
pub const fn broadcast_toolbar_label(french: bool) -> &'static str {
    if french { "Diffusion" } else { "Broadcast" }
}

#[must_use]
pub const fn broadcast_toolbar_tooltip(french: bool, broadcast_on: bool) -> &'static str {
    match (french, broadcast_on) {
        (false, true) => {
            "Broadcast is on: type once into every pane with write checked (Ctrl+Shift+B)"
        }
        (false, false) => {
            "Broadcast is off: type once into every pane with write checked (Ctrl+Shift+B)"
        }
        (true, true) => {
            "La diffusion est activée : tapez une fois dans chaque panneau où « écrire » est coché (Ctrl+Shift+B)"
        }
        (true, false) => {
            "La diffusion est désactivée : tapez une fois dans chaque panneau où « écrire » est coché (Ctrl+Shift+B)"
        }
    }
}

#[must_use]
pub const fn broadcast_banner_text(french: bool) -> &'static str {
    if french {
        "La diffusion est activée. Les frappes vont à chaque panneau où « écrire » est coché."
    } else {
        "Broadcast is on. Keystrokes go to every pane with \"write\" checked."
    }
}

#[must_use]
pub const fn broadcast_command_title(french: bool, broadcast_on: bool) -> &'static str {
    match (french, broadcast_on) {
        (false, true) => "Turn broadcast off",
        (false, false) => "Turn broadcast on",
        (true, true) => "Désactiver la diffusion",
        (true, false) => "Activer la diffusion",
    }
}

#[must_use]
pub const fn pane_broadcast_label(french: bool) -> &'static str {
    if french { "écrire" } else { "write" }
}

#[must_use]
pub const fn pane_broadcast_tooltip(french: bool) -> &'static str {
    if french {
        "Inclure ce panneau dans l'écriture multiple"
    } else {
        "Include this pane in multi-write"
    }
}

/// Formats the status-bar active input target when broadcast mode is active (`app.status.broadcast*`).
#[must_use]
pub fn format_broadcast_status_target(french: bool, pane_count: usize) -> String {
    match (french, pane_count) {
        (false, 1) => "Broadcast → 1 pane".to_owned(),
        (false, n) => format!("Broadcast → {n} panes"),
        (true, 1) => "Diffusion → 1 panneau".to_owned(),
        (true, n) => format!("Diffusion → {n} panneaux"),
    }
}

/// Exact byte-level match of `App.vue:2946`:
/// `const TERMINAL_REPLY = /^\x1b\[[?>]?[\d;]*[cRn]$|^\x1b\[[IO]$|^\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)$/`
///
/// Terminal device-attribute, cursor-position, status, focus-in/out, and OSC query replies belong
/// solely to the terminal that answered them and must never be fanned out to peer panes.
#[must_use]
pub fn is_terminal_reply(bytes: &[u8]) -> bool {
    if bytes.len() < 3 || bytes[0] != 0x1b {
        return false;
    }
    match bytes[1] {
        b'[' => {
            if bytes.len() == 3 && matches!(bytes[2], b'I' | b'O') {
                return true;
            }
            let mut idx = 2;
            if idx < bytes.len() && matches!(bytes[idx], b'?' | b'>') {
                idx += 1;
            }
            while idx + 1 < bytes.len() && (bytes[idx].is_ascii_digit() || bytes[idx] == b';') {
                idx += 1;
            }
            idx + 1 == bytes.len() && matches!(bytes[idx], b'c' | b'R' | b'n')
        }
        b']' => {
            if let Some(body) = bytes[2..].strip_suffix(&[0x07]) {
                !body.iter().any(|&b| b == 0x07 || b == 0x1b)
            } else if let Some(body) = bytes[2..].strip_suffix(b"\x1b\\") {
                !body.iter().any(|&b| b == 0x07 || b == 0x1b)
            } else {
                false
            }
        }
        _ => false,
    }
}

/// `.toolbar-menu.layout-menu` popup frame (`style.css:351-355, 5256`).
pub fn layout_menu_frame(palette: &UiPalette) -> Frame {
    Frame::NONE
        .inner_margin(Margin::same(LAYOUT_MENU_PADDING))
        .corner_radius(CornerRadius::same(9))
        .fill(palette.surface_2)
        .stroke(Stroke::new(1.0_f32, palette.border_strong))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutMenuInput {
    pub french: bool,
    pub has_active_pane: bool,
    pub pane_count: usize,
    pub can_apply_grid: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutMenuAction {
    SplitRight,
    SplitDown,
    ArrangeGrid,
    ApplyPreset(GridPreset),
    CloseActive,
}

/// Toolbar `Layout` icon button (`30x28`, `.tb-icon`, `.open` highlight when menu is open).
pub fn paint_layout_toolbar_button(
    ui: &mut Ui,
    palette: &UiPalette,
    radius_px: f32,
    french: bool,
    menu_open: bool,
) -> Response {
    let label = layout_toolbar_label(french);
    let tooltip = layout_toolbar_tooltip(french);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(30.0, 28.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));

    let fill = if menu_open || (ui.is_enabled() && response.hovered()) {
        palette.surface_3
    } else {
        Color32::TRANSPARENT
    };
    let fg = if !ui.is_enabled() {
        palette.text_dim.gamma_multiply(0.5)
    } else if menu_open || response.hovered() {
        palette.text_strong
    } else {
        palette.text_dim
    };

    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, radius_px, fill);
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(16.0)),
        Icon::Layout,
        fg,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            radius_px,
            ui.visuals().selection.stroke,
            StrokeKind::Inside,
        );
    }
    response.on_hover_text(tooltip)
}

fn menu_separator(ui: &mut Ui, palette: &UiPalette) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 11.0), Sense::hover());
    let y = rect.center().y;
    ui.painter().line_segment(
        [
            Pos2::new(rect.left() + 4.0, y),
            Pos2::new((rect.right() - 4.0).max(rect.left() + 4.0), y),
        ],
        Stroke::new(1.0_f32, palette.border),
    );
}

struct MenuRow<'a> {
    id_salt: &'a str,
    label: &'a str,
    shortcut: Option<&'a str>,
    enabled: bool,
    danger: bool,
    tooltip: Option<&'a str>,
}

fn menu_row_item(ui: &mut Ui, palette: &UiPalette, row: MenuRow<'_>) -> bool {
    let MenuRow {
        id_salt,
        label,
        shortcut,
        enabled,
        danger,
        tooltip,
    } = row;
    let width = ui.available_width();
    let id = ui.make_persistent_id(id_salt);
    let (_, rect) = ui.allocate_space(Vec2::new(width, LAYOUT_MENU_ITEM_HEIGHT));
    let mut response = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));

    let active_hover = enabled && (response.hovered() || response.has_focus());
    let fill = if active_hover {
        if danger {
            with_alpha(palette.danger, 0.18)
        } else {
            palette.surface_3
        }
    } else {
        Color32::TRANSPARENT
    };
    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 5.0, fill);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            5.0,
            ui.visuals().selection.stroke,
            StrokeKind::Inside,
        );
    }

    let text_color = if !enabled {
        palette.text_dim.gamma_multiply(0.5)
    } else if active_hover && danger {
        DANGER_HOVER_TEXT
    } else if active_hover {
        palette.text_strong
    } else {
        palette.text
    };
    let shortcut_color = if !enabled {
        palette.text_dim.gamma_multiply(0.45)
    } else if active_hover && danger {
        DANGER_HOVER_TEXT
    } else {
        palette.text_dim
    };

    let painter = ui.painter().with_clip_rect(rect);
    let mut right_edge = rect.right() - 8.0;
    if let Some(kbd) = shortcut {
        let galley =
            painter.layout_no_wrap(kbd.to_owned(), FontId::proportional(11.0), shortcut_color);
        let pos = Pos2::new(
            right_edge - galley.size().x,
            rect.center().y - galley.size().y * 0.5,
        );
        right_edge = (pos.x - 9.0).max(rect.left() + 8.0);
        painter.galley(pos, galley, shortcut_color);
    }

    let name_width = (right_edge - (rect.left() + 8.0)).max(0.0);
    let mut job = egui::text::LayoutJob::simple(
        label.to_owned(),
        FontId::proportional(12.0),
        text_color,
        name_width,
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = painter.layout_job(job);
    painter.galley(
        Pos2::new(rect.left() + 8.0, rect.center().y - galley.size().y * 0.5),
        galley,
        text_color,
    );

    if let Some(tip) = tooltip {
        response = response.on_hover_text(tip);
    }
    enabled && response.clicked()
}

/// Renders the exact `290px` `.layout-menu` contents (`App.vue:14960-15011`).
pub fn show_layout_menu_contents(
    ui: &mut Ui,
    palette: &UiPalette,
    input: LayoutMenuInput,
) -> Option<LayoutMenuAction> {
    // Outer `.layout-menu` is 290px (`5px` inner padding + `1px` border on each side = `12px`).
    let inner_width = LAYOUT_MENU_WIDTH - f32::from(LAYOUT_MENU_PADDING) * 2.0 - 2.0;
    ui.set_min_width(inner_width);
    ui.set_max_width(inner_width);
    ui.spacing_mut().item_spacing = Vec2::ZERO;

    let mut action = None;

    if menu_row_item(
        ui,
        palette,
        MenuRow {
            id_salt: "layout_split_right",
            label: split_right_label(input.french),
            shortcut: Some("Ctrl+Shift+E"),
            enabled: input.has_active_pane,
            danger: false,
            tooltip: None,
        },
    ) {
        action = Some(LayoutMenuAction::SplitRight);
    }

    if menu_row_item(
        ui,
        palette,
        MenuRow {
            id_salt: "layout_split_down",
            label: split_down_label(input.french),
            shortcut: Some("Ctrl+Shift+O"),
            enabled: input.has_active_pane,
            danger: false,
            tooltip: None,
        },
    ) {
        action = Some(LayoutMenuAction::SplitDown);
    }

    if input.pane_count > 1
        && menu_row_item(
            ui,
            palette,
            MenuRow {
                id_salt: "layout_arrange_grid",
                label: arrange_grid_label(input.french),
                shortcut: None,
                enabled: true,
                danger: false,
                tooltip: Some(arrange_grid_hint(input.french)),
            },
        )
    {
        action = Some(LayoutMenuAction::ArrangeGrid);
    }

    menu_separator(ui, palette);

    // `.menu-label`: padding 5px 8px 6px, color var(--text-dim), font-size 11px.
    let (label_rect, _) = ui.allocate_exact_size(Vec2::new(inner_width, 24.0), Sense::hover());
    ui.painter().text(
        Pos2::new(label_rect.left() + 8.0, label_rect.center().y - 0.5),
        Align2::LEFT_CENTER,
        even_grid_heading(input.french),
        FontId::proportional(11.0),
        palette.text_dim,
    );

    // `.grid-chips`: padding 0 8px 6px, gap 5px, wrapped `.grid-chip` buttons (`height: 26px`).
    Frame::NONE
        .inner_margin(Margin {
            left: 8,
            right: 8,
            top: 0,
            bottom: 6,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(5.0);
            ui.horizontal_wrapped(|ui| {
                for preset in GridPreset::ALL {
                    let chip_label = preset_display_label(preset);
                    let galley = ui.painter().layout_no_wrap(
                        chip_label.to_owned(),
                        FontId::proportional(12.0),
                        palette.text,
                    );
                    let chip_width = (galley.size().x + 18.0).ceil();
                    let (chip_rect, chip_resp) = ui.allocate_exact_size(
                        Vec2::new(chip_width, GRID_CHIP_HEIGHT),
                        if input.can_apply_grid {
                            Sense::click()
                        } else {
                            Sense::hover()
                        },
                    );
                    chip_resp.widget_info(|| {
                        WidgetInfo::labeled(WidgetType::Button, input.can_apply_grid, chip_label)
                    });
                    let hovered =
                        input.can_apply_grid && (chip_resp.hovered() || chip_resp.has_focus());
                    let chip_fill = if hovered {
                        palette.surface_3
                    } else {
                        Color32::TRANSPARENT
                    };
                    let chip_fg = if !input.can_apply_grid {
                        palette.text_dim.gamma_multiply(0.5)
                    } else if hovered {
                        palette.text_strong
                    } else {
                        palette.text
                    };
                    ui.painter().rect(
                        chip_rect,
                        6.0,
                        chip_fill,
                        Stroke::new(1.0_f32, palette.border_strong),
                        StrokeKind::Inside,
                    );
                    if chip_resp.has_focus() {
                        ui.painter().rect_stroke(
                            chip_rect.shrink(0.5),
                            6.0,
                            ui.visuals().selection.stroke,
                            StrokeKind::Inside,
                        );
                    }
                    ui.painter().text(
                        chip_rect.center(),
                        Align2::CENTER_CENTER,
                        chip_label,
                        FontId::proportional(12.0),
                        chip_fg,
                    );
                    let tip = arrange_preset_tooltip(input.french, preset);
                    if chip_resp.on_hover_text(tip).clicked() && input.can_apply_grid {
                        action = Some(LayoutMenuAction::ApplyPreset(preset));
                    }
                }
            });
        });

    menu_separator(ui, palette);

    if menu_row_item(
        ui,
        palette,
        MenuRow {
            id_salt: "layout_close_active",
            label: close_active_menu_label(input.french),
            shortcut: Some("Ctrl+Shift+W"),
            enabled: input.has_active_pane,
            danger: true,
            tooltip: None,
        },
    ) {
        action = Some(LayoutMenuAction::CloseActive);
    }

    action
}

fn paint_arc(
    painter: &Painter,
    center: Pos2,
    radius: f32,
    start_angle: f32,
    end_angle: f32,
    stroke: Stroke,
) {
    const SEGMENTS: usize = 10;
    let mut points = Vec::with_capacity(SEGMENTS + 1);
    for step in 0..=SEGMENTS {
        let t = step as f32 / SEGMENTS as f32;
        let angle = start_angle + (end_angle - start_angle) * t;
        points.push(Pos2::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        ));
    }
    painter.add(egui::Shape::line(points, stroke));
}

/// Paints the `16x16` broadcast radio-wave SVG (`App.vue:14927-14935`):
/// `<circle cx="8" cy="8" r="1.5" />` + inner `r=3.8` and outer `r=6.6` `±45°` arcs.
pub fn paint_broadcast_icon(painter: &Painter, rect: Rect, color: Color32) {
    let scale = (rect.width().min(rect.height()) / 16.0).max(0.1);
    let center = rect.center();
    painter.circle_filled(center, 1.5 * scale, color);
    let stroke = Stroke::new(1.3 * scale, color);
    let left_start = std::f32::consts::PI - FRAC_PI_4;
    let left_end = std::f32::consts::PI + FRAC_PI_4;
    let right_start = -FRAC_PI_4;
    let right_end = FRAC_PI_4;
    for radius in [3.8 * scale, 6.6 * scale] {
        paint_arc(painter, center, radius, left_start, left_end, stroke);
        paint_arc(painter, center, radius, right_start, right_end, stroke);
    }
}

/// Toolbar `Broadcast` icon button (`30x28`, `.tb-icon`, `.on.warn` highlight + `aria-pressed`).
pub fn paint_broadcast_toolbar_button(
    ui: &mut Ui,
    palette: &UiPalette,
    radius_px: f32,
    french: bool,
    broadcast_on: bool,
) -> Response {
    let label = broadcast_toolbar_label(french);
    let tooltip = broadcast_toolbar_tooltip(french, broadcast_on);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(30.0, 28.0), Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), broadcast_on, label)
    });

    let fill = if broadcast_on {
        with_alpha(palette.warn, 0.20)
    } else if ui.is_enabled() && response.hovered() {
        palette.surface_3
    } else {
        Color32::TRANSPARENT
    };
    let fg = if !ui.is_enabled() {
        palette.text_dim.gamma_multiply(0.5)
    } else if broadcast_on {
        BROADCAST_TOOLBAR_ON_TEXT
    } else if response.hovered() {
        palette.text_strong
    } else {
        palette.text_dim
    };

    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, radius_px, fill);
    }
    if broadcast_on {
        ui.painter().rect_stroke(
            rect,
            radius_px,
            Stroke::new(1.0_f32, with_alpha(palette.warn, 0.60)),
            StrokeKind::Inside,
        );
    }
    paint_broadcast_icon(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(16.0)),
        fg,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            radius_px,
            ui.visuals().selection.stroke,
            StrokeKind::Inside,
        );
    }
    response.on_hover_text(tooltip)
}

/// Full-width `24px` `.broadcast-banner` shown directly below the toolbar when broadcast is on.
pub fn show_broadcast_banner(ui: &mut Ui, palette: &UiPalette, french: bool) -> Response {
    let text = broadcast_banner_text(french);
    let width = ui.available_width().max(1.0);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, BROADCAST_BANNER_HEIGHT), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));

    let fill = srgb_mix(palette.chrome, palette.warn, 0.16);
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0.0, fill);
    painter.line_segment(
        [
            Pos2::new(rect.left(), rect.bottom() - 0.5),
            Pos2::new(rect.right(), rect.bottom() - 0.5),
        ],
        Stroke::new(1.0_f32, with_alpha(palette.warn, 0.40)),
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(11.5),
        BROADCAST_MEMBER_TEXT,
    );
    response
}

/// Width reserved in a terminal pane header for the `.bc-toggle` (`write` / `écrire`) control
/// when broadcast mode is active.
#[must_use]
pub const fn pane_broadcast_toggle_width(french: bool) -> f32 {
    if french { 56.0 } else { 50.0 }
}

/// Per-pane `.bc-toggle` checkbox (`write` / `écrire`) in terminal pane headers when broadcast is on.
pub fn show_pane_broadcast_toggle(
    ui: &mut Ui,
    palette: &UiPalette,
    id_source: impl Hash,
    french: bool,
    checked: &mut bool,
) -> Response {
    let label = pane_broadcast_label(french);
    let tooltip = pane_broadcast_tooltip(french);
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_owned(), FontId::proportional(11.0), palette.text);
    let width = (12.0 + 4.0 + galley.size().x + 4.0).ceil();
    let id = Id::new(id_source);
    let (_, rect) = ui.allocate_space(Vec2::new(width, 20.0));
    let mut response = ui.interact(rect, id, Sense::click());
    if ui.is_enabled() && response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *checked, label)
    });

    let box_rect = Rect::from_min_size(
        Pos2::new(rect.left(), rect.center().y - 6.0),
        Vec2::splat(12.0),
    );
    let painter = ui.painter().with_clip_rect(rect);
    if *checked {
        painter.rect_filled(box_rect, 3.0, palette.warn);
        let c = box_rect.center();
        let check_color = Color32::from_rgb(0x1c, 0x15, 0x00);
        painter.add(egui::Shape::line(
            vec![
                Pos2::new(c.x - 3.2, c.y + 0.1),
                Pos2::new(c.x - 0.8, c.y + 2.5),
                Pos2::new(c.x + 3.4, c.y - 2.3),
            ],
            Stroke::new(1.5_f32, check_color),
        ));
    } else {
        let fill = if response.hovered() {
            palette.surface_3
        } else {
            palette.surface
        };
        painter.rect(
            box_rect,
            3.0,
            fill,
            Stroke::new(1.0_f32, palette.border_strong),
            StrokeKind::Inside,
        );
    }

    if response.has_focus() {
        painter.rect_stroke(
            rect.shrink(0.5),
            3.0,
            ui.visuals().selection.stroke,
            StrokeKind::Inside,
        );
    }

    let text_color = if *checked {
        BROADCAST_MEMBER_TEXT
    } else if response.hovered() {
        palette.text
    } else {
        palette.text_dim
    };
    painter.text(
        Pos2::new(box_rect.right() + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(11.0),
        text_color,
    );

    response.on_hover_text(tooltip)
}

/// Active vs inactive pane border stroke colour (`style.css:531-537`:
/// `.pane.broadcast-member.active { border-color: var(--warn); }`).
#[must_use]
pub const fn pane_active_stroke_color(
    palette: &UiPalette,
    is_active: bool,
    broadcast_global: bool,
    pane_broadcast: bool,
    is_terminal_pane: bool,
    default_active_color: Color32,
    inactive_color: Color32,
) -> Color32 {
    if !is_active {
        inactive_color
    } else if broadcast_global && pane_broadcast && is_terminal_pane {
        palette.warn
    } else {
        default_active_color
    }
}
