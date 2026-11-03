//! Classic palette transcribed from Tessel's renderer style.css.
//! Matching the palette is only one part of UI parity; layout remains tracked separately.
use eframe::egui::{self, Color32, FontId, Stroke, TextStyle};

pub const CHROME: Color32 = Color32::from_rgb(0x10, 0x12, 0x16);
pub const SURFACE: Color32 = Color32::from_rgb(0x18, 0x1b, 0x21);
pub const RAISED: Color32 = Color32::from_rgb(0x1f, 0x23, 0x2b);
pub const HOVER: Color32 = Color32::from_rgb(0x28, 0x2d, 0x37);
pub const TERM: Color32 = Color32::from_rgb(0x15, 0x17, 0x1c);
pub const BORDER: Color32 = Color32::from_rgb(0x25, 0x29, 0x32);
pub const ACCENT: Color32 = Color32::from_rgb(0x6c, 0x9c, 0xff);
pub const TEXT: Color32 = Color32::from_rgb(0xd6, 0xd9, 0xdf);
pub const DIM: Color32 = Color32::from_rgb(0x8a, 0x92, 0xa0);

pub fn apply(ctx: &egui::Context, dark: bool) {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    if dark {
        visuals.panel_fill = SURFACE;
        visuals.window_fill = RAISED;
        visuals.extreme_bg_color = CHROME;
        visuals.faint_bg_color = TERM;
        visuals.override_text_color = Some(TEXT);
        visuals.hyperlink_color = ACCENT;
        visuals.warn_fg_color = Color32::from_rgb(0xf5, 0xa5, 0x24);
        visuals.error_fg_color = Color32::from_rgb(0xe5, 0x48, 0x4d);
        visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
        visuals.selection.bg_fill = HOVER;
        visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
        visuals.widgets.noninteractive.bg_fill = SURFACE;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, DIM);
        visuals.widgets.inactive.bg_fill = RAISED;
        visuals.widgets.inactive.weak_bg_fill = RAISED;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
        for widget in [
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.bg_fill = HOVER;
            widget.weak_bg_fill = HOVER;
            widget.bg_stroke = Stroke::new(1.0_f32, ACCENT);
            widget.fg_stroke = Stroke::new(1.0_f32, TEXT);
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
