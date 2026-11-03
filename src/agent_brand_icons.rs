//! Pure-Rust vector brand and shell icon painter (`BrandIcon.vue` + `brandSvgs.js` parity).
//!
//! Renders crisp, DPI-independent vector logos directly onto an [`egui::Painter`] with zero
//! runtime file I/O, zero external image decoding, and zero font-glyph approximations for logos:
//! - Normalizes aliases (`"cursor-agent"` -> `"cursor"`, `"openai"` -> `"codex"`, case-insensitive).
//! - Scales every mark into `BrandIcon.vue`'s exact optical keylines inside a `24×24` coordinate
//!   frame (`MARK = 20.0`, `TILE = 18.5` with `TILE_RADIUS = 4.25` for filled-tile marks, and
//!   `21.0` for `gemini`).
//! - Renders multi-subpath SVG `<path>` icons (`codex`, `copilot`, `aider`, `qwen`, `grok`,
//!   `cline`, `omp`, `pi`, `kilo`) and `gemini` via `OnceLock`-cached vector geometry in source
//!   SVG space. Multi-contour `evenodd` paths are discretized once per brand into adaptive
//!   scanline-trapezoid `egui::Mesh`es (sampling cubic Béziers and elliptical arcs into closed
//!   polylines, splitting at horizontal edges and local Y-extrema plus bounded bisection at
//!   crossings) so holes and islands-in-holes are preserved while per-frame rendering only
//!   affinely transforms and colors cached vertices.
//! - Falls back to `BrandIcon.vue`'s rounded colored tile (`18.5/24`, `rx = 4.25/24`) with the
//!   uppercase initial in luminance-contrasted text (`lum > 150.0 => #14161b` else `#ffffff`), or
//!   the generic terminal glyph when neither brand nor label/accent is supplied.
//! - Applies `.sl-item.off` dimming (`0.55` opacity multiplier) when `dimmed == true`.
//!
//! Third-party attributions (`THIRD_PARTY_NOTICES.md`):
//! - `aider`, `omp`, `pi`, `kilo`, `amp`, `cursor`, `kimi`: adapted from Tessel / Orca
//!   (`Copyright (c) 2026 Lovecast Inc.`, MIT License).
//! - `grok`: adapted from Tessel's `brandSvgs.js`, after `lobehub/lobe-icons`
//!   (`Copyright (c) 2023-present LobeHub`, MIT License).

use std::borrow::Cow;
use std::sync::OnceLock;

use eframe::egui::{
    Align2, Color32, CornerRadius, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind,
    Vec2,
};

const VIEWBOX: f32 = 24.0;
const MARK: f32 = 20.0;
const TILE: f32 = 18.5;
const TILE_RADIUS: f32 = 4.25;

const BADGE_DARK_TEXT: Color32 = Color32::from_rgb(0x14, 0x16, 0x1b);
const DEFAULT_BADGE_ACCENT: Color32 = Color32::from_rgb(0x8a, 0x93, 0xa6);

// Exact SVG paths from `BrandIcon.vue` and `brandSvgs.js`.
const OPENAI_PATH: &str = "M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z";

const GEMINI_PATH: &str = "M12 1.5C12 7.3 16.7 12 22.5 12 16.7 12 12 16.7 12 22.5 12 16.7 7.3 12 1.5 12 7.3 12 12 7.3 12 1.5Z";

const COPILOT_PATH: &str = "M19.245 5.364c1.322 1.36 1.877 3.216 2.11 5.817.622 0 1.2.135 1.592.654l.73.964c.21.278.323.61.323.955v2.62c0 .339-.173.669-.453.868C20.239 19.602 16.157 21.5 12 21.5c-4.6 0-9.205-2.583-11.547-4.258-.28-.2-.452-.53-.453-.868v-2.62c0-.345.113-.679.321-.956l.73-.963c.392-.517.974-.654 1.593-.654l.029-.297c.25-2.446.81-4.213 2.082-5.52 2.461-2.54 5.71-2.851 7.146-2.864h.198c1.436.013 4.685.323 7.146 2.864zm-7.244 4.328c-.284 0-.613.016-.962.05-.123.447-.305.85-.57 1.108-1.05 1.023-2.316 1.18-2.994 1.18-.638 0-1.306-.13-1.851-.464-.516.165-1.012.403-1.044.996a65.882 65.882 0 00-.063 2.884l-.002.48c-.002.563-.005 1.126-.013 1.69.002.326.204.63.51.765 2.482 1.102 4.83 1.657 6.99 1.657 2.156 0 4.504-.555 6.985-1.657a.854.854 0 00.51-.766c.03-1.682.006-3.372-.076-5.053-.031-.596-.528-.83-1.046-.996-.546.333-1.212.464-1.85.464-.677 0-1.942-.157-2.993-1.18-.266-.258-.447-.661-.57-1.108-.32-.032-.64-.049-.96-.05zm-2.525 4.013c.539 0 .976.426.976.95v1.753c0 .525-.437.95-.976.95a.964.964 0 01-.976-.95v-1.752c0-.525.437-.951.976-.951zm5 0c.539 0 .976.426.976.95v1.753c0 .525-.437.95-.976.95a.964.964 0 01-.976-.95v-1.752c0-.525.437-.951.976-.951zM7.635 5.087c-1.05.102-1.935.438-2.385.906-.975 1.037-.765 3.668-.21 4.224.405.394 1.17.657 1.995.657h.09c.649-.013 1.785-.176 2.73-1.11.435-.41.705-1.433.675-2.47-.03-.834-.27-1.52-.63-1.813-.39-.336-1.275-.482-2.265-.394zm6.465.394c-.36.292-.6.98-.63 1.813-.03 1.037.24 2.06.675 2.47.968.957 2.136 1.104 2.776 1.11h.044c.825 0 1.59-.263 1.995-.657.555-.556.765-3.187-.21-4.224-.45-.468-1.335-.804-2.385-.906-.99-.088-1.875.058-2.265.394zM12 7.615c-.24 0-.525.015-.84.044.03.16.045.336.06.526l-.001.159a2.94 2.94 0 01-.014.25c.225-.022.425-.027.612-.028h.366c.187 0 .387.006.612.028-.015-.146-.015-.277-.015-.409.015-.19.03-.365.06-.526a9.29 9.29 0 00-.84-.044z";

const AIDER_PATH: &str = "M2705 3998 c20 -20 28 -121 30 -398 l2 -305 216 -5 c118 -3 218 -8 222 -12 3 -3 10 -46 15 -95 5 -48 16 -126 25 -172 17 -86 17 -81 -17 -233 -14 -67 -13 -365 2 -438 21 -100 22 -159 5 -247 -24 -122 -24 -363 1 -458 23 -88 23 -213 1 -330 -9 -49 -17 -109 -17 -132 l0 -43 203 0 c111 0 208 -4 216 -9 10 -6 18 -51 27 -148 8 -76 16 -152 20 -168 7 -39 -23 -361 -37 -387 -10 -18 -21 -19 -214 -16 -135 2 -208 7 -215 14 -22 22 -33 301 -21 501 6 102 8 189 5 194 -8 13 -417 12 -431 -2 -12 -12 -8 -146 8 -261 8 -55 8 -95 1 -140 -6 -35 -14 -99 -17 -143 -9 -123 -14 -141 -41 -154 -18 -8 -217 -11 -679 -11 l-653 0 -11 33 c-31 97 -43 336 -27 533 5 56 6 113 2 128 l-6 26 -194 0 c-211 0 -252 4 -261 28 -12 33 -17 392 -6 522 15 186 -2 174 260 180 115 3 213 8 217 12 4 4 1 52 -5 105 -7 54 -17 130 -22 168 -7 56 -5 91 11 171 10 55 22 130 26 166 4 36 10 72 15 79 7 12 128 15 665 19 l658 5 8 30 c5 18 4 72 -3 130 -12 115 -7 346 11 454 10 61 10 75 -1 82 -8 5 -300 9 -650 9 l-636 0 -27 25 c-18 16 -26 34 -26 57 0 18 -5 87 -10 153 -10 128 5 449 22 472 5 7 26 13 46 15 78 6 1281 3 1287 -4z M1360 1833 c0 -5 -1 -164 -3 -356 l-2 -347 625 -1 c704 -1 708 -1 722 7 5 4 7 20 4 38 -29 141 -32 491 -6 595 9 38 8 45 -7 57 -15 11 -139 13 -675 14 -362 0 -658 -3 -658 -7z";

const GROK_PATH: &str = "M9.27 15.29l7.978-5.897c.391-.29.95-.177 1.137.272.98 2.369.542 5.215-1.41 7.169-1.951 1.954-4.667 2.382-7.149 1.406l-2.711 1.257c3.889 2.661 8.611 2.003 11.562-.953 2.341-2.344 3.066-5.539 2.388-8.42l.006.007c-.983-4.232.242-5.924 2.75-9.383.06-.082.12-.164.179-.248l-3.301 3.305v-.01L9.267 15.292M7.623 16.723c-2.792-2.67-2.31-6.801.071-9.184 1.761-1.763 4.647-2.483 7.166-1.425l2.705-1.25a7.808 7.808 0 00-1.829-1A8.975 8.975 0 005.984 5.83c-2.533 2.536-3.33 6.436-1.962 9.764 1.022 2.487-.653 4.246-2.34 6.022-.599.63-1.199 1.259-1.682 1.925l7.62-6.815";

const CLINE_HEAD_PATH: &str = "M65.45 16.8c10.89 0 19.71 8.86 19.71 19.8v6.6l5.74 11.46a4 4 0 0 1-.01 3.6l-5.73 11.34v6.6c0 10.94-8.82 19.8-19.71 19.8H26.02C15.13 96 6.31 87.14 6.31 76.2v-6.6L.45 58.3a4 4 0 0 1-.01-3.67l5.87-11.43v-6.6c0-10.94 8.82-19.8 19.71-19.8h39.43Zm-2.52 5.7H29.19c-9.32 0-16.87 7.56-16.87 16.88V45L7.44 54.46a4 4 0 0 0 .01 3.68L12.32 67.5v5.63c0 9.32 7.55 16.87 16.87 16.87h33.74c9.32 0 16.87-7.55 16.87-16.87V67.5l4.77-9.39a4 4 0 0 0 .01-3.61L79.8 45v-5.62c0-9.32-7.55-16.88-16.87-16.88Z";

const QWEN_PATH: &str = "m140.93 85-16.35-28.33-1.93-3.34 8.66-15a3.32 3.32 0 0 0 0-3.34l-9.62-16.67a3.34 3.34 0 0 0-2.89-1.67H82.23l-8.66-15A3.33 3.33 0 0 0 70.68-.02H51.43a3.33 3.33 0 0 0-2.88 1.67L32.19 29.98l-1.92 3.33H12.96a3.34 3.34 0 0 0-2.88 1.67L.45 51.66a3.32 3.32 0 0 0 0 3.34l18.28 31.67-8.66 15a3.32 3.32 0 0 0 0 3.34l9.62 16.67a3.34 3.34 0 0 0 2.89 1.67h36.56l8.66 15a3.35 3.35 0 0 0 2.89 1.67h19.25a3.34 3.34 0 0 0 2.89-1.67l18.28-31.67h17.32a3.34 3.34 0 0 0 2.89-1.67l9.62-16.67a3.32 3.32 0 0 0-.01-3.34ZM51.44 3.33 61.07 20l-9.63 16.66h76.98l-9.62 16.66H45.67l-11.54-20zM57.21 120H22.58l9.63-16.67h19.25l-38.5-66.67h19.25l9.62 16.67L68.78 100l-11.55 20Zm61.59-33.34-9.62-16.67-38.49 66.67-9.63-16.67 9.63-16.66 26.94-46.67h23.1l17.32 30z";

const OMP_PATH: &str = "M10 14h44v9H43v33h-9V23h-9v22h-9V23H10z";

const PI_MAIN_PATH: &str = "M165.29 165.29 H517.36 V400 H400 V517.36 H282.65 V634.72 H165.29 Z M282.65 282.65 V400 H400 V282.65 Z";
const PI_DOT_PATH: &str = "M517.36 400 H634.72 V634.72 H517.36 Z";

const KILO_MARK_PATH: &str = "M322 377H377V421H307.857L278 391.143V322H322V377ZM421 307.857L391.143 278H322V322L377 322V377H421V307.857ZM234 278H190V322H234V278ZM91 391.143L120.857 421H234V377H135V278H91V391.143ZM371.172 189.999V120.856L341.315 90.9995H278V135H327.172V189.999H278V233.999H421V189.999H371.172ZM135 91H91V233.999H135V184.5H190V233.999H234V184.5L190 140.5H135V91ZM234 91H190V140.5H234V91Z";

/// Parses a `#RGB` or `#RRGGBB` CSS hex color (`BrandIcon.vue:46-57`).
#[must_use]
pub fn parse_hex_color(hex: &str) -> Option<Color32> {
    let raw = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
    match raw.len() {
        3 => {
            let mut buf = [0u8; 6];
            for (i, b) in raw.bytes().enumerate() {
                buf[i * 2] = b;
                buf[i * 2 + 1] = b;
            }
            let expanded = std::str::from_utf8(&buf).ok()?;
            let n = u32::from_str_radix(expanded, 16).ok()?;
            Some(Color32::from_rgb(
                ((n >> 16) & 0xff) as u8,
                ((n >> 8) & 0xff) as u8,
                (n & 0xff) as u8,
            ))
        }
        6 => {
            let n = u32::from_str_radix(raw, 16).ok()?;
            Some(Color32::from_rgb(
                ((n >> 16) & 0xff) as u8,
                ((n >> 8) & 0xff) as u8,
                (n & 0xff) as u8,
            ))
        }
        _ => None,
    }
}

/// Luminance-contrasted initial badge foreground (`BrandIcon.vue:46-59`).
#[must_use]
pub fn badge_text_color(accent: Color32) -> Color32 {
    let lum = f32::from(accent.r()) * 0.299_f32
        + f32::from(accent.g()) * 0.587_f32
        + f32::from(accent.b()) * 0.114_f32;
    if lum > 150.0_f32 {
        BADGE_DARK_TEXT
    } else {
        Color32::WHITE
    }
}

fn normalize_brand_id(id: &str) -> Cow<'_, str> {
    let trimmed = id.trim();
    if trimmed.bytes().any(|b| b.is_ascii_uppercase()) {
        let lower = trimmed.to_ascii_lowercase();
        match lower.as_str() {
            "cursor-agent" => Cow::Borrowed("cursor"),
            "openai" => Cow::Borrowed("codex"),
            _ => Cow::Owned(lower),
        }
    } else {
        match trimmed {
            "cursor-agent" => Cow::Borrowed("cursor"),
            "openai" => Cow::Borrowed("codex"),
            _ => Cow::Borrowed(trimmed),
        }
    }
}

/// Returns `true` when `id` resolves to a dedicated vector brand or shell mark rather than an
/// initial-letter badge fallback.
#[must_use]
pub fn has_vector_brand(id: &str) -> bool {
    matches!(
        normalize_brand_id(id).as_ref(),
        "claude"
            | "codex"
            | "opencode"
            | "opencode2"
            | "gemini"
            | "copilot"
            | "aider"
            | "amp"
            | "kimi"
            | "qwen"
            | "grok"
            | "cline"
            | "cursor"
            | "omp"
            | "pi"
            | "kilo"
            | "powershell"
            | "pwsh"
            | "cmd"
            | "gitbash"
            | "bash"
            | "git"
            | "wsl"
    )
}

fn dim(color: Color32, dimmed: bool) -> Color32 {
    if dimmed {
        color.gamma_multiply(0.55)
    } else {
        color
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    Color32::from_rgba_unmultiplied(
        (f32::from(a.r()) * inv + f32::from(b.r()) * t).round() as u8,
        (f32::from(a.g()) * inv + f32::from(b.g()) * t).round() as u8,
        (f32::from(a.b()) * inv + f32::from(b.b()) * t).round() as u8,
        (f32::from(a.a()) * inv + f32::from(b.a()) * t).round() as u8,
    )
}

#[derive(Clone, Copy)]
struct Frame24 {
    square: Rect,
    scale24: f32,
}

impl Frame24 {
    fn new(rect: Rect) -> Option<Self> {
        if !rect.is_positive() || !rect.min.x.is_finite() || !rect.min.y.is_finite() {
            return None;
        }
        let side = rect.width().min(rect.height());
        if side <= 0.5_f32 || !side.is_finite() {
            return None;
        }
        let square = Rect::from_center_size(rect.center(), Vec2::splat(side));
        Some(Self {
            square,
            scale24: side / VIEWBOX,
        })
    }

    fn pt24(self, x: f32, y: f32) -> Pos2 {
        Pos2::new(
            self.square.min.x + x * self.scale24,
            self.square.min.y + y * self.scale24,
        )
    }

    fn len24(self, v: f32) -> f32 {
        v * self.scale24
    }

    fn rect24(self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(
            self.pt24(x, y),
            Vec2::new(w * self.scale24, h * self.scale24),
        )
    }

    fn tile_rect(self) -> Rect {
        let off = (VIEWBOX - TILE) * 0.5;
        self.rect24(off, off, TILE, TILE)
    }

    fn tile_rounding(self) -> CornerRadius {
        CornerRadius::same(self.len24(TILE_RADIUS).round().clamp(1.0, 127.0) as u8)
    }

    fn subbox(self, box_xywh: (f32, f32, f32, f32), target_units: f32) -> SubBox {
        let (bx, by, bw, bh) = box_xywh;
        let frame_xy = (VIEWBOX - target_units) * 0.5;
        let inner_scale = (target_units / bw).min(target_units / bh);
        let ox = frame_xy + (target_units - bw * inner_scale) * 0.5;
        let oy = frame_xy + (target_units - bh * inner_scale) * 0.5;
        SubBox {
            outer: self,
            bx,
            by,
            ox,
            oy,
            inner_scale,
        }
    }
}

#[derive(Clone, Copy)]
struct SubBox {
    outer: Frame24,
    bx: f32,
    by: f32,
    ox: f32,
    oy: f32,
    inner_scale: f32,
}

impl SubBox {
    fn pt(self, x: f32, y: f32) -> Pos2 {
        let vx = self.ox + (x - self.bx) * self.inner_scale;
        let vy = self.oy + (y - self.by) * self.inner_scale;
        self.outer.pt24(vx, vy)
    }

    fn len(self, v: f32) -> f32 {
        self.outer.len24(v * self.inner_scale)
    }

    fn rect_xywh(self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_max(self.pt(x, y), self.pt(x + w, y + h))
    }
}

/// Paints a built-in [`crate::agent_catalog::Preset`] icon inside `rect`.
#[allow(dead_code)]
pub fn paint_preset(
    painter: &Painter,
    rect: Rect,
    preset: &crate::agent_catalog::Preset,
    dimmed: bool,
) {
    let accent = parse_hex_color(preset.accent).unwrap_or(DEFAULT_BADGE_ACCENT);
    paint(painter, rect, preset.id, preset.name, accent, dimmed);
}

/// Paints an agent or shell brand icon (`BrandIcon.vue` parity) inscribed 1:1 inside `rect`.
///
/// - `id`: canonical or alias identifier (`"claude"`, `"codex"`, `"cursor-agent"`, `"cursor"`, …).
/// - `label`: human display name used for the uppercase initial fallback on colored tiles.
/// - `accent`: brand accent color (`Color32::TRANSPARENT` uses `#8a93a6` if falling back to tile).
/// - `dimmed`: multiplies opacity by `0.55` when the agent is hidden from menus (`.sl-item.off`).
pub fn paint(painter: &Painter, rect: Rect, id: &str, label: &str, accent: Color32, dimmed: bool) {
    let Some(f) = Frame24::new(rect) else {
        return;
    };
    let kind = normalize_brand_id(id);
    match kind.as_ref() {
        "claude" => paint_claude(painter, f, dimmed),
        "codex" => paint_cached_svg(
            painter,
            f.subbox((0.16, 0.0, 23.68, 24.0), MARK),
            codex_mesh(),
            dim(Color32::from_rgb(0xec, 0xec, 0xec), dimmed),
        ),
        "opencode" | "opencode2" => paint_opencode(painter, f, dimmed),
        "gemini" => paint_gemini(painter, f, dimmed),
        "copilot" => paint_cached_svg(
            painter,
            f.subbox((0.0, 2.5, 24.0, 19.0), MARK),
            copilot_mesh(),
            dim(Color32::WHITE, dimmed),
        ),
        "aider" => paint_cached_svg(
            painter,
            f.subbox((85.02, 35.24, 280.86, 360.79), MARK),
            aider_mesh(),
            dim(Color32::from_rgb(0xe6, 0xe6, 0xe6), dimmed),
        ),
        "amp" => paint_amp(painter, f, dimmed),
        "kimi" => paint_kimi(painter, f, dimmed),
        "qwen" => paint_cached_svg(
            painter,
            f.subbox((0.0, 0.0, 141.38, 139.97), MARK),
            qwen_mesh(),
            dim(Color32::from_rgb(0x6d, 0x44, 0xe8), dimmed),
        ),
        "grok" => paint_cached_svg(
            painter,
            f.subbox((0.0, 0.5, 24.0, 23.04), MARK),
            grok_mesh(),
            dim(Color32::from_rgb(0xe6, 0xe6, 0xe6), dimmed),
        ),
        "cline" => paint_cline(painter, f, dimmed),
        "cursor" => paint_cursor(painter, f, dimmed),
        "omp" => paint_omp(painter, f, dimmed),
        "pi" => paint_pi(painter, f, dimmed),
        "kilo" => paint_kilo(painter, f, dimmed),
        "powershell" => paint_powershell(painter, f, false, dimmed),
        "pwsh" => paint_powershell(painter, f, true, dimmed),
        "cmd" => paint_cmd(painter, f, dimmed),
        "gitbash" | "bash" | "git" => paint_git(painter, f, dimmed),
        "wsl" => paint_wsl(painter, f, dimmed),
        _ => {
            if !label.trim().is_empty() || accent != Color32::TRANSPARENT {
                paint_initial_tile(painter, f, id, label, accent, dimmed);
            } else {
                paint_fallback_terminal(painter, f, dimmed);
            }
        }
    }
}

fn paint_claude(painter: &Painter, f: Frame24, dimmed: bool) {
    let sb = f.subbox((0.82, 0.14, 21.72, 23.2), MARK);
    let lens: [f32; 12] = [
        1.0, 0.74, 0.92, 0.68, 0.98, 0.8, 0.94, 0.7, 1.0, 0.76, 0.9, 0.72,
    ];
    let color = dim(Color32::from_rgb(0xd9, 0x77, 0x57), dimmed);
    let stroke_w = sb.len(2.3).max(1.0);
    let stroke = Stroke::new(stroke_w, color);
    let cap_r = stroke_w * 0.5;
    for (i, l) in lens.into_iter().enumerate() {
        let a = ((i as f32) * 30.0 - 82.0).to_radians();
        let (sa, ca) = a.sin_cos();
        let r0 = 2.2;
        let r1 = 2.2 + l * 8.6;
        let p0 = sb.pt(12.0 + ca * r0, 12.0 + sa * r0);
        let p1 = sb.pt(12.0 + ca * r1, 12.0 + sa * r1);
        painter.line_segment([p0, p1], stroke);
        if cap_r >= 0.45 {
            painter.circle_filled(p0, cap_r, color);
            painter.circle_filled(p1, cap_r, color);
        }
    }
}

fn paint_opencode(painter: &Painter, f: Frame24, dimmed: bool) {
    let sb = f.subbox((4.0, 2.0, 16.0, 20.0), MARK);
    let inner = dim(Color32::from_rgb(0x4b, 0x46, 0x46), dimmed);
    let outer = dim(Color32::from_rgb(0xf1, 0xec, 0xec), dimmed);
    painter.rect_filled(sb.rect_xywh(8.0, 10.0, 8.0, 8.0), 0.0, inner);
    // Exact `evenodd` decomposition of `M4 2h16v20H4ZM8 6v12h8V6Z`:
    painter.rect_filled(sb.rect_xywh(4.0, 2.0, 16.0, 4.0), 0.0, outer);
    painter.rect_filled(sb.rect_xywh(4.0, 18.0, 16.0, 4.0), 0.0, outer);
    painter.rect_filled(sb.rect_xywh(4.0, 6.0, 4.0, 12.0), 0.0, outer);
    painter.rect_filled(sb.rect_xywh(16.0, 6.0, 4.0, 12.0), 0.0, outer);
}

fn paint_gemini(painter: &Painter, f: Frame24, dimmed: bool) {
    static GEMINI_RING: OnceLock<Vec<(f32, f32)>> = OnceLock::new();
    let ring = GEMINI_RING.get_or_init(|| {
        parse_svg_subpaths(GEMINI_PATH)
            .into_iter()
            .next()
            .map_or_else(Vec::new, |sub| sub.points)
    });
    if ring.len() < 4 {
        return;
    }

    let sb = f.subbox((1.5, 1.5, 21.0, 21.0), 21.0);
    let c0 = dim(Color32::from_rgb(0x42, 0x85, 0xf4), dimmed);
    let c1 = dim(Color32::from_rgb(0x9b, 0x72, 0xcb), dimmed);
    let c2 = dim(Color32::from_rgb(0xd9, 0x65, 0x70), dimmed);
    let grad_at = |x: f32, y: f32| -> Color32 {
        // SVG linearGradient `(3, 21) -> (21, 3)` (`dx = 18, dy = -18, len^2 = 648`).
        let t = (((x - 3.0) * 18.0 + (y - 21.0) * (-18.0)) / 648.0).clamp(0.0, 1.0);
        if t <= 0.55 {
            lerp_color(c0, c1, t / 0.55)
        } else {
            lerp_color(c1, c2, (t - 0.55) / 0.45)
        }
    };

    // Star-shaped with respect to `(12, 12)` over sampled cubic-Bézier ring vertices.
    let mut mesh = Mesh::default();
    mesh.vertices.reserve(ring.len() + 1);
    mesh.indices.reserve(ring.len() * 3);
    mesh.colored_vertex(sb.pt(12.0, 12.0), grad_at(12.0, 12.0));
    for &(x, y) in ring {
        mesh.colored_vertex(sb.pt(x, y), grad_at(x, y));
    }
    let ring_len = ring.len() as u32;
    for i in 0..ring_len {
        let next = (i + 1) % ring_len;
        mesh.add_triangle(0, 1 + i, 1 + next);
    }
    painter.add(Shape::mesh(mesh));
}

fn paint_cline(painter: &Painter, f: Frame24, dimmed: bool) {
    let sb = f.subbox((0.0, 0.48, 91.36, 95.52), MARK);
    let color = dim(Color32::from_rgb(0xe6, 0xe6, 0xe6), dimmed);
    paint_cached_svg(painter, sb, cline_head_mesh(), color);
    painter.circle_filled(sb.pt(45.73, 11.5), sb.len(11.0), color);
    let eye_rx = CornerRadius::same(sb.len(6.5).round().clamp(1.0, 127.0) as u8);
    painter.rect_filled(sb.rect_xywh(27.0, 41.0, 13.0, 30.0), eye_rx, color);
    painter.rect_filled(sb.rect_xywh(51.0, 41.0, 13.0, 30.0), eye_rx, color);
}

fn paint_amp(painter: &Painter, f: Frame24, dimmed: bool) {
    // Inspected directly from `assets/agent-icons/amp.png` (`64x64`, coral `#f34e3f`):
    // 3 nested tilted right-angle chevrons + diagonal stem from the inner chevron vertex.
    let sb = f.subbox((0.0, 0.0, 24.0, 24.0), MARK);
    let color = dim(Color32::from_rgb(0xf3, 0x4e, 0x3f), dimmed);
    let w = sb.len(2.75).max(1.1);
    let stroke = Stroke::new(w, color);

    let brackets: [[(f32, f32); 3]; 3] = [
        [(9.2, 2.3), (19.6, 5.1), (22.3, 15.5)],
        [(5.3, 6.2), (15.4, 8.9), (18.1, 19.0)],
        [(1.5, 10.1), (11.2, 12.7), (13.8, 22.4)],
    ];
    for pts in brackets {
        let p0 = sb.pt(pts[0].0, pts[0].1);
        let p1 = sb.pt(pts[1].0, pts[1].1);
        let p2 = sb.pt(pts[2].0, pts[2].1);
        painter.add(Shape::line(vec![p0, p1, p2], stroke));
    }
    let stem_a = sb.pt(11.0, 12.9);
    let stem_b = sb.pt(3.3, 20.6);
    painter.line_segment([stem_a, stem_b], stroke);
}

fn paint_cursor(painter: &Painter, f: Frame24, dimmed: bool) {
    // Inspected directly from `assets/agent-icons/cursor.png` (`TILE = 18.5`, `rx = 4.25`):
    // `#141414` dark rounded tile + `#efefef` point-top hexagon + `#141414` isometric cursor cutout.
    let bg = dim(Color32::from_rgb(0x14, 0x14, 0x14), dimmed);
    let fg = dim(Color32::from_rgb(0xef, 0xef, 0xef), dimmed);
    painter.rect_filled(f.tile_rect(), f.tile_rounding(), bg);

    let r = 6.7_f32;
    let dx = r * 0.866_025_4_f32;
    let dy = r * 0.5_f32;
    let hex = vec![
        f.pt24(12.0, 12.0 - r),
        f.pt24(12.0 + dx, 12.0 - dy),
        f.pt24(12.0 + dx, 12.0 + dy),
        f.pt24(12.0, 12.0 + r),
        f.pt24(12.0 - dx, 12.0 + dy),
        f.pt24(12.0 - dx, 12.0 - dy),
    ];
    painter.add(Shape::convex_polygon(hex, fg, Stroke::NONE));

    // Concave cursor cutout decomposed into 2 exact convex triangles.
    let ul = f.pt24(12.0 - dx + 0.65, 12.0 - dy + 0.35);
    let ur = f.pt24(12.0 + dx - 0.65, 12.0 - dy + 0.35);
    let mid = f.pt24(12.0, 12.05);
    let bot = f.pt24(12.0, 12.0 + r - 0.75);
    painter.add(Shape::convex_polygon(vec![ul, ur, mid], bg, Stroke::NONE));
    painter.add(Shape::convex_polygon(vec![mid, ur, bot], bg, Stroke::NONE));
}

fn paint_kimi(painter: &Painter, f: Frame24, dimmed: bool) {
    // Inspected directly from `assets/agent-icons/kimi.png` (`TILE = 18.5`, `rx = 4.25`):
    // `#14181f` dark rounded tile + `#f2f4f7` lunar disc + tilted cloud bands from the right.
    let bg = dim(Color32::from_rgb(0x14, 0x18, 0x1f), dimmed);
    let moon = dim(Color32::from_rgb(0xf2, 0xf4, 0xf7), dimmed);
    let tile = f.tile_rect();
    painter.rect_filled(tile, f.tile_rounding(), bg);
    painter.circle_filled(f.pt24(12.0, 12.0), f.len24(6.15), moon);

    let band_w = f.len24(1.45).max(0.9);
    let stroke = Stroke::new(band_w, bg);
    let cap_r = band_w * 0.5;
    let bands: [((f32, f32), (f32, f32)); 4] = [
        ((14.1, 8.2), (18.6, 10.1)),
        ((12.4, 10.6), (18.6, 13.2)),
        ((11.6, 13.3), (18.3, 16.1)),
        ((12.7, 16.0), (17.5, 18.0)),
    ];
    let clipped = painter.with_clip_rect(tile.shrink(f.len24(0.6)));
    for ((x0, y0), (x1, y1)) in bands {
        let p0 = f.pt24(x0, y0);
        let p1 = f.pt24(x1, y1);
        clipped.line_segment([p0, p1], stroke);
        clipped.circle_filled(p0, cap_r, bg);
    }
}

fn paint_omp(painter: &Painter, f: Frame24, dimmed: bool) {
    let sb = f.subbox((9.97, 13.97, 44.05, 42.03), TILE);
    let c0 = dim(Color32::from_rgb(0xec, 0x48, 0x99), dimmed);
    let c1 = dim(Color32::from_rgb(0x8b, 0x5c, 0xf6), dimmed);
    let c2 = dim(Color32::from_rgb(0x38, 0xd0, 0xe0), dimmed);
    let grad = |x: f32, y: f32| -> Color32 {
        let tx = (x - 10.0) / 44.0;
        let ty = (y - 14.0) / 42.0;
        let t = ((tx + ty) * 0.5).clamp(0.0, 1.0);
        if t <= 0.5 {
            lerp_color(c0, c1, t * 2.0)
        } else {
            lerp_color(c1, c2, (t - 0.5) * 2.0)
        }
    };
    // Exact decomposition of `M10 14h44v9H43v33h-9V23h-9v22h-9V23H10z` into 3 axis-aligned bars:
    let mut mesh = Mesh::default();
    for (x, y, w, h) in [
        (10.0_f32, 14.0_f32, 44.0_f32, 9.0_f32),
        (16.0_f32, 23.0_f32, 9.0_f32, 22.0_f32),
        (34.0_f32, 23.0_f32, 9.0_f32, 33.0_f32),
    ] {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(sb.pt(x, y), grad(x, y));
        mesh.colored_vertex(sb.pt(x + w, y), grad(x + w, y));
        mesh.colored_vertex(sb.pt(x + w, y + h), grad(x + w, y + h));
        mesh.colored_vertex(sb.pt(x, y + h), grad(x, y + h));
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    let _ = OMP_PATH;
    painter.add(Shape::mesh(mesh));
}

fn paint_pi(painter: &Painter, f: Frame24, dimmed: bool) {
    let sb = f.subbox((164.67, 165.33, 470.67, 469.33), TILE);
    let color = dim(Color32::from_rgb(0xe6, 0xe6, 0xe6), dimmed);
    paint_cached_svg(painter, sb, pi_mesh(), color);
}

fn paint_kilo(painter: &Painter, f: Frame24, dimmed: bool) {
    let bg = dim(Color32::BLACK, dimmed);
    let fg = dim(Color32::from_rgb(0xfa, 0xf7, 0x4f), dimmed);
    painter.rect_filled(f.tile_rect(), f.tile_rounding(), bg);
    let sb = f.subbox((0.0, 0.0, 512.0, 512.0), TILE);
    paint_cached_svg(painter, sb, kilo_mark_mesh(), fg);
}

fn paint_powershell(painter: &Painter, f: Frame24, pwsh7: bool, dimmed: bool) {
    let sb = f.subbox((1.0, 3.0, 22.0, 18.0), MARK);
    let plate = dim(
        if pwsh7 {
            Color32::from_rgb(0x34, 0x50, 0x7a)
        } else {
            Color32::from_rgb(0x26, 0x71, 0xbe)
        },
        dimmed,
    );
    let fg = dim(Color32::WHITE, dimmed);
    let poly = vec![
        sb.pt(6.0, 4.0),
        sb.pt(22.0, 4.0),
        sb.pt(18.0, 20.0),
        sb.pt(2.0, 20.0),
    ];
    painter.add(Shape::convex_polygon(
        poly,
        plate,
        Stroke::new(sb.len(1.6), plate),
    ));
    let prompt_stroke = Stroke::new(sb.len(1.9).max(1.0), fg);
    painter.add(Shape::line(
        vec![sb.pt(7.0, 8.0), sb.pt(12.0, 12.0), sb.pt(5.5, 16.5)],
        prompt_stroke,
    ));
    painter.line_segment([sb.pt(12.0, 16.5), sb.pt(16.5, 16.5)], prompt_stroke);
}

fn paint_cmd(painter: &Painter, f: Frame24, dimmed: bool) {
    let sb = f.subbox((1.4, 2.9, 21.2, 18.2), MARK);
    let bg = dim(Color32::from_rgb(0x0c, 0x0c, 0x0c), dimmed);
    let border = dim(Color32::from_rgb(0x5a, 0x5a, 0x5a), dimmed);
    let fg = dim(Color32::from_rgb(0xe6, 0xe6, 0xe6), dimmed);
    let box_rect = sb.rect_xywh(2.0, 3.5, 20.0, 17.0);
    let rx = CornerRadius::same(sb.len(2.5).round().clamp(1.0, 127.0) as u8);
    painter.rect_filled(box_rect, rx, bg);
    painter.rect_stroke(
        box_rect,
        rx,
        Stroke::new(sb.len(1.2).max(0.8), border),
        StrokeKind::Middle,
    );
    let stroke = Stroke::new(sb.len(1.7).max(1.0), fg);
    painter.add(Shape::line(
        vec![sb.pt(6.0, 9.5), sb.pt(9.2, 12.0), sb.pt(6.0, 14.5)],
        stroke,
    ));
    painter.line_segment([sb.pt(11.0, 15.0), sb.pt(17.0, 15.0)], stroke);
}

fn paint_git(painter: &Painter, f: Frame24, dimmed: bool) {
    let sb = f.subbox((2.04, 2.04, 19.92, 19.92), MARK);
    let orange = dim(Color32::from_rgb(0xf0, 0x50, 0x32), dimmed);
    let fg = dim(Color32::WHITE, dimmed);
    let half_diag = 7.8 * std::f32::consts::SQRT_2;
    let diamond = vec![
        sb.pt(12.0, 12.0 - half_diag),
        sb.pt(12.0 + half_diag, 12.0),
        sb.pt(12.0, 12.0 + half_diag),
        sb.pt(12.0 - half_diag, 12.0),
    ];
    painter.add(Shape::convex_polygon(diamond, orange, Stroke::NONE));
    let stroke = Stroke::new(sb.len(1.5).max(0.9), fg);
    painter.line_segment([sb.pt(9.3, 7.6), sb.pt(14.3, 12.6)], stroke);
    painter.line_segment([sb.pt(12.0, 10.3), sb.pt(12.0, 16.3)], stroke);
    for (cx, cy) in [(12.0, 10.3), (14.6, 12.9), (12.0, 16.3)] {
        painter.circle_filled(sb.pt(cx, cy), sb.len(1.5), fg);
    }
}

fn paint_wsl(painter: &Painter, f: Frame24, dimmed: bool) {
    let sb = f.subbox((2.0, 3.5, 20.0, 17.0), MARK);
    let orange = dim(Color32::from_rgb(0xe9, 0x54, 0x20), dimmed);
    let fg = dim(Color32::WHITE, dimmed);
    let rx = CornerRadius::same(sb.len(3.0).round().clamp(1.0, 127.0) as u8);
    painter.rect_filled(sb.rect_xywh(2.0, 3.5, 20.0, 17.0), rx, orange);
    let stroke = Stroke::new(sb.len(1.8).max(1.0), fg);
    painter.add(Shape::line(
        vec![sb.pt(6.5, 9.5), sb.pt(9.7, 12.0), sb.pt(6.5, 14.5)],
        stroke,
    ));
    painter.line_segment([sb.pt(11.5, 15.0), sb.pt(17.5, 15.0)], stroke);
}

fn paint_initial_tile(
    painter: &Painter,
    f: Frame24,
    id: &str,
    label: &str,
    accent: Color32,
    dimmed: bool,
) {
    let base_accent = if accent == Color32::TRANSPARENT {
        DEFAULT_BADGE_ACCENT
    } else {
        accent
    };
    let tile_fill = dim(base_accent, dimmed);
    let text_fill = dim(badge_text_color(base_accent), dimmed);
    let tile = f.tile_rect();
    painter.rect_filled(tile, f.tile_rounding(), tile_fill);

    let source = if !label.trim().is_empty() {
        label.trim()
    } else if !id.trim().is_empty() {
        id.trim()
    } else {
        "?"
    };
    let initial: String = source
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .collect();
    let font_px = f.len24(12.0).max(7.0);
    painter.text(
        tile.center(),
        Align2::CENTER_CENTER,
        initial,
        FontId::proportional(font_px),
        text_fill,
    );
}

fn paint_fallback_terminal(painter: &Painter, f: Frame24, dimmed: bool) {
    let fg = dim(Color32::from_rgb(0xd6, 0xd9, 0xdf), dimmed);
    let rx = CornerRadius::same(f.len24(3.0).round().clamp(1.0, 127.0) as u8);
    painter.rect_stroke(
        f.rect24(2.0, 3.5, 20.0, 17.0),
        rx,
        Stroke::new(f.len24(1.5).max(0.9), fg),
        StrokeKind::Middle,
    );
    let stroke = Stroke::new(f.len24(1.6).max(0.9), fg);
    painter.add(Shape::line(
        vec![f.pt24(6.5, 9.5), f.pt24(9.7, 12.0), f.pt24(6.5, 14.5)],
        stroke,
    ));
    painter.line_segment([f.pt24(11.5, 15.0), f.pt24(17.5, 15.0)], stroke);
}

// ---------------------------------------------------------------------------
// `OnceLock`-cached `evenodd` scanline-trapezoid SVG `<path>` discretization
// ---------------------------------------------------------------------------
//
// Design & approximation notes:
// - Cubic Béziers (`C`/`S`, 12 segments) and elliptical arcs (`A`, 4..=24 segments) are flattened
//   once per brand into closed polylines in source SVG coordinates.
// - Horizontal Y-bands are seeded from `BASE_SLABS = 96` uniform slices merged with every contour
//   horizontal-edge Y and local Y-extremum Y, plus recursive bisection (`MAX_BISECT_DEPTH = 4`)
//   wherever top/bottom `evenodd` horizontal span counts or overlaps change (such as segment
//   crossings or cusps).
// - Each compatible band emits sloped trapezoids (`2` triangles per active `evenodd` interval),
//   preserving interior holes (e.g. `codex`, `aider`, `cline`, `pi`, `qwen`) and islands-in-holes
//   (e.g. `copilot`'s eye pills inside the visor cutout).
// - The resulting `(positions, indices)` buffer is stored in a `static OnceLock<CachedSvgMesh>`
//   per brand; per-frame rendering only affinely maps `(x, y) -> sb.pt(x, y)` and assigns `color`.

struct Subpath {
    points: Vec<(f32, f32)>,
}

struct CachedSvgMesh {
    positions: Vec<(f32, f32)>,
    indices: Vec<u32>,
}

impl CachedSvgMesh {
    fn push_quad(&mut self, a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)) {
        let base = self.positions.len() as u32;
        self.positions.extend_from_slice(&[a, b, c, d]);
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

fn codex_mesh() -> &'static CachedSvgMesh {
    static MESH: OnceLock<CachedSvgMesh> = OnceLock::new();
    MESH.get_or_init(|| build_evenodd_svg_mesh(&[OPENAI_PATH], None))
}

fn copilot_mesh() -> &'static CachedSvgMesh {
    static MESH: OnceLock<CachedSvgMesh> = OnceLock::new();
    MESH.get_or_init(|| build_evenodd_svg_mesh(&[COPILOT_PATH], None))
}

fn aider_mesh() -> &'static CachedSvgMesh {
    static MESH: OnceLock<CachedSvgMesh> = OnceLock::new();
    MESH.get_or_init(|| {
        build_evenodd_svg_mesh(&[AIDER_PATH], Some(|x, y| (x * 0.1, 436.0 - y * 0.1)))
    })
}

fn qwen_mesh() -> &'static CachedSvgMesh {
    static MESH: OnceLock<CachedSvgMesh> = OnceLock::new();
    MESH.get_or_init(|| build_evenodd_svg_mesh(&[QWEN_PATH], None))
}

fn grok_mesh() -> &'static CachedSvgMesh {
    static MESH: OnceLock<CachedSvgMesh> = OnceLock::new();
    MESH.get_or_init(|| build_evenodd_svg_mesh(&[GROK_PATH], None))
}

fn cline_head_mesh() -> &'static CachedSvgMesh {
    static MESH: OnceLock<CachedSvgMesh> = OnceLock::new();
    MESH.get_or_init(|| build_evenodd_svg_mesh(&[CLINE_HEAD_PATH], None))
}

fn pi_mesh() -> &'static CachedSvgMesh {
    static MESH: OnceLock<CachedSvgMesh> = OnceLock::new();
    MESH.get_or_init(|| build_evenodd_svg_mesh(&[PI_MAIN_PATH, PI_DOT_PATH], None))
}

fn kilo_mark_mesh() -> &'static CachedSvgMesh {
    static MESH: OnceLock<CachedSvgMesh> = OnceLock::new();
    MESH.get_or_init(|| build_evenodd_svg_mesh(&[KILO_MARK_PATH], None))
}

fn paint_cached_svg(painter: &Painter, sb: SubBox, geom: &CachedSvgMesh, color: Color32) {
    if geom.indices.is_empty() {
        return;
    }
    let mut mesh = Mesh::default();
    mesh.vertices.reserve(geom.positions.len());
    for &(x, y) in &geom.positions {
        mesh.colored_vertex(sb.pt(x, y), color);
    }
    mesh.indices.extend_from_slice(&geom.indices);
    painter.add(Shape::mesh(mesh));
}

type SvgPointTransform = fn(f32, f32) -> (f32, f32);

fn build_evenodd_svg_mesh(
    paths: &[&str],
    pre_transform: Option<SvgPointTransform>,
) -> CachedSvgMesh {
    let mut contours: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for &d in paths {
        for sub in parse_svg_subpaths(d) {
            if sub.points.len() < 3 {
                continue;
            }
            let mut pts: Vec<(f32, f32)> = Vec::with_capacity(sub.points.len());
            for (rx, ry) in sub.points {
                let (tx, ty) = match pre_transform {
                    Some(tf) => tf(rx, ry),
                    None => (rx, ry),
                };
                if pts.last().is_none_or(|&(px, py)| {
                    let dx = tx - px;
                    let dy = ty - py;
                    dx * dx + dy * dy > 1e-8_f32
                }) {
                    min_x = min_x.min(tx);
                    max_x = max_x.max(tx);
                    min_y = min_y.min(ty);
                    max_y = max_y.max(ty);
                    pts.push((tx, ty));
                }
            }
            if pts.len() >= 3 {
                contours.push(pts);
            }
        }
    }

    let mut out = CachedSvgMesh {
        positions: Vec::new(),
        indices: Vec::new(),
    };
    if contours.is_empty()
        || !min_x.is_finite()
        || !min_y.is_finite()
        || max_x <= min_x
        || max_y <= min_y
    {
        return out;
    }

    let span_x = (max_x - min_x).max(1e-3_f32);
    let span_y = (max_y - min_y).max(1e-3_f32);
    let min_w = span_x * 1e-5_f32;
    let min_h = span_y * 1e-4_f32;

    const BASE_SLABS: usize = 96;
    let mut ys = Vec::with_capacity(BASE_SLABS + 64);
    for s in 0..=BASE_SLABS {
        ys.push(min_y + span_y * (s as f32 / BASE_SLABS as f32));
    }
    for pts in &contours {
        let n = pts.len();
        for i in 0..n {
            let prev = pts[(i + n - 1) % n];
            let cur = pts[i];
            let next = pts[(i + 1) % n];
            let horiz = (cur.1 - prev.1).abs() <= min_h || (next.1 - cur.1).abs() <= min_h;
            let extremum = (prev.1 - cur.1) * (next.1 - cur.1) > 0.0_f32;
            if horiz || extremum {
                ys.push(cur.1.clamp(min_y, max_y));
            }
        }
    }
    ys.sort_by(f32::total_cmp);
    let mut merged_ys: Vec<f32> = Vec::with_capacity(ys.len());
    for y in ys {
        if merged_ys.last().is_none_or(|&prev| y - prev > min_h) {
            merged_ys.push(y);
        }
    }
    if let Some(last) = merged_ys.last_mut() {
        *last = max_y;
    }

    for band in merged_ys.windows(2) {
        emit_band(&mut out, &contours, band[0], band[1], min_w, min_h, 0);
    }
    out
}

fn emit_band(
    out: &mut CachedSvgMesh,
    contours: &[Vec<(f32, f32)>],
    y0: f32,
    y1: f32,
    min_w: f32,
    min_h: f32,
    depth: u8,
) {
    const MAX_BISECT_DEPTH: u8 = 4;
    let h = y1 - y0;
    if h <= min_h * 0.25_f32 {
        return;
    }
    let eps = h * 0.005_f32;
    let top = evenodd_intervals_at_y(contours, y0 + eps, min_w);
    let bot = evenodd_intervals_at_y(contours, y1 - eps, min_w);

    let compatible = top.len() == bot.len()
        && top
            .iter()
            .zip(&bot)
            .all(|(t, b)| t.0.max(b.0) <= t.1.min(b.1) + h * 2.5_f32);

    if compatible {
        for ((t0, t1), (b0, b1)) in top.into_iter().zip(bot) {
            out.push_quad((t0, y0), (t1, y0), (b1, y1), (b0, y1));
        }
    } else if depth < MAX_BISECT_DEPTH && h > min_h {
        let ym = (y0 + y1) * 0.5_f32;
        emit_band(out, contours, y0, ym, min_w, min_h, depth + 1);
        emit_band(out, contours, ym, y1, min_w, min_h, depth + 1);
    } else {
        let yc = (y0 + y1) * 0.5_f32;
        for (x0, x1) in evenodd_intervals_at_y(contours, yc, min_w) {
            out.push_quad((x0, y0), (x1, y0), (x1, y1), (x0, y1));
        }
    }
}

fn evenodd_intervals_at_y(contours: &[Vec<(f32, f32)>], y: f32, min_width: f32) -> Vec<(f32, f32)> {
    let mut xs: Vec<f32> = Vec::new();
    for pts in contours {
        let n = pts.len();
        for i in 0..n {
            let p0 = pts[i];
            let p1 = pts[(i + 1) % n];
            if (p0.1 <= y && p1.1 > y) || (p1.1 <= y && p0.1 > y) {
                let t = (y - p0.1) / (p1.1 - p0.1);
                xs.push(p0.0 + t * (p1.0 - p0.0));
            }
        }
    }
    xs.sort_by(f32::total_cmp);
    let mut out = Vec::with_capacity(xs.len() / 2);
    for &[x0, x1] in xs.as_chunks::<2>().0 {
        if x1 > x0 + min_width {
            out.push((x0, x1));
        }
    }
    out
}

struct SvgScanner<'a> {
    bytes: &'a [u8],
    idx: usize,
}

impl<'a> SvgScanner<'a> {
    fn new(d: &'a str) -> Self {
        Self {
            bytes: d.as_bytes(),
            idx: 0,
        }
    }

    fn skip_sep(&mut self) {
        while let Some(&b) = self.bytes.get(self.idx) {
            if b.is_ascii_whitespace() || b == b',' {
                self.idx += 1;
            } else {
                break;
            }
        }
    }

    fn next_cmd(&mut self) -> Option<u8> {
        self.skip_sep();
        let &b = self.bytes.get(self.idx)?;
        if b.is_ascii_alphabetic() {
            self.idx += 1;
            Some(b)
        } else {
            None
        }
    }

    fn has_number_ahead(&mut self) -> bool {
        self.skip_sep();
        matches!(
            self.bytes.get(self.idx),
            Some(b'0'..=b'9' | b'+' | b'-' | b'.')
        )
    }

    fn read_flag(&mut self) -> Option<bool> {
        self.skip_sep();
        match self.bytes.get(self.idx)? {
            b'0' => {
                self.idx += 1;
                Some(false)
            }
            b'1' => {
                self.idx += 1;
                Some(true)
            }
            _ => None,
        }
    }

    fn read_number(&mut self) -> Option<f32> {
        self.skip_sep();
        let start = self.idx;
        if matches!(self.bytes.get(self.idx), Some(b'+' | b'-')) {
            self.idx += 1;
        }
        let mut saw_digit = false;
        while matches!(self.bytes.get(self.idx), Some(b'0'..=b'9')) {
            saw_digit = true;
            self.idx += 1;
        }
        if self.bytes.get(self.idx) == Some(&b'.') {
            self.idx += 1;
            while matches!(self.bytes.get(self.idx), Some(b'0'..=b'9')) {
                saw_digit = true;
                self.idx += 1;
            }
        }
        if !saw_digit {
            self.idx = start;
            return None;
        }
        if matches!(self.bytes.get(self.idx), Some(b'e' | b'E')) {
            let exp_start = self.idx;
            self.idx += 1;
            if matches!(self.bytes.get(self.idx), Some(b'+' | b'-')) {
                self.idx += 1;
            }
            let mut exp_digit = false;
            while matches!(self.bytes.get(self.idx), Some(b'0'..=b'9')) {
                exp_digit = true;
                self.idx += 1;
            }
            if !exp_digit {
                self.idx = exp_start;
            }
        }
        let slice = std::str::from_utf8(&self.bytes[start..self.idx]).ok()?;
        slice.parse::<f32>().ok()
    }
}

fn parse_svg_subpaths(d: &str) -> Vec<Subpath> {
    let mut scan = SvgScanner::new(d);
    let mut out: Vec<Subpath> = Vec::new();
    let mut cur_pts: Vec<(f32, f32)> = Vec::new();
    let mut cur = (0.0_f32, 0.0_f32);
    let mut start = (0.0_f32, 0.0_f32);
    let mut prev_cubic_ctrl: Option<(f32, f32)> = None;
    let mut cmd = b'M';

    while scan.idx < scan.bytes.len() {
        if let Some(next) = scan.next_cmd() {
            cmd = next;
        } else if !scan.has_number_ahead() {
            break;
        }

        match cmd {
            b'M' | b'm' => {
                let Some(dx) = scan.read_number() else { break };
                let Some(dy) = scan.read_number() else { break };
                if !cur_pts.is_empty() {
                    out.push(Subpath {
                        points: std::mem::take(&mut cur_pts),
                    });
                }
                cur = if cmd == b'm' {
                    (cur.0 + dx, cur.1 + dy)
                } else {
                    (dx, dy)
                };
                start = cur;
                cur_pts.push(cur);
                prev_cubic_ctrl = None;
                cmd = if cmd == b'm' { b'l' } else { b'L' };
            }
            b'L' | b'l' => {
                let Some(dx) = scan.read_number() else { break };
                let Some(dy) = scan.read_number() else { break };
                cur = if cmd == b'l' {
                    (cur.0 + dx, cur.1 + dy)
                } else {
                    (dx, dy)
                };
                cur_pts.push(cur);
                prev_cubic_ctrl = None;
            }
            b'H' | b'h' => {
                let Some(dx) = scan.read_number() else { break };
                cur.0 = if cmd == b'h' { cur.0 + dx } else { dx };
                cur_pts.push(cur);
                prev_cubic_ctrl = None;
            }
            b'V' | b'v' => {
                let Some(dy) = scan.read_number() else { break };
                cur.1 = if cmd == b'v' { cur.1 + dy } else { dy };
                cur_pts.push(cur);
                prev_cubic_ctrl = None;
            }
            b'C' | b'c' => {
                let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x), Some(y)) = (
                    scan.read_number(),
                    scan.read_number(),
                    scan.read_number(),
                    scan.read_number(),
                    scan.read_number(),
                    scan.read_number(),
                ) else {
                    break;
                };
                let (c1, c2, p3) = if cmd == b'c' {
                    (
                        (cur.0 + x1, cur.1 + y1),
                        (cur.0 + x2, cur.1 + y2),
                        (cur.0 + x, cur.1 + y),
                    )
                } else {
                    ((x1, y1), (x2, y2), (x, y))
                };
                sample_cubic(&mut cur_pts, cur, c1, c2, p3, 12);
                prev_cubic_ctrl = Some(c2);
                cur = p3;
            }
            b'S' | b's' => {
                let (Some(x2), Some(y2), Some(x), Some(y)) = (
                    scan.read_number(),
                    scan.read_number(),
                    scan.read_number(),
                    scan.read_number(),
                ) else {
                    break;
                };
                let c1 = match prev_cubic_ctrl {
                    Some(prev) => (2.0_f32 * cur.0 - prev.0, 2.0_f32 * cur.1 - prev.1),
                    None => cur,
                };
                let (c2, p3) = if cmd == b's' {
                    ((cur.0 + x2, cur.1 + y2), (cur.0 + x, cur.1 + y))
                } else {
                    ((x2, y2), (x, y))
                };
                sample_cubic(&mut cur_pts, cur, c1, c2, p3, 12);
                prev_cubic_ctrl = Some(c2);
                cur = p3;
            }
            b'A' | b'a' => {
                let (Some(rx), Some(ry), Some(rot), Some(large_arc), Some(sweep), Some(x), Some(y)) = (
                    scan.read_number(),
                    scan.read_number(),
                    scan.read_number(),
                    scan.read_flag(),
                    scan.read_flag(),
                    scan.read_number(),
                    scan.read_number(),
                ) else {
                    break;
                };
                let target = if cmd == b'a' {
                    (cur.0 + x, cur.1 + y)
                } else {
                    (x, y)
                };
                sample_svg_arc(&mut cur_pts, cur, (rx, ry), rot, large_arc, sweep, target);
                cur = target;
                prev_cubic_ctrl = None;
            }
            b'Z' | b'z' => {
                cur = start;
                if !cur_pts.is_empty() {
                    out.push(Subpath {
                        points: std::mem::take(&mut cur_pts),
                    });
                }
                prev_cubic_ctrl = None;
            }
            _ => break,
        }
    }

    if !cur_pts.is_empty() {
        out.push(Subpath { points: cur_pts });
    }
    out
}

fn sample_cubic(
    out: &mut Vec<(f32, f32)>,
    p0: (f32, f32),
    c1: (f32, f32),
    c2: (f32, f32),
    p3: (f32, f32),
    steps: usize,
) {
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let u = 1.0_f32 - t;
        let b0 = u * u * u;
        let b1 = 3.0_f32 * u * u * t;
        let b2 = 3.0_f32 * u * t * t;
        let b3 = t * t * t;
        out.push((
            b0 * p0.0 + b1 * c1.0 + b2 * c2.0 + b3 * p3.0,
            b0 * p0.1 + b1 * c1.1 + b2 * c2.1 + b3 * p3.1,
        ));
    }
}

fn sample_svg_arc(
    out: &mut Vec<(f32, f32)>,
    from: (f32, f32),
    radii: (f32, f32),
    x_axis_rotation_deg: f32,
    large_arc_flag: bool,
    sweep_flag: bool,
    to: (f32, f32),
) {
    let (x1, y1) = from;
    let (x2, y2) = to;
    let mut rx = radii.0.abs();
    let mut ry = radii.1.abs();
    if (x1 - x2).abs() < 1e-5_f32 && (y1 - y2).abs() < 1e-5_f32 {
        return;
    }
    if rx < 1e-5_f32 || ry < 1e-5_f32 {
        out.push(to);
        return;
    }

    let phi = x_axis_rotation_deg.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();
    let dx2 = (x1 - x2) * 0.5_f32;
    let dy2 = (y1 - y2) * 0.5_f32;
    let x1p = cos_phi * dx2 + sin_phi * dy2;
    let y1p = -sin_phi * dx2 + cos_phi * dy2;

    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0_f32 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }

    let rx2 = rx * rx;
    let ry2 = ry * ry;
    let x1p2 = x1p * x1p;
    let y1p2 = y1p * y1p;
    let num = (rx2 * ry2 - rx2 * y1p2 - ry2 * x1p2).max(0.0_f32);
    let den = rx2 * y1p2 + ry2 * x1p2;
    let root = if den > 1e-9_f32 {
        (num / den).sqrt()
    } else {
        0.0_f32
    };
    let sign = if large_arc_flag == sweep_flag {
        -1.0_f32
    } else {
        1.0_f32
    };
    let cxp = sign * root * (rx * y1p / ry);
    let cyp = sign * root * (-ry * x1p / rx);

    let cx = cos_phi * cxp - sin_phi * cyp + (x1 + x2) * 0.5_f32;
    let cy = sin_phi * cxp + cos_phi * cyp + (y1 + y2) * 0.5_f32;

    let vec_angle = |ux: f32, uy: f32, vx: f32, vy: f32| -> f32 {
        (ux * vy - uy * vx).atan2(ux * vx + uy * vy)
    };
    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;

    let theta1 = vec_angle(1.0_f32, 0.0_f32, ux, uy);
    let mut dtheta = vec_angle(ux, uy, vx, vy);
    if !sweep_flag && dtheta > 0.0_f32 {
        dtheta -= std::f32::consts::TAU;
    } else if sweep_flag && dtheta < 0.0_f32 {
        dtheta += std::f32::consts::TAU;
    }

    let steps = ((dtheta.abs() / (std::f32::consts::PI / 10.0_f32)).ceil() as usize).clamp(4, 24);
    for i in 1..=steps {
        let t = theta1 + dtheta * (i as f32 / steps as f32);
        let (st, ct) = t.sin_cos();
        let px = cos_phi * rx * ct - sin_phi * ry * st + cx;
        let py = sin_phi * rx * ct + cos_phi * ry * st + cy;
        out.push((px, py));
    }
}
