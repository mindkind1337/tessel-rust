//! Classic palette transcribed from Tessel's renderer style.css.
//! Matching the palette is only one part of UI parity; layout remains tracked separately.
use eframe::egui::{self, Color32, FontId, Stroke, TextStyle};

use crate::theme_palettes::{TerminalPalette, ThemeId};

pub fn active(ctx: &egui::Context) -> ThemeId {
    ctx.data(|data| data.get_temp::<ThemeId>(egui::Id::new("tessel-theme")))
        .unwrap_or_default()
}
pub fn chrome(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        active(ctx).spec().ui.chrome
    } else {
        ctx.style().visuals.panel_fill
    }
}
pub fn terminal(ctx: &egui::Context) -> &'static TerminalPalette {
    &active(ctx).spec().term
}

pub fn apply(ctx: &egui::Context, dark: bool, theme: Option<ThemeId>) {
    let id = theme.unwrap_or_default();
    let dark = dark || theme.is_some();
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("tessel-theme"), id));
    let palette = &id.spec().ui;
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    if dark {
        visuals.panel_fill = palette.surface;
        visuals.window_fill = palette.surface_2;
        visuals.extreme_bg_color = palette.chrome;
        visuals.faint_bg_color = palette.term;
        visuals.override_text_color = Some(palette.text);
        visuals.hyperlink_color = palette.accent;
        visuals.warn_fg_color = palette.warn;
        visuals.error_fg_color = palette.danger;
        visuals.window_stroke = Stroke::new(1.0_f32, palette.border);
        visuals.selection.bg_fill = palette.surface_3;
        visuals.selection.stroke = Stroke::new(1.0_f32, palette.accent);
        visuals.widgets.noninteractive.bg_fill = palette.surface;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, palette.border);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, palette.text_dim);
        visuals.widgets.inactive.bg_fill = palette.surface_2;
        visuals.widgets.inactive.weak_bg_fill = palette.surface_2;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, palette.border);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, palette.text);
        for widget in [
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.bg_fill = palette.surface_3;
            widget.weak_bg_fill = palette.surface_3;
            widget.bg_stroke = Stroke::new(1.0_f32, palette.accent);
            widget.fg_stroke = Stroke::new(1.0_f32, palette.text);
        }
    }
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        for kind in [TextStyle::Body, TextStyle::Button] {
            style.text_styles.insert(kind, FontId::proportional(13.0));
        }
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(11.0));
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(16.0));
    });
}
