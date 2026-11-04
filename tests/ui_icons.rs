#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

use eframe::egui::{self, Color32, LayerId, Order, Pos2, Rect, Shape, Vec2};
use icons::{Icon, paint, paint_brand, paint_spinner};
use theme_palettes::ThemeId;

fn capture_shapes(draw: impl Fn(&egui::Painter)) -> Vec<Shape> {
    let ctx = egui::Context::default();
    let probe_layer = LayerId::new(Order::Foreground, egui::Id::new("icon-probe"));
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(200.0_f32, 200.0_f32),
            )),
            ..Default::default()
        },
        |ctx| {
            let painter = ctx.layer_painter(probe_layer);
            draw(&painter);
        },
    );
    let mut flat = Vec::new();
    for cs in out.shapes {
        flatten_shape(cs.shape, &mut flat);
    }
    flat
}

fn flatten_shape(shape: Shape, out: &mut Vec<Shape>) {
    match shape {
        Shape::Vec(list) => {
            for inner in list {
                flatten_shape(inner, out);
            }
        }
        Shape::Noop => {}
        other => out.push(other),
    }
}

fn shape_uses_color(shape: &Shape, expected: Color32) -> bool {
    match shape {
        Shape::Circle(c) => c.fill == expected || c.stroke.color == expected,
        Shape::Rect(r) => r.fill == expected || r.stroke.color == expected,
        Shape::Path(p) => {
            p.fill == expected || p.stroke.color == egui::epaint::ColorMode::Solid(expected)
        }
        Shape::LineSegment { stroke, .. } => stroke.color == expected,
        _ => false,
    }
}

#[test]
fn all_icons_paint_bounded_vector_shapes_at_10_13_16_24_and_preserve_theme_colors() {
    let theme_colors = [
        ThemeId::Classic.spec().ui.text,
        ThemeId::Dracula.spec().ui.accent,
        ThemeId::Nord.spec().ui.text_dim,
    ];

    for icon in Icon::ALL {
        for &size in &[10.0_f32, 13.0_f32, 16.0_f32, 24.0_f32] {
            for &color in &theme_colors {
                let rect = Rect::from_min_size(Pos2::new(20.0_f32, 30.0_f32), Vec2::splat(size));
                let shapes = capture_shapes(|painter| paint(painter, rect, icon, color));
                assert!(
                    shapes.len() >= 2,
                    "{icon:?} at {size}px should emit >= 2 vector shapes, got {}",
                    shapes.len()
                );
                let padded = rect.expand(0.25_f32);
                for shape in &shapes {
                    assert!(
                        !matches!(shape, Shape::Text(_)),
                        "{icon:?} must never approximate icons with text glyphs"
                    );
                    assert!(
                        shape_uses_color(shape, color),
                        "{icon:?} shape {shape:?} did not propagate theme color {color:?}"
                    );
                    let bounds = shape.visual_bounding_rect();
                    assert!(
                        bounds.min.x.is_finite()
                            && bounds.min.y.is_finite()
                            && bounds.max.x.is_finite()
                            && bounds.max.y.is_finite(),
                        "{icon:?} produced non-finite bounds: {bounds:?}"
                    );
                    assert!(
                        padded.contains_rect(bounds),
                        "{icon:?} at {size}px overflowed rect {rect:?}: shape bounds {bounds:?}"
                    );
                }
            }
        }

        // Non-square rect (`28x16`) must inscribe centered `16x16` square (preserving 1:1 ratio):
        let wide_rect =
            Rect::from_min_size(Pos2::new(10.0_f32, 10.0_f32), Vec2::new(28.0_f32, 16.0_f32));
        let centered_sq =
            Rect::from_center_size(wide_rect.center(), Vec2::splat(16.0_f32)).expand(0.25_f32);
        let wide_shapes = capture_shapes(|p| paint(p, wide_rect, icon, Color32::WHITE));
        assert!(!wide_shapes.is_empty());
        for s in &wide_shapes {
            assert!(
                centered_sq.contains_rect(s.visual_bounding_rect()),
                "{icon:?} failed 1:1 inscription inside {wide_rect:?}: {:?}",
                s.visual_bounding_rect()
            );
        }

        // Zero / negative rects emit zero shapes and never panic:
        let empty = Rect::from_min_size(Pos2::new(5.0_f32, 5.0_f32), Vec2::ZERO);
        assert!(capture_shapes(|p| paint(p, empty, icon, Color32::WHITE)).is_empty());
    }
}

#[test]
fn panel_toggles_and_settings_gear_match_reference_svg_distinctions() {
    let rect = Rect::from_min_size(Pos2::new(16.0_f32, 16.0_f32), Vec2::splat(16.0_f32));
    let color = Color32::from_rgb(0xd4, 0xd7, 0xdd);

    let left_open = capture_shapes(|p| paint(p, rect, Icon::PanelLeftOpen, color));
    let left_closed = capture_shapes(|p| paint(p, rect, Icon::PanelLeftClosed, color));
    let left_filled = left_open
        .iter()
        .find_map(|s| match s {
            Shape::Rect(r) if r.fill == color => Some(r.rect),
            _ => None,
        })
        .expect("PanelLeftOpen must fill left sidebar half");
    assert!(left_filled.center().x < rect.center().x);
    assert!(
        left_closed
            .iter()
            .all(|s| !matches!(s, Shape::Rect(r) if r.fill == color))
    );

    let right_open = capture_shapes(|p| paint(p, rect, Icon::PanelRightOpen, color));
    let right_closed = capture_shapes(|p| paint(p, rect, Icon::PanelRightClosed, color));
    let right_filled = right_open
        .iter()
        .find_map(|s| match s {
            Shape::Rect(r) if r.fill == color => Some(r.rect),
            _ => None,
        })
        .expect("PanelRightOpen must fill right sidebar half");
    assert!(right_filled.center().x > rect.center().x);
    assert!(
        right_closed
            .iter()
            .all(|s| !matches!(s, Shape::Rect(r) if r.fill == color))
    );

    // `Settings` gear (`App.vue:15051`) must parse all 8 teeth + arcs into a closed path:
    let gear_rect = Rect::from_min_size(Pos2::new(10.0_f32, 10.0_f32), Vec2::splat(24.0_f32));
    let gear_shapes = capture_shapes(|p| paint(p, gear_rect, Icon::Settings, color));
    let closed_gear_pts = gear_shapes
        .iter()
        .find_map(|s| match s {
            Shape::Path(path) if path.closed => Some(path.points.len()),
            _ => None,
        })
        .expect("Settings gear outer path must be closed");
    assert!(
        closed_gear_pts > 40,
        "Settings gear should tessellate all 8 teeth/arcs, got {closed_gear_pts} vertices"
    );
}

#[test]
fn brand_logo_paints_exact_six_tiles_and_switches_release_vs_dev_palette() {
    for &size in &[13.0_f32, 16.0_f32, 24.0_f32] {
        let rect = Rect::from_min_size(Pos2::new(12.0_f32, 12.0_f32), Vec2::splat(size));

        let rel_shapes = capture_shapes(|p| paint_brand(p, rect, false));
        assert_eq!(rel_shapes.len(), 6);
        let rel_fills: Vec<Color32> = rel_shapes
            .iter()
            .map(|s| match s {
                Shape::Rect(r) => {
                    assert!(rect.expand(0.1_f32).contains_rect(r.rect));
                    r.fill
                }
                other => panic!("expected rounded rect tile, got {other:?}"),
            })
            .collect();
        assert_eq!(
            rel_fills,
            vec![
                Color32::from_rgb(0x6b, 0x8f, 0xff),
                Color32::from_rgb(0x6b, 0x8f, 0xff),
                Color32::from_rgb(0x6b, 0x8f, 0xff),
                Color32::from_rgb(0x5e, 0x78, 0xd4),
                Color32::from_rgb(0x44, 0x58, 0x9b),
                Color32::from_rgb(0xe6, 0xe6, 0xe6),
            ]
        );

        let dev_shapes = capture_shapes(|p| paint_brand(p, rect, true));
        assert_eq!(dev_shapes.len(), 6);
        let dev_fills: Vec<Color32> = dev_shapes
            .iter()
            .map(|s| match s {
                Shape::Rect(r) => {
                    assert!(rect.expand(0.1_f32).contains_rect(r.rect));
                    r.fill
                }
                other => panic!("expected rounded rect tile, got {other:?}"),
            })
            .collect();
        assert_eq!(
            dev_fills,
            vec![
                Color32::from_rgb(0xf5, 0xc5, 0x42),
                Color32::from_rgb(0xf5, 0xc5, 0x42),
                Color32::from_rgb(0xf5, 0xc5, 0x42),
                Color32::from_rgb(0xc9, 0xa2, 0x3b),
                Color32::from_rgb(0x95, 0x7a, 0x31),
                Color32::from_rgb(0xe6, 0xe6, 0xe6),
            ]
        );
    }

    let empty = Rect::from_min_size(Pos2::ZERO, Vec2::ZERO);
    assert!(capture_shapes(|p| paint_brand(p, empty, false)).is_empty());
    assert!(capture_shapes(|p| paint_spinner(p, empty, Color32::WHITE, 0.0_f32)).is_empty());

    let spin_rect = Rect::from_min_size(Pos2::new(8.0_f32, 8.0_f32), Vec2::splat(16.0_f32));
    let spin_shapes = capture_shapes(|p| paint_spinner(p, spin_rect, Color32::WHITE, 1.2_f32));
    assert_eq!(spin_shapes.len(), 1);
    assert!(
        spin_rect
            .expand(0.25_f32)
            .contains_rect(spin_shapes[0].visual_bounding_rect())
    );
}
