#[path = "../src/appearance_panel.rs"]
mod appearance_panel;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

use appearance_panel::{
    AppearanceAction, AppearanceInput, AppearancePanel, DEFAULT_TERMINAL_FONT_SIZE,
    MAX_TERMINAL_FONT_SIZE, MIN_TERMINAL_FONT_SIZE,
};
use eframe::egui::accesskit::{Role, Toggled};
use eframe::egui::{self, Color32, Key, Modifiers, Pos2, Rect, Shape, Vec2};
use theme_palettes::{THEMES, ThemeId, WARP_OK_GREEN};

fn collect_shape_texts(shape: &Shape, clip: Rect, out: &mut Vec<(String, Rect, Rect, usize)>) {
    match shape {
        Shape::Text(t) => {
            out.push((
                t.galley.job.text.clone(),
                Rect::from_min_size(t.pos, t.galley.size()),
                clip,
                t.galley.rows.len(),
            ));
        }
        Shape::Vec(list) => {
            for inner in list {
                collect_shape_texts(inner, clip, out);
            }
        }
        _ => {}
    }
}

fn painted_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Rect, Rect, usize)> {
    let mut out = Vec::new();
    for cs in shapes {
        collect_shape_texts(&cs.shape, cs.clip_rect, &mut out);
    }
    out
}

fn has_painted_text(shapes: &[egui::epaint::ClippedShape], exact: &str) -> bool {
    painted_texts(shapes)
        .iter()
        .any(|(text, _, _, _)| text == exact)
}

fn find_painted_text_rect(shapes: &[egui::epaint::ClippedShape], exact: &str) -> Option<Rect> {
    painted_texts(shapes)
        .into_iter()
        .find(|(text, _, _, _)| text == exact)
        .map(|(_, rect, _, _)| rect)
}

fn has_rect_with_fill(shapes: &[egui::epaint::ClippedShape], target: Color32) -> bool {
    fn walk(shape: &Shape, target: Color32) -> bool {
        match shape {
            Shape::Rect(r) => r.fill == target && r.rect.width() >= 2.0 && r.rect.height() >= 2.0,
            Shape::Vec(list) => list.iter().any(|s| walk(s, target)),
            _ => false,
        }
    }
    shapes.iter().any(|cs| walk(&cs.shape, target))
}

fn has_circle_with_fill(shapes: &[egui::epaint::ClippedShape], target: Color32) -> bool {
    fn walk(shape: &Shape, target: Color32) -> bool {
        match shape {
            Shape::Circle(c) => c.fill == target && c.radius >= 2.5,
            Shape::Vec(list) => list.iter().any(|s| walk(s, target)),
            _ => false,
        }
    }
    shapes.iter().any(|cs| walk(&cs.shape, target))
}

fn find_slider_track_rect(shapes: &[egui::epaint::ClippedShape], min_y: f32) -> Option<Rect> {
    fn walk(shape: &Shape, min_y: f32, out: &mut Vec<Rect>) {
        match shape {
            Shape::Rect(r) => {
                if r.rect.min.y >= min_y
                    && r.rect.width() >= 65.0
                    && (r.rect.height() - 4.0).abs() <= 0.6
                {
                    out.push(r.rect);
                }
            }
            Shape::Vec(list) => {
                for s in list {
                    walk(s, min_y, out);
                }
            }
            _ => {}
        }
    }
    let mut candidates = Vec::new();
    for cs in shapes {
        walk(&cs.shape, min_y, &mut candidates);
    }
    candidates
        .into_iter()
        .max_by(|a, b| a.width().partial_cmp(&b.width()).unwrap())
}

fn count_vector_strokes(shapes: &[egui::epaint::ClippedShape]) -> usize {
    fn walk(shape: &Shape) -> usize {
        match shape {
            Shape::LineSegment { stroke, .. } => usize::from(stroke.width > 0.0),
            Shape::Path(p) => usize::from(p.points.len() >= 2 && p.stroke.width > 0.0),
            Shape::Vec(list) => list.iter().map(walk).sum(),
            _ => 0,
        }
    }
    shapes.iter().map(|cs| walk(&cs.shape)).sum()
}

fn key_event(key: Key, pressed: bool, repeat: bool, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers,
    }
}

fn click_events(pos: Pos2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn run_panel_frame(
    ctx: &egui::Context,
    panel: &mut AppearancePanel,
    input: AppearanceInput,
    screen_size: Vec2,
    events: Vec<egui::Event>,
    focused: bool,
) -> (egui::FullOutput, Option<AppearanceAction>) {
    let mut got = None;
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen_size)),
            events,
            focused,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                got = panel.show(ui, &input);
            });
        },
    );
    (out, got)
}

#[test]
fn plain_render_all_seven_themes_palette_previews_localization_and_truthful_copy() {
    let ctx = egui::Context::default();
    let mut panel = AppearancePanel::new();

    // 1. Out-of-range and non-finite font sizes clamp cleanly on display and never emit an action on plain render.
    for (raw_size, expected_px) in [
        (4.0_f32, "10 px"),
        (99.0_f32, "28 px"),
        (f32::NAN, "15 px"),
        (f32::INFINITY, "15 px"),
    ] {
        let (out, act) = run_panel_frame(
            &ctx,
            &mut panel,
            AppearanceInput {
                theme: Some(ThemeId::Classic),
                dark: true,
                french: false,
                terminal_font_size: raw_size,
            },
            Vec2::new(900.0, 900.0),
            Vec::new(),
            true,
        );
        assert_eq!(
            act, None,
            "plain render must never emit an action for size={raw_size}"
        );
        assert!(
            has_painted_text(&out.shapes, expected_px),
            "expected clamped font badge `{expected_px}` for raw_size={raw_size}"
        );
    }
    assert_eq!(MIN_TERMINAL_FONT_SIZE, 10.0);
    assert_eq!(MAX_TERMINAL_FONT_SIZE, 28.0);
    assert_eq!(DEFAULT_TERMINAL_FONT_SIZE, 15.0);

    // 2. English & French headers, all 7 theme names & descriptions, palette swatches, and truthful scope copy.
    for (french, page_h, theme_h, font_h, font_row, font_scope) in [
        (
            false,
            "Appearance",
            "Theme",
            "Terminal font",
            "Terminal font size",
            "Applies to terminal panes and the code editor (10–28 px), not the whole application interface.",
        ),
        (
            true,
            "Apparence",
            "Thème",
            "Police du terminal",
            "Taille de police du terminal",
            "S’applique aux panneaux de terminal et à l’éditeur de code (10 à 28 px), pas à toute l’interface.",
        ),
    ] {
        let (out, act) = run_panel_frame(
            &ctx,
            &mut panel,
            AppearanceInput {
                theme: Some(ThemeId::Warp),
                dark: true,
                french,
                terminal_font_size: 15.0,
            },
            Vec2::new(900.0, 920.0),
            Vec::new(),
            true,
        );
        assert_eq!(act, None);
        assert!(has_painted_text(&out.shapes, page_h));
        assert!(has_painted_text(&out.shapes, "Interface"));
        assert!(has_painted_text(&out.shapes, theme_h));
        assert!(has_painted_text(&out.shapes, font_h));
        assert!(has_painted_text(&out.shapes, font_row));
        assert!(has_painted_text(&out.shapes, font_scope));
        assert!(has_painted_text(&out.shapes, "English"));
        assert!(has_painted_text(&out.shapes, "Français"));

        for theme in THEMES {
            let label = theme.label(french);
            let desc = theme.description(french);
            assert!(
                has_painted_text(&out.shapes, label),
                "missing theme card title `{label}` (french={french})"
            );
            assert!(
                has_painted_text(&out.shapes, desc),
                "missing theme card description `{desc}` (french={french})"
            );
            let spec = theme.spec();
            assert!(
                has_rect_with_fill(&out.shapes, spec.term.background),
                "missing miniature terminal bg preview for {:?}",
                theme
            );
            assert!(
                has_rect_with_fill(&out.shapes, spec.ui.chrome),
                "missing miniature chrome bar preview for {:?}",
                theme
            );
            assert!(
                has_circle_with_fill(&out.shapes, spec.ui.accent),
                "missing accent swatch dot for {:?}",
                theme
            );
            if let Some(ansi) = &spec.term.ansi {
                assert!(
                    has_circle_with_fill(&out.shapes, ansi.green),
                    "missing ANSI green swatch dot for {:?}",
                    theme
                );
                assert!(
                    has_circle_with_fill(&out.shapes, ansi.cyan),
                    "missing ANSI cyan swatch dot for {:?}",
                    theme
                );
            }
        }
        assert!(
            has_circle_with_fill(&out.shapes, WARP_OK_GREEN),
            "Warp card must render WARP_OK_GREEN swatch dot"
        );

        // Zero unimplemented controls or ASCII glyph substitutes.
        for forbidden in [
            "Font family",
            "Famille de police",
            "Cursor style",
            "Style du curseur",
            "UI zoom",
            "Zoom de l’interface",
            "100%",
            "+",
            "-",
            "−",
            "✓",
        ] {
            assert!(
                !has_painted_text(&out.shapes, forbidden),
                "unexpected dummy or ASCII glyph text `{forbidden}` painted"
            );
        }
        assert!(
            count_vector_strokes(&out.shapes) >= 5,
            "expected vector strokes for active theme checkmark and +/- stepper icons"
        );
    }
}

#[test]
fn legacy_light_honesty_vs_default_dark_and_theme_card_clicks() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut panel = AppearancePanel::new();

    // 1. Legacy light (`theme: None, dark: false`) in EN & FR: shows honest callout, marks 0 theme cards selected,
    //    and clicking `Classic` emits `Theme(Classic)` without inventing an 8th theme option.
    for (french, expected_callout, classic_label) in [
        (
            false,
            "Legacy light theme active — choose one of the 7 themes below to switch.",
            "Classic",
        ),
        (
            true,
            "Thème clair hérité actif — choisissez l’un des 7 thèmes ci-dessous pour changer.",
            "Classique",
        ),
    ] {
        let (out, act) = run_panel_frame(
            &ctx,
            &mut panel,
            AppearanceInput {
                theme: None,
                dark: false,
                french,
                terminal_font_size: 15.0,
            },
            Vec2::new(900.0, 920.0),
            Vec::new(),
            true,
        );
        assert_eq!(act, None);
        assert!(
            has_painted_text(&out.shapes, expected_callout),
            "expected legacy light callout `{expected_callout}`"
        );

        let radio_nodes: Vec<(String, Option<Toggled>)> = out
            .platform_output
            .accesskit_update
            .as_ref()
            .map(|u| {
                u.nodes
                    .iter()
                    .filter(|(_, n)| n.role() == Role::RadioButton)
                    .map(|(_, n)| (n.label().unwrap_or_default().to_owned(), n.toggled()))
                    .collect()
            })
            .unwrap_or_default();
        // Exactly 2 language radios + 7 theme radios = 9 total (no invented 8th theme radio!).
        assert_eq!(
            radio_nodes.len(),
            9,
            "expected 2 language + 7 theme radio buttons, got {radio_nodes:?}"
        );
        let selected_radios: Vec<&str> = radio_nodes
            .iter()
            .filter(|(_, t)| *t == Some(Toggled::True))
            .map(|(lbl, _)| lbl.as_str())
            .collect();
        let expected_lang = if french { "Français" } else { "English" };
        assert_eq!(
            selected_radios,
            vec![expected_lang],
            "in legacy light mode, 0 of the 7 theme cards may pretend to be selected: {radio_nodes:?}"
        );

        let classic_pos = find_painted_text_rect(&out.shapes, classic_label)
            .expect("Classic title rect")
            .center();
        let (_, click_act) = run_panel_frame(
            &ctx,
            &mut panel,
            AppearanceInput {
                theme: None,
                dark: false,
                french,
                terminal_font_size: 15.0,
            },
            Vec2::new(900.0, 920.0),
            click_events(classic_pos),
            true,
        );
        assert_eq!(
            click_act,
            Some(AppearanceAction::Theme(ThemeId::Classic)),
            "clicking Classic from legacy light must switch to Classic"
        );
    }

    // 2. Default dark (`theme: None, dark: true`): no legacy callout, `Classic` is active (clicking it emits None),
    //    clicking `Dracula` emits `Theme(Dracula)`, and clicking `Solarized Dark` emits `Theme(SolarizedDark)`.
    let (out_dark, _) = run_panel_frame(
        &ctx,
        &mut panel,
        AppearanceInput {
            theme: None,
            dark: true,
            french: false,
            terminal_font_size: 15.0,
        },
        Vec2::new(900.0, 920.0),
        Vec::new(),
        true,
    );
    assert!(
        !has_painted_text(
            &out_dark.shapes,
            "Legacy light theme active — choose one of the 7 themes below to switch."
        ),
        "default dark mode must not show legacy light callout"
    );

    let classic_pos = find_painted_text_rect(&out_dark.shapes, "Classic")
        .expect("Classic")
        .center();
    let (_, same_act) = run_panel_frame(
        &ctx,
        &mut panel,
        AppearanceInput {
            theme: None,
            dark: true,
            french: false,
            terminal_font_size: 15.0,
        },
        Vec2::new(900.0, 920.0),
        click_events(classic_pos),
        true,
    );
    assert_eq!(
        same_act, None,
        "clicking already-effective Classic in dark mode must emit None"
    );

    for (title, expected_id) in [
        ("Dracula", ThemeId::Dracula),
        ("Tokyo Night", ThemeId::TokyoNight),
        ("Solarized Dark", ThemeId::SolarizedDark),
    ] {
        let pos = find_painted_text_rect(&out_dark.shapes, title)
            .unwrap_or_else(|| panic!("missing `{title}` rect"))
            .center();
        let (_, got) = run_panel_frame(
            &ctx,
            &mut panel,
            AppearanceInput {
                theme: Some(ThemeId::Classic),
                dark: true,
                french: false,
                terminal_font_size: 15.0,
            },
            Vec2::new(900.0, 920.0),
            click_events(pos),
            true,
        );
        assert_eq!(got, Some(AppearanceAction::Theme(expected_id)));
    }
}

#[test]
fn language_toggle_and_terminal_font_stepper_slider_and_mid_drag_cancellation() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut panel = AppearancePanel::new();

    let base_input = AppearanceInput {
        theme: Some(ThemeId::Dracula),
        dark: true,
        french: false,
        terminal_font_size: 15.0,
    };
    let (out0, act0) = run_panel_frame(
        &ctx,
        &mut panel,
        base_input,
        Vec2::new(900.0, 920.0),
        Vec::new(),
        true,
    );
    assert_eq!(act0, None);

    // 1. Language segmented control clicks.
    let en_pos = find_painted_text_rect(&out0.shapes, "English")
        .expect("English")
        .center();
    let fr_pos = find_painted_text_rect(&out0.shapes, "Français")
        .expect("Français")
        .center();

    let (_, click_en_when_en) = run_panel_frame(
        &ctx,
        &mut panel,
        base_input,
        Vec2::new(900.0, 920.0),
        click_events(en_pos),
        true,
    );
    assert_eq!(click_en_when_en, None);

    let (_, click_fr_when_en) = run_panel_frame(
        &ctx,
        &mut panel,
        base_input,
        Vec2::new(900.0, 920.0),
        click_events(fr_pos),
        true,
    );
    assert_eq!(
        click_fr_when_en,
        Some(AppearanceAction::Language { french: true })
    );

    let fr_input = AppearanceInput {
        french: true,
        ..base_input
    };
    let (out_fr, _) = run_panel_frame(
        &ctx,
        &mut panel,
        fr_input,
        Vec2::new(900.0, 920.0),
        Vec::new(),
        true,
    );
    let en_pos_fr = find_painted_text_rect(&out_fr.shapes, "English")
        .expect("English in FR")
        .center();
    let (_, click_en_when_fr) = run_panel_frame(
        &ctx,
        &mut panel,
        fr_input,
        Vec2::new(900.0, 920.0),
        click_events(en_pos_fr),
        true,
    );
    assert_eq!(
        click_en_when_fr,
        Some(AppearanceAction::Language { french: false })
    );

    // 2. Stepper `-` and `+` buttons + 10..=28 bounds.
    let val_rect = find_painted_text_rect(&out0.shapes, "15 px").expect("15 px badge");
    // `val_rect` is centered in the 48px middle cell of the 108px stepper (`[30px -][48px val][30px +]`).
    let minus_pos = Pos2::new(val_rect.center().x - 39.0, val_rect.center().y);
    let plus_pos = Pos2::new(val_rect.center().x + 39.0, val_rect.center().y);

    let (_, shrink_act) = run_panel_frame(
        &ctx,
        &mut panel,
        base_input,
        Vec2::new(900.0, 920.0),
        click_events(minus_pos),
        true,
    );
    assert_eq!(shrink_act, Some(AppearanceAction::TerminalFontSize(14.0)));

    let (_, grow_act) = run_panel_frame(
        &ctx,
        &mut panel,
        base_input,
        Vec2::new(900.0, 920.0),
        click_events(plus_pos),
        true,
    );
    assert_eq!(grow_act, Some(AppearanceAction::TerminalFontSize(16.0)));

    // At minimum bound 10.0: `-` emits None and is marked disabled in AccessKit; `+` emits 11.0.
    let min_input = AppearanceInput {
        terminal_font_size: 10.0,
        ..base_input
    };
    let (out_min, _) = run_panel_frame(
        &ctx,
        &mut panel,
        min_input,
        Vec2::new(900.0, 920.0),
        Vec::new(),
        true,
    );
    let smaller_disabled = out_min
        .platform_output
        .accesskit_update
        .as_ref()
        .and_then(|u| {
            u.nodes
                .iter()
                .find(|(_, n)| n.role() == Role::Button && n.label() == Some("Smaller"))
                .map(|(_, n)| n.is_disabled())
        });
    assert_eq!(smaller_disabled, Some(true));
    let (_, below_min) = run_panel_frame(
        &ctx,
        &mut panel,
        min_input,
        Vec2::new(900.0, 920.0),
        click_events(minus_pos),
        true,
    );
    assert_eq!(below_min, None);
    let (_, from_min_plus) = run_panel_frame(
        &ctx,
        &mut panel,
        min_input,
        Vec2::new(900.0, 920.0),
        click_events(plus_pos),
        true,
    );
    assert_eq!(
        from_min_plus,
        Some(AppearanceAction::TerminalFontSize(11.0))
    );

    // At maximum bound 28.0: `+` emits None and is marked disabled in AccessKit; `-` emits 27.0.
    let max_input = AppearanceInput {
        terminal_font_size: 28.0,
        ..base_input
    };
    let (out_max, _) = run_panel_frame(
        &ctx,
        &mut panel,
        max_input,
        Vec2::new(900.0, 920.0),
        Vec::new(),
        true,
    );
    let bigger_disabled = out_max
        .platform_output
        .accesskit_update
        .as_ref()
        .and_then(|u| {
            u.nodes
                .iter()
                .find(|(_, n)| n.role() == Role::Button && n.label() == Some("Bigger"))
                .map(|(_, n)| n.is_disabled())
        });
    assert_eq!(bigger_disabled, Some(true));
    let (_, above_max) = run_panel_frame(
        &ctx,
        &mut panel,
        max_input,
        Vec2::new(900.0, 920.0),
        click_events(plus_pos),
        true,
    );
    assert_eq!(above_max, None);
    let (_, from_max_minus) = run_panel_frame(
        &ctx,
        &mut panel,
        max_input,
        Vec2::new(900.0, 920.0),
        click_events(minus_pos),
        true,
    );
    assert_eq!(
        from_max_minus,
        Some(AppearanceAction::TerminalFontSize(27.0))
    );

    // 3. Range slider click, continuous drag, and mid-drag Popup / window-focus-loss cancellation.
    let track_rect =
        find_slider_track_rect(&out0.shapes, val_rect.min.y - 12.0).expect("slider track rect");
    let left_track = Pos2::new(track_rect.min.x, track_rect.center().y);
    let right_track = Pos2::new(track_rect.max.x, track_rect.center().y);
    let mid_track = track_rect.center();

    let (_, slider_min) = run_panel_frame(
        &ctx,
        &mut panel,
        base_input,
        Vec2::new(900.0, 920.0),
        click_events(left_track),
        true,
    );
    assert_eq!(slider_min, Some(AppearanceAction::TerminalFontSize(10.0)));

    let (_, slider_max) = run_panel_frame(
        &ctx,
        &mut panel,
        base_input,
        Vec2::new(900.0, 920.0),
        click_events(right_track),
        true,
    );
    assert_eq!(slider_max, Some(AppearanceAction::TerminalFontSize(28.0)));

    // Start dragging at left_track (10 px) while current is 15 px -> emits 10.0 on press.
    let (_, drag_press) = run_panel_frame(
        &ctx,
        &mut panel,
        base_input,
        Vec2::new(900.0, 920.0),
        vec![
            egui::Event::PointerMoved(left_track),
            egui::Event::PointerButton {
                pos: left_track,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
        true,
    );
    assert_eq!(drag_press, Some(AppearanceAction::TerminalFontSize(10.0)));

    // Open a Popup mid-drag while pointer moves to right_track -> cancelled immediately!
    let popup_id = egui::Id::new("mid_drag_popup_guard");
    let mut popup_drag_act = None;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 920.0))),
            events: vec![egui::Event::PointerMoved(right_track)],
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::Popup::open_id(ctx, popup_id);
            egui::CentralPanel::default().show(ctx, |ui| {
                let anchor = ui.button("Popup anchor");
                let _ = egui::Popup::from_response(&anchor).id(popup_id).show(|ui| {
                    ui.label("Open popup");
                });
                popup_drag_act = panel.show(ui, &base_input);
            });
        },
    );
    assert_eq!(
        popup_drag_act, None,
        "mid-drag popup must cancel slider drag"
    );

    // Close popup while mouse button is STILL held down from the cancelled gesture and move to mid_track -> still None!
    let mut after_popup_held_act = None;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 920.0))),
            events: vec![egui::Event::PointerMoved(mid_track)],
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::Popup::close_all(ctx);
            egui::CentralPanel::default().show(ctx, |ui| {
                after_popup_held_act = panel.show(ui, &base_input);
            });
        },
    );
    assert_eq!(
        after_popup_held_act, None,
        "cancelled slider gesture must not resume without a fresh press"
    );

    // Release the cancelled gesture at right_track -> still None!
    let (_, release_cancelled) = run_panel_frame(
        &ctx,
        &mut panel,
        base_input,
        Vec2::new(900.0, 920.0),
        vec![egui::Event::PointerButton {
            pos: right_track,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        true,
    );
    assert_eq!(
        release_cancelled, None,
        "releasing a cancelled slider drag must not fire a click update"
    );

    // Mid-drag window focus loss (`focused: false`) also cancels an active slider drag cleanly.
    let (_, blur_press) = run_panel_frame(
        &ctx,
        &mut panel,
        base_input,
        Vec2::new(900.0, 920.0),
        vec![
            egui::Event::PointerMoved(left_track),
            egui::Event::PointerButton {
                pos: left_track,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
        true,
    );
    assert_eq!(blur_press, Some(AppearanceAction::TerminalFontSize(10.0)));
    let (_, during_blur) = run_panel_frame(
        &ctx,
        &mut panel,
        base_input,
        Vec2::new(900.0, 920.0),
        vec![egui::Event::PointerMoved(right_track)],
        false,
    );
    assert_eq!(during_blur, None);
    let (_, release_after_blur) = run_panel_frame(
        &ctx,
        &mut panel,
        base_input,
        Vec2::new(900.0, 920.0),
        vec![egui::Event::PointerButton {
            pos: right_track,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        true,
    );
    assert_eq!(release_after_blur, None);
}

#[test]
fn focus_scoped_keyboard_navigation_anti_repeat_and_modal_guards() {
    let ctx = egui::Context::default();
    let mut panel = AppearancePanel::new();
    let mut search_text = String::from("theme");
    let search_id = egui::Id::new("external_settings_search");

    let mut state = AppearanceInput {
        theme: Some(ThemeId::Classic),
        dark: true,
        french: false,
        terminal_font_size: 15.0,
    };

    let run_with_search = |ctx: &egui::Context,
                           panel: &mut AppearancePanel,
                           search_text: &mut String,
                           state: AppearanceInput,
                           events: Vec<egui::Event>,
                           enabled: bool,
                           focused: bool| {
        let mut got = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(920.0, 900.0))),
                events,
                focused,
                ..Default::default()
            },
            |ctx| {
                egui::SidePanel::left("mock_settings_nav")
                    .exact_width(180.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(search_text)
                                .id(search_id)
                                .lock_focus(false),
                        );
                    });
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        got = panel.show(ui, &state);
                    });
                });
            },
        );
        got
    };

    // Warm-up frame and focus external search input (`SettingsNav` rail).
    let _ = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        Vec::new(),
        true,
        true,
    );
    ctx.memory_mut(|m| m.request_focus(search_id));
    let _ = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        Vec::new(),
        true,
        true,
    );

    // 1. Keys while external search input has focus must NEVER be stolen by AppearancePanel.
    for k in [Key::ArrowRight, Key::ArrowDown, Key::Plus, Key::Enter] {
        let act = run_with_search(
            &ctx,
            &mut panel,
            &mut search_text,
            state,
            vec![
                key_event(k, true, false, Modifiers::NONE),
                key_event(k, false, false, Modifiers::NONE),
            ],
            true,
            true,
        );
        assert_eq!(
            act, None,
            "external search input focus must keep `{k:?}` untouched"
        );
    }

    // `TextEdit::singleline` surrenders focus on `Key::Enter`, so re-focus `search_id` before testing `Tab` order.
    ctx.memory_mut(|m| m.request_focus(search_id));
    let _ = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        Vec::new(),
        true,
        true,
    );

    // 2. Tab #1 moves focus from `search_id` -> `Language` group (followed by egui's requested repaint frame).
    let _ = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![
            key_event(Key::Tab, true, false, Modifiers::NONE),
            key_event(Key::Tab, false, false, Modifiers::NONE),
        ],
        true,
        true,
    );
    let _ = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        Vec::new(),
        true,
        true,
    );

    // Press `ArrowRight` without releasing yet -> switches to Français once, then held/repeat frames emit None!
    let lang_act = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![key_event(Key::ArrowRight, true, false, Modifiers::NONE)],
        true,
        true,
    );
    assert_eq!(lang_act, Some(AppearanceAction::Language { french: true }));

    let lang_repeat = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![key_event(Key::ArrowRight, true, true, Modifiers::NONE)],
        true,
        true,
    );
    assert_eq!(lang_repeat, None, "held repeat ArrowRight must not re-fire");

    let lang_unreleased = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![key_event(Key::ArrowRight, true, false, Modifiers::NONE)],
        true,
        true,
    );
    assert_eq!(
        lang_unreleased, None,
        "unreleased ArrowRight must not re-fire before Key release"
    );

    // Release ArrowRight and apply `state.french = true`, then `ArrowLeft` switches back to English!
    state.french = true;
    let _ = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![key_event(Key::ArrowRight, false, false, Modifiers::NONE)],
        true,
        true,
    );
    let back_to_en = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![
            key_event(Key::ArrowLeft, true, false, Modifiers::NONE),
            key_event(Key::ArrowLeft, false, false, Modifiers::NONE),
        ],
        true,
        true,
    );
    assert_eq!(
        back_to_en,
        Some(AppearanceAction::Language { french: false })
    );
    state.french = false;

    // 3. Tab #2 moves focus from `Language` -> `Themes` group.
    let _ = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![
            key_event(Key::Tab, true, false, Modifiers::NONE),
            key_event(Key::Tab, false, false, Modifiers::NONE),
        ],
        true,
        true,
    );
    let _ = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        Vec::new(),
        true,
        true,
    );

    // `Enter` on already-selected `Classic` emits None.
    let enter_same = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![
            key_event(Key::Enter, true, false, Modifiers::NONE),
            key_event(Key::Enter, false, false, Modifiers::NONE),
        ],
        true,
        true,
    );
    assert_eq!(enter_same, None);

    // `ArrowLeft` wraps from `Classic` (0) -> `SolarizedDark` (6); `Enter` (held) emits `Theme(SolarizedDark)` once!
    let wrap_move = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![
            key_event(Key::ArrowLeft, true, false, Modifiers::NONE),
            key_event(Key::ArrowLeft, false, false, Modifiers::NONE),
        ],
        true,
        true,
    );
    assert_eq!(
        wrap_move, None,
        "arrow move updates cursor without applying"
    );

    let enter_solarized = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![key_event(Key::Enter, true, false, Modifiers::NONE)],
        true,
        true,
    );
    assert_eq!(
        enter_solarized,
        Some(AppearanceAction::Theme(ThemeId::SolarizedDark))
    );
    let enter_held_again = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![key_event(Key::Enter, true, false, Modifiers::NONE)],
        true,
        true,
    );
    assert_eq!(enter_held_again, None);
    let _ = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![key_event(Key::Enter, false, false, Modifiers::NONE)],
        true,
        true,
    );
    state.theme = Some(ThemeId::SolarizedDark);

    // 4. Tab #3 moves focus from `Themes` -> `FontSize` group.
    let _ = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![
            key_event(Key::Tab, true, false, Modifiers::NONE),
            key_event(Key::Tab, false, false, Modifiers::NONE),
        ],
        true,
        true,
    );
    let _ = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        Vec::new(),
        true,
        true,
    );

    // `Shift+Equals` (`+` on US QWERTY) steps +1.0 (`15 -> 16`); `Minus` steps -1.0 (`16 -> 15`).
    let step_up = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![
            key_event(Key::Equals, true, false, Modifiers::SHIFT),
            key_event(Key::Equals, false, false, Modifiers::SHIFT),
        ],
        true,
        true,
    );
    assert_eq!(step_up, Some(AppearanceAction::TerminalFontSize(16.0)));
    state.terminal_font_size = 16.0;

    let step_down = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![
            key_event(Key::Minus, true, false, Modifiers::NONE),
            key_event(Key::Minus, false, false, Modifiers::NONE),
        ],
        true,
        true,
    );
    assert_eq!(step_down, Some(AppearanceAction::TerminalFontSize(15.0)));

    // 5. Disabled UI (`enabled: false`) and unfocused window (`focused: false`) block keyboard actions.
    let disabled_act = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![
            key_event(Key::ArrowRight, true, false, Modifiers::NONE),
            key_event(Key::ArrowRight, false, false, Modifiers::NONE),
        ],
        false,
        true,
    );
    assert_eq!(disabled_act, None);

    let unfocused_act = run_with_search(
        &ctx,
        &mut panel,
        &mut search_text,
        state,
        vec![
            key_event(Key::ArrowRight, true, false, Modifiers::NONE),
            key_event(Key::ArrowRight, false, false, Modifiers::NONE),
        ],
        true,
        false,
    );
    assert_eq!(unfocused_act, None);

    panel.reset();
}

#[test]
fn responsive_320px_narrow_and_870px_wide_layouts_never_overflow_and_expose_accesskit() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut panel = AppearancePanel::new();

    // 1. Narrow 320px layout (French + Legacy light callout): 1-column theme cards, stacked rows, zero overflow.
    let mut narrow_panel_rect = Rect::NOTHING;
    let out_narrow = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 1300.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            let resp = egui::SidePanel::left("appearance_narrow_320")
                .default_width(320.0)
                .max_width(320.0)
                .resizable(false)
                .show(ctx, |ui| {
                    let _ = panel.show(
                        ui,
                        &AppearanceInput {
                            theme: None,
                            dark: false,
                            french: true,
                            terminal_font_size: 15.0,
                        },
                    );
                });
            narrow_panel_rect = resp.response.rect;
            egui::CentralPanel::default().show(ctx, |_ui| {});
        },
    );

    assert!(
        narrow_panel_rect.width() <= 324.0,
        "AppearancePanel must never inflate a 320px container; got {}",
        narrow_panel_rect.width()
    );
    for (text, rect, clip, _) in painted_texts(&out_narrow.shapes) {
        let vis = rect.intersect(clip);
        if vis.is_positive() && vis.min.x >= narrow_panel_rect.min.x - 2.0 {
            assert!(
                vis.max.x <= narrow_panel_rect.max.x + 2.0,
                "text `{text}` overflowed 320px panel bounds: vis={vis:?}, panel={narrow_panel_rect:?}"
            );
        }
    }
    let fr_classic = find_painted_text_rect(&out_narrow.shapes, "Classique").expect("Classique");
    let fr_warp =
        find_painted_text_rect(&out_narrow.shapes, "Inspiré de Warp").expect("Inspiré de Warp");
    assert!(
        (fr_classic.min.x - fr_warp.min.x).abs() <= 2.0 && fr_warp.min.y > fr_classic.max.y + 40.0,
        "at 320px width, theme cards must stack in 1 column: classic={fr_classic:?}, warp={fr_warp:?}"
    );

    // 2. Wide 870px layout (English): 3-column theme cards, inline controls, zero overflow, AccessKit roles present.
    let mut wide_panel_rect = Rect::NOTHING;
    let out_wide = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 920.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            let resp = egui::CentralPanel::default().show(ctx, |ui| {
                let _ = panel.show(
                    ui,
                    &AppearanceInput {
                        theme: Some(ThemeId::Nord),
                        dark: true,
                        french: false,
                        terminal_font_size: 18.0,
                    },
                );
            });
            wide_panel_rect = resp.response.rect;
        },
    );

    for (text, rect, clip, _) in painted_texts(&out_wide.shapes) {
        let vis = rect.intersect(clip);
        if vis.is_positive() {
            assert!(
                vis.max.x <= wide_panel_rect.max.x + 2.0,
                "text `{text}` overflowed wide panel bounds: vis={vis:?}, panel={wide_panel_rect:?}"
            );
        }
    }
    let en_classic = find_painted_text_rect(&out_wide.shapes, "Classic").expect("Classic");
    let en_warp = find_painted_text_rect(&out_wide.shapes, "Warp-inspired").expect("Warp-inspired");
    let en_dracula = find_painted_text_rect(&out_wide.shapes, "Dracula").expect("Dracula");
    assert!(
        (en_classic.center().y - en_warp.center().y).abs() <= 2.0
            && (en_warp.center().y - en_dracula.center().y).abs() <= 2.0
            && en_warp.min.x > en_classic.max.x + 40.0
            && en_dracula.min.x > en_warp.max.x + 40.0,
        "at >=680px width, first 3 theme cards must render in a 3-column row"
    );

    let slider_labels: Vec<String> = out_wide
        .platform_output
        .accesskit_update
        .as_ref()
        .map(|u| {
            u.nodes
                .iter()
                .filter(|(_, n)| n.role() == Role::Slider)
                .filter_map(|(_, n)| n.label().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(slider_labels, vec!["Terminal font size (18 px)"]);
}
