//! Pure-Rust vector icon painter for Tessel's native `egui` chrome and modals.
//!
//! Geometry sources (zero runtime I/O, zero font/glyph approximations):
//! - Priority 1: Tessel's inline toolbar SVGs in `App.vue` (`14732..15058`): 6-tile brand mark,
//!   `Search` (`16x16`), `Plus` (`16x16`), `ChevronDown` (`16x16`), `Share` (`16x16`),
//!   `Layout` (`16x16`), `PanelLeftOpen`/`Closed` (`16x16`), `PanelRightOpen`/`Closed` (`16x16`),
//!   and `Settings` gear (`24x24`).
//! - Priority 2: Lucide (`lucide-vue-next` `v0.577.0`, ISC / Feather MIT — see
//!   `THIRD_PARTY_NOTICES.md`) for `Close` (`x`), `Help` (`circle-question-mark`), and
//!   `Terminal` (`square-terminal`).

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Search,
    Plus,
    ChevronDown,
    Close,
    Help,
    Settings,
    Share,
    Layout,
    Terminal,
    PanelLeftOpen,
    PanelLeftClosed,
    PanelRightOpen,
    PanelRightClosed,
    PaneMore,
    PaneMaximize,
    PaneRestore,
    PaneClose,
}

impl Icon {
    pub const ALL: [Self; 17] = [
        Self::Search,
        Self::Plus,
        Self::ChevronDown,
        Self::Close,
        Self::Help,
        Self::Settings,
        Self::Share,
        Self::Layout,
        Self::Terminal,
        Self::PanelLeftOpen,
        Self::PanelLeftClosed,
        Self::PanelRightOpen,
        Self::PanelRightClosed,
        Self::PaneMore,
        Self::PaneMaximize,
        Self::PaneRestore,
        Self::PaneClose,
    ];
}

#[derive(Clone, Copy)]
struct Viewport {
    square: Rect,
    scale: f32,
}

impl Viewport {
    fn new(rect: Rect, viewbox_size: f32) -> Option<Self> {
        if !rect.is_positive() || !rect.min.x.is_finite() || !rect.min.y.is_finite() {
            return None;
        }
        let side = rect.width().min(rect.height());
        if side <= 0.0_f32 || !side.is_finite() {
            return None;
        }
        let square = Rect::from_center_size(rect.center(), Vec2::splat(side));
        Some(Self {
            square,
            scale: side / viewbox_size,
        })
    }

    fn pt(self, x: f32, y: f32) -> Pos2 {
        Pos2::new(
            self.square.min.x + x * self.scale,
            self.square.min.y + y * self.scale,
        )
    }

    fn rect_xywh(self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.pt(x, y), Vec2::new(w * self.scale, h * self.scale))
    }

    fn len(self, v: f32) -> f32 {
        v * self.scale
    }
}

/// Paints Tessel's exact 6-tile brand mark (`App.vue:14734-14748`, `viewBox="0 0 24 24"`).
///
/// - Release (`development == false`): `#6b8fff` top bar, `#5e78d4` mid stem, `#44589b` bottom
///   stem, `#e6e6e6` cursor tile.
/// - Development (`development == true`): `#f5c542` top bar, `#c9a23b` mid stem, `#957a31`
///   bottom stem, `#e6e6e6` cursor tile.
pub fn paint_brand(painter: &Painter, rect: Rect, development: bool) {
    let Some(vp) = Viewport::new(rect, 24.0_f32) else {
        return;
    };
    let (top, mid, bot) = if development {
        (
            Color32::from_rgb(0xf5, 0xc5, 0x42),
            Color32::from_rgb(0xc9, 0xa2, 0x3b),
            Color32::from_rgb(0x95, 0x7a, 0x31),
        )
    } else {
        (
            Color32::from_rgb(0x6b, 0x8f, 0xff),
            Color32::from_rgb(0x5e, 0x78, 0xd4),
            Color32::from_rgb(0x44, 0x58, 0x9b),
        )
    };
    let cursor = Color32::from_rgb(0xe6, 0xe6, 0xe6);
    let rx = vp.len(1.2_f32);

    let tiles = [
        (2.0_f32, 2.5_f32, top),
        (9.25_f32, 2.5_f32, top),
        (16.5_f32, 2.5_f32, top),
        (9.25_f32, 9.25_f32, mid),
        (9.25_f32, 16.0_f32, bot),
        (16.5_f32, 16.0_f32, cursor),
    ];
    for (x, y, fill) in tiles {
        painter.rect_filled(vp.rect_xywh(x, y, 5.5_f32, 5.5_f32), rx, fill);
    }
}

/// Paints a crisp vector [`Icon`] inscribed 1:1 inside `rect` with `color`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    match icon {
        // Exact inline 16x16 TerminalPane.vue header controls.
        Icon::PaneMore | Icon::PaneMaximize | Icon::PaneRestore | Icon::PaneClose => {
            let Some(vp) = Viewport::new(rect, 16.0) else {
                return;
            };
            if icon == Icon::PaneMore {
                for x in [3.0, 8.0, 13.0] {
                    painter.circle_filled(vp.pt(x, 8.0), vp.len(1.4), color);
                }
            } else {
                let (path, width) = match icon {
                    Icon::PaneMaximize => ("M2 6V2h4M10 2h4v4M14 10v4h-4M6 14H2v-4", 1.5),
                    Icon::PaneRestore => ("M6 2v4H2M14 6h-4V2M10 14v-4h4M2 10h4v4", 1.5),
                    _ => ("M3.5 3.5l9 9M12.5 3.5l-9 9", 1.6),
                };
                paint_svg_path(painter, vp, path, Stroke::new(vp.len(width), color), true);
            }
        }
        Icon::Search => {
            let Some(vp) = Viewport::new(rect, 16.0_f32) else {
                return;
            };
            let stroke = Stroke::new(vp.len(1.4_f32), color);
            painter.circle_stroke(vp.pt(7.0_f32, 7.0_f32), vp.len(4.6_f32), stroke);
            paint_svg_path(painter, vp, "M10.4 10.4L14 14", stroke, true);
        }
        Icon::Plus => {
            let Some(vp) = Viewport::new(rect, 16.0_f32) else {
                return;
            };
            let stroke = Stroke::new(vp.len(1.5_f32), color);
            paint_svg_path(painter, vp, "M8 3.2v9.6M3.2 8h9.6", stroke, true);
        }
        Icon::ChevronDown => {
            let Some(vp) = Viewport::new(rect, 16.0_f32) else {
                return;
            };
            let stroke = Stroke::new(vp.len(1.8_f32), color);
            paint_svg_path(painter, vp, "M4 6l4 4 4-4", stroke, true);
        }
        Icon::Share => {
            let Some(vp) = Viewport::new(rect, 16.0_f32) else {
                return;
            };
            let stroke = Stroke::new(vp.len(1.3_f32), color);
            painter.circle_stroke(vp.pt(6.0_f32, 5.2_f32), vp.len(2.3_f32), stroke);
            paint_svg_path(
                painter,
                vp,
                "M1.8 13.2c.5-2.3 2.2-3.6 4.2-3.6s3.7 1.3 4.2 3.6",
                stroke,
                true,
            );
            paint_svg_path(
                painter,
                vp,
                "M10.6 3.2a2.3 2.3 0 010 4.2M12 9.9c1.1.5 1.9 1.6 2.2 3.3",
                stroke,
                true,
            );
        }
        Icon::Layout => {
            let Some(vp) = Viewport::new(rect, 16.0_f32) else {
                return;
            };
            let stroke = Stroke::new(vp.len(1.3_f32), color);
            painter.rect_stroke(
                vp.rect_xywh(1.8_f32, 2.3_f32, 12.4_f32, 11.4_f32),
                vp.len(2.0_f32),
                stroke,
                StrokeKind::Middle,
            );
            paint_svg_path(painter, vp, "M8 2.3v11.4M8 8h6.2", stroke, false);
        }
        Icon::PanelLeftOpen | Icon::PanelLeftClosed => {
            let Some(vp) = Viewport::new(rect, 16.0_f32) else {
                return;
            };
            let stroke = Stroke::new(vp.len(1.3_f32), color);
            painter.rect_stroke(
                vp.rect_xywh(1.75_f32, 2.25_f32, 12.5_f32, 11.5_f32),
                vp.len(2.25_f32),
                stroke,
                StrokeKind::Middle,
            );
            if icon == Icon::PanelLeftOpen {
                painter.rect_filled(
                    vp.rect_xywh(2.4_f32, 2.9_f32, 4.1_f32, 10.2_f32),
                    vp.len(1.4_f32),
                    color,
                );
            } else {
                paint_svg_path(painter, vp, "M6.5 2.5v11", stroke, false);
            }
        }
        Icon::PanelRightOpen | Icon::PanelRightClosed => {
            let Some(vp) = Viewport::new(rect, 16.0_f32) else {
                return;
            };
            let stroke = Stroke::new(vp.len(1.3_f32), color);
            painter.rect_stroke(
                vp.rect_xywh(1.75_f32, 2.25_f32, 12.5_f32, 11.5_f32),
                vp.len(2.25_f32),
                stroke,
                StrokeKind::Middle,
            );
            if icon == Icon::PanelRightOpen {
                painter.rect_filled(
                    vp.rect_xywh(9.5_f32, 2.9_f32, 4.1_f32, 10.2_f32),
                    vp.len(1.4_f32),
                    color,
                );
            } else {
                paint_svg_path(painter, vp, "M9.5 2.5v11", stroke, false);
            }
        }
        Icon::Settings => {
            let Some(vp) = Viewport::new(rect, 24.0_f32) else {
                return;
            };
            let stroke = Stroke::new(vp.len(1.6_f32), color);
            painter.circle_stroke(vp.pt(12.0_f32, 12.0_f32), vp.len(3.0_f32), stroke);
            paint_svg_path(
                painter,
                vp,
                "M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 11-2.83 2.83l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 11-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 11-2.83-2.83l.06-.06a1.65 1.65 0 00.33-1.82 1.65 1.65 0 00-1.51-1H3a2 2 0 110-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 112.83-2.83l.06.06a1.65 1.65 0 001.82.33H9a1.65 1.65 0 001-1.51V3a2 2 0 114 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 112.83 2.83l-.06.06a1.65 1.65 0 00-.33 1.82V9a1.65 1.65 0 001.51 1H21a2 2 0 110 4h-.09a1.65 1.65 0 00-1.51 1z",
                stroke,
                true,
            );
        }
        Icon::Close => {
            let Some(vp) = Viewport::new(rect, 24.0_f32) else {
                return;
            };
            let stroke = Stroke::new(vp.len(2.0_f32), color);
            paint_svg_path(painter, vp, "M18 6 6 18M6 6l12 12", stroke, true);
        }
        Icon::Help => {
            let Some(vp) = Viewport::new(rect, 24.0_f32) else {
                return;
            };
            let stroke = Stroke::new(vp.len(2.0_f32), color);
            painter.circle_stroke(vp.pt(12.0_f32, 12.0_f32), vp.len(10.0_f32), stroke);
            paint_svg_path(
                painter,
                vp,
                "M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3",
                stroke,
                true,
            );
            painter.circle_filled(vp.pt(12.0_f32, 17.0_f32), vp.len(1.05_f32), color);
        }
        Icon::Terminal => {
            let Some(vp) = Viewport::new(rect, 24.0_f32) else {
                return;
            };
            let stroke = Stroke::new(vp.len(2.0_f32), color);
            painter.rect_stroke(
                vp.rect_xywh(3.0_f32, 3.0_f32, 18.0_f32, 18.0_f32),
                vp.len(2.0_f32),
                stroke,
                StrokeKind::Middle,
            );
            paint_svg_path(painter, vp, "m7 11 2-2-2-2M11 13h4", stroke, true);
        }
    }
}

// --- Pure-Rust SVG path (`M/L/H/V/C/S/A/Z`) parser & tessellator --------------------

struct Subpath {
    points: Vec<(f32, f32)>,
    closed: bool,
}

fn paint_svg_path(painter: &Painter, vp: Viewport, d: &str, stroke: Stroke, round_caps: bool) {
    let cap_r = stroke.width * 0.5_f32;
    for sub in parse_svg_subpaths(d) {
        if sub.points.is_empty() {
            continue;
        }
        let mut pts: Vec<Pos2> = Vec::with_capacity(sub.points.len());
        for (x, y) in sub.points {
            let p = vp.pt(x, y);
            if pts.last().is_none_or(|prev| prev.distance_sq(p) > 1e-4_f32) {
                pts.push(p);
            }
        }
        if sub.closed && pts.len() >= 3 {
            if pts
                .first()
                .is_some_and(|f| pts.last().is_some_and(|l| f.distance_sq(*l) <= 1e-3_f32))
            {
                pts.pop();
            }
            if pts.len() >= 3 {
                painter.add(Shape::Path(egui::epaint::PathShape::closed_line(
                    pts, stroke,
                )));
            }
        } else if pts.len() >= 2 {
            let first = pts[0];
            let last = pts[pts.len() - 1];
            painter.add(Shape::Path(egui::epaint::PathShape::line(pts, stroke)));
            if round_caps && cap_r >= 0.4_f32 {
                painter.circle_filled(first, cap_r, stroke.color);
                painter.circle_filled(last, cap_r, stroke.color);
            }
        } else if pts.len() == 1 && round_caps && cap_r >= 0.4_f32 {
            painter.circle_filled(pts[0], cap_r, stroke.color);
        }
    }
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
                        closed: false,
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
                        closed: true,
                    });
                }
                prev_cubic_ctrl = None;
            }
            _ => break,
        }
    }

    if !cur_pts.is_empty() {
        out.push(Subpath {
            points: cur_pts,
            closed: false,
        });
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
