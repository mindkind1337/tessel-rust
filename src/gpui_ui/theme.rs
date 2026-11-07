//! 1:1 GPUI CSS design tokens, sRGB `color-mix`, layout metrics, and DirectWrite font stacks
//! transcribed from `C:\Tessel\src\renderer\src\style.css`, `themes.css`, and `themePalettes.js`.

use crate::theme_palettes::{ThemeId, WARP_OK_GREEN};
use eframe::egui::Color32;
use gpui::{Rgba, SharedString};

pub const TOOLBAR_HEIGHT_PX: f32 = 40.0;
pub const TOOLBAR_BTN_HEIGHT_PX: f32 = 28.0;
pub const TOOLBAR_ICON_BTN_WIDTH_PX: f32 = 30.0;
pub const CAPTION_BTN_WIDTH_PX: f32 = 46.0;
pub const BROADCAST_BANNER_HEIGHT_PX: f32 = 24.0;
pub const VIEW_TABS_HEIGHT_PX: f32 = 30.0;
pub const PANE_NAV_HEIGHT_PX: f32 = 30.0;
pub const STATUSBAR_HEIGHT_PX: f32 = 24.0;

pub const SIDEBAR_RAIL_WIDTH_PX: f32 = 52.0;
pub const SIDEBAR_DEFAULT_WIDTH_PX: f32 = 248.0;
pub const SIDEBAR_MIN_WIDTH_PX: f32 = 180.0;
pub const SIDEBAR_MAX_WIDTH_PX: f32 = 420.0;

pub const RIGHT_PANEL_DEFAULT_WIDTH_PX: f32 = 340.0;
pub const RIGHT_PANEL_MIN_WIDTH_PX: f32 = 280.0;
pub const RIGHT_PANEL_MAX_WIDTH_PX: f32 = 720.0;

pub const SPLIT_DIVIDER_PX: f32 = 4.0;

pub const PANE_CHIP_HIDE_MODEL_BELOW_PX: f64 = 340.0;
pub const PANE_CHIP_HIDE_ACCOUNT_BELOW_PX: f64 = 460.0;

#[must_use]
pub fn ui_font_family() -> SharedString {
    if cfg!(windows) {
        SharedString::new_static("Segoe UI Variable Text")
    } else if cfg!(target_os = "macos") {
        SharedString::new_static(".SystemUIFont")
    } else {
        SharedString::new_static("sans-serif")
    }
}

#[must_use]
pub fn mono_font_family() -> SharedString {
    if cfg!(windows) {
        SharedString::new_static("Cascadia Mono")
    } else if cfg!(target_os = "macos") {
        SharedString::new_static("SF Mono")
    } else {
        SharedString::new_static("monospace")
    }
}

#[must_use]
pub fn color32_to_rgba(c: Color32) -> Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Rgba {
        color: gpui::Rgba::new(
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            1.0,
        )
        .color,
        alpha: f32::from(a) / 255.0,
    }
}

#[must_use]
pub fn rgba_with_alpha(mut c: Rgba, alpha: f32) -> Rgba {
    c.alpha = alpha.clamp(0.0, 1.0);
    c
}

/// Exact CSS `color-mix(in srgb, a pct_a%, b)` interpolation in `[0.0, 1.0]` sRGB space.
#[must_use]
pub fn color_mix_srgb(a: Rgba, weight_a: f32, b: Rgba) -> Rgba {
    let w = weight_a.clamp(0.0, 1.0);
    let inv = 1.0 - w;
    Rgba::new(
        a.color.red * w + b.color.red * inv,
        a.color.green * w + b.color.green * inv,
        a.color.blue * w + b.color.blue * inv,
        a.alpha * w + b.alpha * inv,
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuiThemeTokens {
    pub id: ThemeId,
    pub radius_px: f32,
    pub chrome: Rgba,
    pub surface: Rgba,
    pub surface_2: Rgba,
    pub surface_3: Rgba,
    pub term: Rgba,
    pub backdrop: Rgba,
    pub border: Rgba,
    pub border_strong: Rgba,
    pub accent: Rgba,
    pub accent_soft_bg: Rgba,
    pub warn: Rgba,
    pub danger: Rgba,
    pub ok_green: Rgba,
    pub text: Rgba,
    pub text_strong: Rgba,
    pub text_dim: Rgba,
    pub titlebar_bg: Rgba,
    pub titlebar_symbol: Rgba,
    pub active_pane_ring: Rgba,
    pub broadcast_pane_ring: Rgba,
    pub broadcast_banner_bg: Rgba,
    pub broadcast_banner_fg: Rgba,
    pub caption_close_hover_bg: Rgba,
}

impl GpuiThemeTokens {
    #[must_use]
    pub fn from_theme(theme: ThemeId) -> Self {
        let spec = theme.spec();
        let chrome = color32_to_rgba(spec.ui.chrome);
        let surface = color32_to_rgba(spec.ui.surface);
        let surface_2 = color32_to_rgba(spec.ui.surface_2);
        let surface_3 = color32_to_rgba(spec.ui.surface_3);
        let term = color32_to_rgba(spec.ui.term);
        let backdrop = color32_to_rgba(spec.ui.backdrop);
        let border = color32_to_rgba(spec.ui.border);
        let border_strong = color32_to_rgba(spec.ui.border_strong);
        let accent = color32_to_rgba(spec.ui.accent);
        let warn = color32_to_rgba(spec.ui.warn);
        let danger = color32_to_rgba(spec.ui.danger);
        let ok_green = color32_to_rgba(WARP_OK_GREEN);
        let text = color32_to_rgba(spec.ui.text);
        let text_strong = color32_to_rgba(spec.ui.text_strong);
        let text_dim = color32_to_rgba(spec.ui.text_dim);
        let titlebar_bg = color32_to_rgba(spec.title_bar.color);
        let titlebar_symbol = color32_to_rgba(spec.title_bar.symbol_color);

        Self {
            id: theme,
            radius_px: spec.radius_px,
            chrome,
            surface,
            surface_2,
            surface_3,
            term,
            backdrop,
            border,
            border_strong,
            accent,
            accent_soft_bg: rgba_with_alpha(accent, 0.16),
            warn,
            danger,
            ok_green,
            text,
            text_strong,
            text_dim,
            titlebar_bg,
            titlebar_symbol,
            // Matches `color-mix(in srgb, var(--accent) 70%, transparent)`
            active_pane_ring: rgba_with_alpha(accent, 0.70),
            broadcast_pane_ring: rgba_with_alpha(warn, 0.82),
            broadcast_banner_bg: color_mix_srgb(warn, 0.14, chrome),
            broadcast_banner_fg: gpui::rgb(0xffd98f),
            caption_close_hover_bg: gpui::rgb(0xe81123),
        }
    }
}
