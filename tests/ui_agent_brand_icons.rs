//! Standalone headless `egui` integration tests for `src/agent_brand_icons.rs`
//! (`BrandIcon.vue` + `brandSvgs.js` + Orca asset parity across all 42 `agent_catalog::ALL` presets).

#[allow(dead_code)]
#[path = "../src/agent_catalog.rs"]
mod agent_catalog;

#[path = "../src/agent_brand_icons.rs"]
mod agent_brand_icons;

use std::collections::BTreeSet;

use eframe::egui::{
    self, Color32, Context, Pos2, RawInput, Rect, Shape, Vec2, epaint::ClippedShape,
};

fn capture_paint(rect: Rect, mut draw: impl FnMut(&egui::Painter)) -> Vec<ClippedShape> {
    let ctx = Context::default();
    let mut shapes = Vec::new();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(512.0, 512.0))),
        ..Default::default()
    };
    let output = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let before = ui.painter().clip_rect();
            let _ = before;
            draw(ui.painter());
        });
    });
    // Filter out the `CentralPanel` background rect (`512×512`) so we inspect only icon primitives.
    for clipped in output.shapes {
        let is_panel_bg = matches!(
            &clipped.shape,
            Shape::Rect(r) if r.rect.width() >= 500.0 && r.rect.height() >= 500.0
        );
        if !is_panel_bg {
            shapes.push(clipped);
        }
    }
    let _ = rect;
    shapes
}

fn collect_shape_bounds(shape: &Shape, out: &mut Rect, count: &mut usize) {
    match shape {
        Shape::Noop => {}
        Shape::Vec(inner) => {
            for s in inner {
                collect_shape_bounds(s, out, count);
            }
        }
        Shape::Circle(c) => {
            let r = c.radius + c.stroke.width * 0.5;
            *out = out.union(Rect::from_center_size(c.center, Vec2::splat(r * 2.0)));
            *count += 1;
        }
        Shape::Ellipse(e) => {
            let r = e.radius + Vec2::splat(e.stroke.width * 0.5);
            *out = out.union(Rect::from_center_size(e.center, r * 2.0));
            *count += 1;
        }
        Shape::LineSegment { points, stroke } => {
            let pad = stroke.width * 0.5;
            for p in points {
                *out = out.union(Rect::from_center_size(*p, Vec2::splat(pad * 2.0)));
            }
            *count += 1;
        }
        Shape::Path(p) => {
            let pad = p.stroke.width * 0.5;
            for pt in &p.points {
                *out = out.union(Rect::from_center_size(*pt, Vec2::splat(pad * 2.0)));
            }
            *count += 1;
        }
        Shape::Rect(r) => {
            *out = out.union(r.rect.expand(r.stroke.width * 0.5));
            *count += 1;
        }
        Shape::Text(t) => {
            let g_rect = t.galley.rect.translate(t.pos.to_vec2());
            *out = out.union(g_rect);
            *count += 1;
        }
        Shape::Mesh(m) => {
            for v in &m.vertices {
                *out = out.union(Rect::from_min_max(v.pos, v.pos));
            }
            *count += 1;
        }
        _ => {
            *out = out.union(shape.visual_bounding_rect());
            *count += 1;
        }
    }
}

fn contains_text_shape(shapes: &[ClippedShape]) -> bool {
    fn check(s: &Shape) -> bool {
        match s {
            Shape::Text(_) => true,
            Shape::Vec(v) => v.iter().any(check),
            _ => false,
        }
    }
    shapes.iter().any(|cs| check(&cs.shape))
}

fn max_channel_and_alpha(shapes: &[ClippedShape]) -> (u8, u8) {
    fn visit(s: &Shape, max_rgb: &mut u8, max_a: &mut u8) {
        let mut record = |c: Color32| {
            if c != Color32::TRANSPARENT {
                *max_rgb = (*max_rgb).max(c.r().max(c.g()).max(c.b()));
                *max_a = (*max_a).max(c.a());
            }
        };
        match s {
            Shape::Vec(v) => {
                for item in v {
                    visit(item, max_rgb, max_a);
                }
            }
            Shape::Circle(c) => {
                record(c.fill);
                record(c.stroke.color);
            }
            Shape::LineSegment { stroke, .. } => record(stroke.color),
            Shape::Path(p) => {
                record(p.fill);
                if let egui::epaint::ColorMode::Solid(sc) = p.stroke.color {
                    record(sc);
                }
            }
            Shape::Rect(r) => {
                record(r.fill);
                record(r.stroke.color);
            }
            Shape::Text(t) => {
                record(t.override_text_color.unwrap_or(t.fallback_color));
            }
            Shape::Mesh(m) => {
                for v in &m.vertices {
                    record(v.color);
                }
            }
            _ => {}
        }
    }
    let mut rgb = 0;
    let mut a = 0;
    for cs in shapes {
        visit(&cs.shape, &mut rgb, &mut a);
    }
    (rgb, a)
}

fn point_in_triangle(p: Pos2, a: Pos2, b: Pos2, c: Pos2) -> bool {
    let sign = |p1: Pos2, p2: Pos2, p3: Pos2| -> f32 {
        (p1.x - p3.x) * (p2.y - p3.y) - (p2.x - p3.x) * (p1.y - p3.y)
    };
    let d1 = sign(p, a, b);
    let d2 = sign(p, b, c);
    let d3 = sign(p, c, a);
    let has_neg = (d1 < -1e-4) || (d2 < -1e-4) || (d3 < -1e-4);
    let has_pos = (d1 > 1e-4) || (d2 > 1e-4) || (d3 > 1e-4);
    !(has_neg && has_pos)
}

fn point_covered(shapes: &[ClippedShape], p: Pos2) -> bool {
    fn hit(shape: &Shape, p: Pos2) -> bool {
        match shape {
            Shape::Vec(v) => v.iter().any(|s| hit(s, p)),
            Shape::Rect(r) => r.fill != Color32::TRANSPARENT && r.rect.contains(p),
            Shape::Circle(c) => c.fill != Color32::TRANSPARENT && c.center.distance(p) <= c.radius,
            Shape::Path(path) => {
                if path.fill == Color32::TRANSPARENT || path.points.len() < 3 {
                    return false;
                }
                let a = path.points[0];
                for pair in path.points[1..].windows(2) {
                    if point_in_triangle(p, a, pair[0], pair[1]) {
                        return true;
                    }
                }
                false
            }
            Shape::Mesh(m) => {
                for &[i0, i1, i2] in m.indices.as_chunks::<3>().0 {
                    let a = m.vertices[i0 as usize].pos;
                    let b = m.vertices[i1 as usize].pos;
                    let c = m.vertices[i2 as usize].pos;
                    if point_in_triangle(p, a, b, c) {
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }
    shapes
        .iter()
        .any(|cs| cs.clip_rect.contains(p) && hit(&cs.shape, p))
}

fn map_svg_to_screen(
    rect: Rect,
    box_xywh: (f32, f32, f32, f32),
    target_units: f32,
    x: f32,
    y: f32,
) -> Pos2 {
    let side = rect.width().min(rect.height());
    let square = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let scale24 = side / 24.0_f32;
    let (bx, by, bw, bh) = box_xywh;
    let frame_xy = (24.0_f32 - target_units) * 0.5_f32;
    let inner_scale = (target_units / bw).min(target_units / bh);
    let ox = frame_xy + (target_units - bw * inner_scale) * 0.5_f32;
    let oy = frame_xy + (target_units - bh * inner_scale) * 0.5_f32;
    let vx = ox + (x - bx) * inner_scale;
    let vy = oy + (y - by) * inner_scale;
    Pos2::new(square.min.x + vx * scale24, square.min.y + vy * scale24)
}

#[test]
fn all_42_catalog_presets_render_finite_bounded_shapes_across_sizes() {
    assert_eq!(agent_catalog::ALL.len(), 42);

    let test_rects = [
        Rect::from_min_size(Pos2::new(12.0, 12.0), Vec2::splat(16.0)),
        Rect::from_min_size(Pos2::new(12.0, 12.0), Vec2::splat(20.0)),
        Rect::from_min_size(Pos2::new(12.0, 12.0), Vec2::splat(24.0)),
        Rect::from_min_size(Pos2::new(20.0, 14.0), Vec2::new(34.0, 18.0)),
        Rect::from_min_size(Pos2::new(18.0, 22.0), Vec2::new(18.0, 32.0)),
    ];

    for preset in agent_catalog::ALL {
        let parsed_accent = agent_brand_icons::parse_hex_color(preset.accent);
        assert!(
            parsed_accent.is_some(),
            "preset `{}` has invalid hex accent `{}`",
            preset.id,
            preset.accent
        );

        for rect in test_rects {
            let shapes = capture_paint(rect, |painter| {
                agent_brand_icons::paint_preset(painter, rect, preset, false);
            });
            assert!(
                !shapes.is_empty(),
                "preset `{}` produced zero shapes in {:?}",
                preset.id,
                rect
            );

            let mut bounds = Rect::NOTHING;
            let mut prim_count = 0usize;
            for cs in &shapes {
                collect_shape_bounds(&cs.shape, &mut bounds, &mut prim_count);
            }
            assert!(prim_count > 0);
            assert!(
                bounds.min.x.is_finite()
                    && bounds.min.y.is_finite()
                    && bounds.max.x.is_finite()
                    && bounds.max.y.is_finite(),
                "preset `{}` produced non-finite bounds {:?}",
                preset.id,
                bounds
            );

            let side = rect.width().min(rect.height());
            let inscribed = Rect::from_center_size(rect.center(), Vec2::splat(side)).expand(1.5);
            assert!(
                inscribed.contains_rect(bounds),
                "preset `{}` bounds {:?} escaped inscribed square {:?} (outer {:?})",
                preset.id,
                bounds,
                inscribed,
                rect
            );
        }
    }
}

#[test]
fn dedicated_vector_brands_aliases_and_shells_emit_pure_geometry_without_text() {
    let vector_ids = [
        "claude",
        "codex",
        "openai",
        "opencode",
        "opencode2",
        "gemini",
        "copilot",
        "aider",
        "amp",
        "kimi",
        "qwen",
        "grok",
        "cline",
        "cursor",
        "cursor-agent",
        "omp",
        "pi",
        "kilo",
        "powershell",
        "pwsh",
        "cmd",
        "gitbash",
        "bash",
        "git",
        "wsl",
    ];
    let rect = Rect::from_min_size(Pos2::new(24.0, 24.0), Vec2::splat(20.0));

    for id in vector_ids {
        assert!(
            agent_brand_icons::has_vector_brand(id),
            "expected `{id}` to have dedicated vector brand"
        );
        let shapes = capture_paint(rect, |painter| {
            agent_brand_icons::paint(painter, rect, id, "Ignored Label", Color32::RED, false);
        });
        assert!(!shapes.is_empty(), "`{id}` emitted zero shapes");
        assert!(
            !contains_text_shape(&shapes),
            "`{id}` unexpectedly fell back to text initial badge"
        );
    }

    // Alias parity: `"cursor-agent"` (and mixed-case padded) matches `"cursor"`, `"openai"` matches `"codex"`.
    let cursor_shapes = capture_paint(rect, |p| {
        agent_brand_icons::paint(p, rect, "cursor", "Cursor CLI", Color32::WHITE, false);
    });
    let cursor_agent_shapes = capture_paint(rect, |p| {
        agent_brand_icons::paint(
            p,
            rect,
            "  Cursor-Agent  ",
            "Cursor CLI",
            Color32::WHITE,
            false,
        );
    });
    assert_eq!(cursor_shapes.len(), cursor_agent_shapes.len());

    let codex_shapes = capture_paint(rect, |p| {
        agent_brand_icons::paint(p, rect, "codex", "Codex CLI", Color32::WHITE, false);
    });
    let openai_shapes = capture_paint(rect, |p| {
        agent_brand_icons::paint(p, rect, "OpenAI", "OpenAI", Color32::WHITE, false);
    });
    assert_eq!(codex_shapes.len(), openai_shapes.len());

    // Multi-stop linear gradients on `gemini` and `omp` produce >= 3 distinct vertex colors.
    for grad_id in ["gemini", "omp"] {
        let shapes = capture_paint(rect, |p| {
            agent_brand_icons::paint(p, rect, grad_id, grad_id, Color32::WHITE, false);
        });
        let mut distinct_colors = BTreeSet::new();
        for cs in &shapes {
            if let Shape::Mesh(m) = &cs.shape {
                for v in &m.vertices {
                    distinct_colors.insert((v.color.r(), v.color.g(), v.color.b()));
                }
            }
        }
        assert!(
            distinct_colors.len() >= 3,
            "`{grad_id}` gradient mesh should contain multiple interpolated colors, got {:?}",
            distinct_colors
        );
    }
}

#[test]
fn evenodd_svg_marks_preserve_holes_and_islands_in_holes() {
    let rect = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::splat(240.0));

    // 1. `codex` (`OPENAI_PATH`): hexagonal center `(12, 12)` is a hole; spoke `(12, 8.45)` is solid.
    let codex = capture_paint(rect, |p| {
        agent_brand_icons::paint(p, rect, "codex", "Codex CLI", Color32::WHITE, false);
    });
    let codex_box = (0.16, 0.0, 23.68, 24.0);
    assert!(
        !point_covered(&codex, map_svg_to_screen(rect, codex_box, 20.0, 12.0, 12.0)),
        "`codex` center hexagon must remain an open hole"
    );
    assert!(
        point_covered(&codex, map_svg_to_screen(rect, codex_box, 20.0, 12.0, 8.45)),
        "`codex` inner spoke around the center hexagon must be filled"
    );

    // 2. `copilot`: visor interior `(12, 15)` is a hole, while left/right eye pills `(9.48, 15.5)`
    //    and `(14.48, 15.5)` inside that visor hole are filled islands!
    let copilot = capture_paint(rect, |p| {
        agent_brand_icons::paint(p, rect, "copilot", "Copilot", Color32::WHITE, false);
    });
    let copilot_box = (0.0, 2.5, 24.0, 19.0);
    assert!(
        !point_covered(
            &copilot,
            map_svg_to_screen(rect, copilot_box, 20.0, 12.0, 15.0)
        ),
        "`copilot` visor interior between the eyes must remain an open hole"
    );
    assert!(
        point_covered(
            &copilot,
            map_svg_to_screen(rect, copilot_box, 20.0, 9.48, 15.5)
        ),
        "`copilot` left eye island inside visor hole must be filled"
    );
    assert!(
        point_covered(
            &copilot,
            map_svg_to_screen(rect, copilot_box, 20.0, 14.48, 15.5)
        ),
        "`copilot` right eye island inside visor hole must be filled"
    );
    assert!(
        point_covered(
            &copilot,
            map_svg_to_screen(rect, copilot_box, 20.0, 12.0, 4.0)
        ),
        "`copilot` upper dome must be filled"
    );

    // 3. `aider`: inner counter-hole of `'A'` `(200, 288)` is open; top bar `(200, 60)` is filled.
    let aider = capture_paint(rect, |p| {
        agent_brand_icons::paint(p, rect, "aider", "Aider", Color32::WHITE, false);
    });
    let aider_box = (85.02, 35.24, 280.86, 360.79);
    assert!(
        !point_covered(
            &aider,
            map_svg_to_screen(rect, aider_box, 20.0, 200.0, 288.0)
        ),
        "`aider` inner counter-hole must remain open"
    );
    assert!(
        point_covered(
            &aider,
            map_svg_to_screen(rect, aider_box, 20.0, 200.0, 60.0)
        ),
        "`aider` top bar must be filled"
    );

    // 4. `cline`: hollow face cutout `(45, 56)` between the eyes is open; eyes & frame are filled.
    let cline = capture_paint(rect, |p| {
        agent_brand_icons::paint(p, rect, "cline", "Cline", Color32::WHITE, false);
    });
    let cline_box = (0.0, 0.48, 91.36, 95.52);
    assert!(
        !point_covered(&cline, map_svg_to_screen(rect, cline_box, 20.0, 45.0, 56.0)),
        "`cline` face cutout between the two eye pills must remain open"
    );
    assert!(
        point_covered(&cline, map_svg_to_screen(rect, cline_box, 20.0, 33.5, 56.0)),
        "`cline` left eye pill must be filled"
    );
    assert!(
        point_covered(&cline, map_svg_to_screen(rect, cline_box, 20.0, 45.0, 19.5)),
        "`cline` top frame bar must be filled"
    );

    // 5. `pi`: inner square `[282.65..400, 282.65..400]` is an open hole; bar & dot are filled.
    let pi = capture_paint(rect, |p| {
        agent_brand_icons::paint(p, rect, "pi", "Pi", Color32::WHITE, false);
    });
    let pi_box = (164.67, 165.33, 470.67, 469.33);
    assert!(
        !point_covered(&pi, map_svg_to_screen(rect, pi_box, 18.5, 341.3, 341.3)),
        "`pi` inner square cutout must remain an open hole"
    );
    assert!(
        point_covered(&pi, map_svg_to_screen(rect, pi_box, 18.5, 341.3, 220.0)),
        "`pi` top bar must be filled"
    );
    assert!(
        point_covered(&pi, map_svg_to_screen(rect, pi_box, 18.5, 576.0, 517.0)),
        "`pi` detached lower-right square must be filled"
    );

    // 6. `qwen`: horizontal inner cutout `(80, 45)` is open; upper solid band `(80, 28)` is filled.
    let qwen = capture_paint(rect, |p| {
        agent_brand_icons::paint(p, rect, "qwen", "Qwen Code", Color32::WHITE, false);
    });
    let qwen_box = (0.0, 0.0, 141.38, 139.97);
    assert!(
        !point_covered(&qwen, map_svg_to_screen(rect, qwen_box, 20.0, 80.0, 45.0)),
        "`qwen` inner cutout must remain open"
    );
    assert!(
        point_covered(&qwen, map_svg_to_screen(rect, qwen_box, 20.0, 80.0, 28.0)),
        "`qwen` solid ribbon must be filled"
    );
}

#[test]
fn initial_tile_fallback_luminance_contrast_and_terminal_fallback_match_brand_icon_vue() {
    assert_eq!(
        agent_brand_icons::parse_hex_color("#fff"),
        Some(Color32::from_rgb(0xff, 0xff, 0xff))
    );
    assert_eq!(
        agent_brand_icons::parse_hex_color("14161b"),
        Some(Color32::from_rgb(0x14, 0x16, 0x1b))
    );
    assert_eq!(agent_brand_icons::parse_hex_color("#12"), None);
    assert_eq!(agent_brand_icons::parse_hex_color("#zzzzzz"), None);

    let dark_ink = Color32::from_rgb(0x14, 0x16, 0x1b);
    // Bright accents (`lum > 150.0`) use `#14161b`.
    assert_eq!(
        agent_brand_icons::badge_text_color(Color32::from_rgb(0xf8, 0xf6, 0x75)),
        dark_ink
    );
    assert_eq!(
        agent_brand_icons::badge_text_color(Color32::from_rgb(0xf5, 0xf5, 0xf5)),
        dark_ink
    );
    // Dark accents (`lum <= 150.0`) use `#ffffff`.
    assert_eq!(
        agent_brand_icons::badge_text_color(Color32::from_rgb(0x18, 0x68, 0xdb)),
        Color32::WHITE
    );
    assert_eq!(
        agent_brand_icons::badge_text_color(Color32::from_rgb(0x90, 0x46, 0xff)),
        Color32::WHITE
    );

    let rect = Rect::from_min_size(Pos2::new(16.0, 16.0), Vec2::splat(24.0));
    assert!(!agent_brand_icons::has_vector_brand("antigravity"));
    assert!(!agent_brand_icons::has_vector_brand("custom-agent"));

    // Custom Unicode agent with bright accent renders rounded tile + `"É"` in `#14161b`.
    let bright_accent = Color32::from_rgb(0xf8, 0xf6, 0x75);
    let custom_shapes = capture_paint(rect, |p| {
        agent_brand_icons::paint(p, rect, "custom-agent", "étoile", bright_accent, false);
    });
    let mut saw_tile = false;
    let mut saw_initial = false;
    for cs in &custom_shapes {
        match &cs.shape {
            Shape::Rect(r) if r.fill == bright_accent => {
                assert!((r.rect.width() - 18.5).abs() < 0.25);
                saw_tile = true;
            }
            Shape::Text(t) => {
                assert_eq!(t.galley.text(), "É");
                let text_col = t.override_text_color.unwrap_or(t.fallback_color);
                assert_eq!(text_col, dark_ink);
                saw_initial = true;
            }
            _ => {}
        }
    }
    assert!(saw_tile && saw_initial);

    // Empty `id` + empty `label` + `TRANSPARENT` accent renders generic vector terminal glyph.
    let term_shapes = capture_paint(rect, |p| {
        agent_brand_icons::paint(p, rect, "", "   ", Color32::TRANSPARENT, false);
    });
    assert!(!term_shapes.is_empty());
    assert!(!contains_text_shape(&term_shapes));
}

#[test]
fn dimmed_mode_lowers_intensity_and_degenerate_rects_never_panic() {
    let rect = Rect::from_min_size(Pos2::new(16.0, 16.0), Vec2::splat(20.0));

    for (id, label, accent) in [
        ("claude", "Claude Code", Color32::from_rgb(0xd9, 0x77, 0x57)),
        ("codex", "Codex CLI", Color32::from_rgb(0x10, 0xa3, 0x7f)),
        ("gemini", "Gemini CLI", Color32::from_rgb(0x42, 0x85, 0xf4)),
        ("omp", "OMP", Color32::from_rgb(0xc0, 0x4f, 0xd8)),
        (
            "antigravity",
            "Antigravity",
            Color32::from_rgb(0x5b, 0x8d, 0xef),
        ),
    ] {
        let normal = capture_paint(rect, |p| {
            agent_brand_icons::paint(p, rect, id, label, accent, false);
        });
        let dimmed = capture_paint(rect, |p| {
            agent_brand_icons::paint(p, rect, id, label, accent, true);
        });
        let (norm_rgb, norm_a) = max_channel_and_alpha(&normal);
        let (dim_rgb, dim_a) = max_channel_and_alpha(&dimmed);
        assert!(
            dim_a < norm_a || dim_rgb < norm_rgb,
            "`{id}` dimmed icon ({dim_rgb}, {dim_a}) did not reduce intensity vs normal ({norm_rgb}, {norm_a})"
        );
    }

    let degenerate_rects = [
        Rect::NOTHING,
        Rect::ZERO,
        Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::splat(0.2)),
        Rect::from_min_max(Pos2::new(20.0, 20.0), Pos2::new(10.0, 10.0)),
        Rect::from_min_size(Pos2::new(f32::NAN, 0.0), Vec2::splat(20.0)),
        Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::splat(f32::INFINITY)),
    ];
    for bad in degenerate_rects {
        let shapes = capture_paint(bad, |p| {
            agent_brand_icons::paint(p, bad, "codex", "Codex", Color32::WHITE, false);
        });
        assert!(
            shapes.is_empty(),
            "degenerate rect {bad:?} should emit zero shapes"
        );
    }
}
