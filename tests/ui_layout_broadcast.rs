//! Standalone headless `egui` + `AccessKit` parity tests for `src/layout_broadcast_ui.rs`
//! (toolbar `Layout` `290px` menu, conditional `Arrange panes in a grid`, `Even grid` `1 x 2..3 x 3`
//! presets & non-destructive `plan_preset_grid`, toolbar `Broadcast` button `aria-pressed` + `.warn`
//! styling, `24px` broadcast banner, per-pane `write` / `écrire` toggle + active warn border,
//! `TERMINAL_REPLY` byte filter, and EN/FR status-bar / command-palette copy across all 7 themes).

#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[path = "../src/layout_broadcast_ui.rs"]
mod layout_broadcast_ui;
#[allow(dead_code)]
#[path = "../src/pane_grid.rs"]
mod pane_grid;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

use eframe::egui::accesskit::{Role, Toggled};
use eframe::egui::{self, Color32, Event, Modifiers, PointerButton, Pos2, Rect, Shape, Vec2};
use layout_broadcast_ui::{
    BROADCAST_BANNER_HEIGHT, BROADCAST_MEMBER_TEXT, BROADCAST_TOOLBAR_ON_TEXT, LAYOUT_MENU_WIDTH,
    LayoutMenuAction, LayoutMenuInput, arrange_grid_label, broadcast_banner_text,
    broadcast_command_title, broadcast_toolbar_label, broadcast_toolbar_tooltip,
    close_active_command_title, close_active_menu_label, even_grid_command_title,
    even_grid_heading, format_broadcast_status_target, is_terminal_reply, layout_menu_frame,
    layout_toolbar_label, paint_broadcast_toolbar_button, paint_layout_toolbar_button,
    pane_active_stroke_color, pane_broadcast_label, pane_broadcast_toggle_width, plan_preset_grid,
    preset_display_label, show_broadcast_banner, show_layout_menu_contents,
    show_pane_broadcast_toggle, split_down_label, split_right_label,
};
use pane_grid::GridPreset;
use theme_palettes::{THEMES, ThemeId};

fn collect_shape_texts(shape: &Shape, clip: Rect, out: &mut Vec<(String, Rect, Color32, Rect)>) {
    match shape {
        Shape::Text(t) => {
            let color = t
                .override_text_color
                .or_else(|| t.galley.job.sections.first().map(|s| s.format.color))
                .unwrap_or(Color32::TRANSPARENT);
            out.push((
                t.galley.job.text.clone(),
                Rect::from_min_size(t.pos, t.galley.size()),
                color,
                clip,
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

fn painted_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Rect, Color32, Rect)> {
    let mut out = Vec::new();
    for cs in shapes {
        collect_shape_texts(&cs.shape, cs.clip_rect, &mut out);
    }
    out
}

fn has_painted_text(shapes: &[egui::epaint::ClippedShape], exact: &str) -> bool {
    painted_texts(shapes)
        .iter()
        .any(|(text, rect, _, clip)| text == exact && clip.intersects(*rect))
}

fn find_painted_text_center(shapes: &[egui::epaint::ClippedShape], exact: &str) -> Option<Pos2> {
    painted_texts(shapes)
        .into_iter()
        .find(|(text, rect, _, clip)| text == exact && clip.intersects(*rect))
        .map(|(_, rect, _, clip)| rect.intersect(clip).center())
}

fn click_events(pos: Pos2) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn accesskit_node_toggled(out: &egui::FullOutput, role: Role, label: &str) -> Option<Toggled> {
    let update = out.platform_output.accesskit_update.as_ref()?;
    for (_, node) in &update.nodes {
        if node.role() == role && node.label() == Some(label) {
            return node.toggled();
        }
    }
    None
}

fn run_layout_menu_frame(
    ctx: &egui::Context,
    events: Vec<Event>,
    theme: ThemeId,
    input: LayoutMenuInput,
) -> (egui::FullOutput, Rect, Option<LayoutMenuAction>) {
    let area_id = egui::Id::new("test_layout_menu_area");
    let spec = theme.spec();
    // An unseen `egui::Area` settles its clip rect on its second pass (`egui` two-pass Area sizing).
    if events.is_empty() && ctx.memory(|m| m.area_rect(area_id).is_none()) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 640.0))),
                ..Default::default()
            },
            |ctx| {
                egui::Area::new(area_id)
                    .fixed_pos(Pos2::new(80.0, 40.0))
                    .show(ctx, |ui| {
                        layout_menu_frame(&spec.ui)
                            .show(ui, |ui| show_layout_menu_contents(ui, &spec.ui, input));
                    });
            },
        );
    }
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 640.0))),
        events,
        ..Default::default()
    };
    let mut outer_rect = Rect::NOTHING;
    let mut emitted = None;
    let out = ctx.run(raw, |ctx| {
        egui::Area::new(area_id)
            .fixed_pos(Pos2::new(80.0, 40.0))
            .show(ctx, |ui| {
                let inner = layout_menu_frame(&spec.ui)
                    .show(ui, |ui| show_layout_menu_contents(ui, &spec.ui, input));
                outer_rect = inner.response.rect;
                emitted = inner.inner;
            });
    });
    (out, outer_rect, emitted)
}

#[test]
fn layout_menu_290px_geometry_and_conditional_arrange_grid_en_fr() {
    for french in [false, true] {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        let (out_single, rect_single, action_single) = run_layout_menu_frame(
            &ctx,
            Vec::new(),
            ThemeId::Classic,
            LayoutMenuInput {
                french,
                has_active_pane: true,
                can_split_shell: true,
                pane_count: 1,
                can_apply_grid: true,
            },
        );
        assert_eq!(action_single, None);
        assert!(
            (rect_single.width() - LAYOUT_MENU_WIDTH).abs() <= 0.5,
            "expected 290px layout menu width, got {}",
            rect_single.width()
        );
        assert!(has_painted_text(
            &out_single.shapes,
            split_right_label(french)
        ));
        assert!(has_painted_text(&out_single.shapes, "Ctrl+Shift+E"));
        assert!(has_painted_text(
            &out_single.shapes,
            split_down_label(french)
        ));
        assert!(has_painted_text(&out_single.shapes, "Ctrl+Shift+O"));
        assert!(
            !has_painted_text(&out_single.shapes, arrange_grid_label(french)),
            "Arrange panes in a grid must NOT appear when pane_count <= 1"
        );
        assert!(has_painted_text(
            &out_single.shapes,
            even_grid_heading(french)
        ));
        for preset in GridPreset::ALL {
            assert!(has_painted_text(
                &out_single.shapes,
                preset_display_label(preset)
            ));
        }
        assert!(has_painted_text(
            &out_single.shapes,
            close_active_menu_label(french)
        ));
        assert!(has_painted_text(&out_single.shapes, "Ctrl+Shift+W"));

        // With 2 panes, "Arrange panes in a grid" appears and emits ArrangeGrid on click.
        let (out_multi, rect_multi, _) = run_layout_menu_frame(
            &ctx,
            Vec::new(),
            ThemeId::Classic,
            LayoutMenuInput {
                french,
                has_active_pane: true,
                can_split_shell: true,
                pane_count: 2,
                can_apply_grid: true,
            },
        );
        assert!((rect_multi.width() - LAYOUT_MENU_WIDTH).abs() <= 0.5);
        assert!(rect_multi.height() > rect_single.height());
        let arrange_pos = find_painted_text_center(&out_multi.shapes, arrange_grid_label(french))
            .expect("Arrange panes in a grid should be painted for 2 panes");
        let (_, _, arrange_action) = run_layout_menu_frame(
            &ctx,
            click_events(arrange_pos),
            ThemeId::Classic,
            LayoutMenuInput {
                french,
                has_active_pane: true,
                can_split_shell: true,
                pane_count: 2,
                can_apply_grid: true,
            },
        );
        assert_eq!(arrange_action, Some(LayoutMenuAction::ArrangeGrid));
    }
}

#[test]
fn layout_menu_clicks_emit_split_preset_and_close_actions_and_respect_disabled() {
    let ctx = egui::Context::default();
    let input = LayoutMenuInput {
        french: false,
        has_active_pane: true,
        can_split_shell: true,
        pane_count: 3,
        can_apply_grid: true,
    };
    let (out, _, _) = run_layout_menu_frame(&ctx, Vec::new(), ThemeId::Classic, input);

    let cases = [
        ("Split right", LayoutMenuAction::SplitRight),
        ("Split down", LayoutMenuAction::SplitDown),
        ("1 x 2", LayoutMenuAction::ApplyPreset(GridPreset::OneByTwo)),
        ("2 x 1", LayoutMenuAction::ApplyPreset(GridPreset::TwoByOne)),
        ("2 x 2", LayoutMenuAction::ApplyPreset(GridPreset::TwoByTwo)),
        (
            "3 x 2",
            LayoutMenuAction::ApplyPreset(GridPreset::ThreeByTwo),
        ),
        (
            "3 x 3",
            LayoutMenuAction::ApplyPreset(GridPreset::ThreeByThree),
        ),
        ("Close active pane", LayoutMenuAction::CloseActive),
    ];

    for (label, expected) in cases {
        let pos = find_painted_text_center(&out.shapes, label)
            .unwrap_or_else(|| panic!("missing menu item {label}"));
        let (_, _, got) = run_layout_menu_frame(&ctx, click_events(pos), ThemeId::Classic, input);
        assert_eq!(got, Some(expected), "unexpected action for {label}");
    }

    // When no active pane exists (`has_active_pane == false`), Split and Close are inert,
    // while Even grid chips still work to populate an empty workspace.
    let empty_input = LayoutMenuInput {
        french: false,
        has_active_pane: false,
        can_split_shell: false,
        pane_count: 0,
        can_apply_grid: true,
    };
    let (empty_out, _, _) = run_layout_menu_frame(&ctx, Vec::new(), ThemeId::Classic, empty_input);
    for disabled_label in ["Split right", "Split down", "Close active pane"] {
        let pos = find_painted_text_center(&empty_out.shapes, disabled_label).unwrap();
        let (_, _, got) =
            run_layout_menu_frame(&ctx, click_events(pos), ThemeId::Classic, empty_input);
        assert_eq!(got, None, "{disabled_label} must be disabled when empty");
    }
    let chip_pos = find_painted_text_center(&empty_out.shapes, "2 x 2").unwrap();
    let (_, _, chip_got) =
        run_layout_menu_frame(&ctx, click_events(chip_pos), ThemeId::Classic, empty_input);
    assert_eq!(
        chip_got,
        Some(LayoutMenuAction::ApplyPreset(GridPreset::TwoByTwo))
    );
}

#[test]
fn preset_grid_plan_keeps_existing_panes_adds_overflow_rows_and_caps_spawns() {
    // 1. Empty workspace -> 2 x 2 spawns 4 shells, selects first spawned shell.
    let plan_empty = plan_preset_grid(GridPreset::TwoByTwo, &[], 100);
    assert_eq!(plan_empty.cols, 2);
    assert_eq!(plan_empty.effective_rows, 2);
    assert_eq!(plan_empty.shells_to_spawn, 4);
    let fin_empty = plan_empty.finalize(&[101, 102, 103, 104], None);
    assert_eq!(fin_empty.ordered_ids, vec![101, 102, 103, 104]);
    assert_eq!(fin_empty.row_counts, vec![2, 2]);
    assert_eq!(fin_empty.active_id, Some(101));

    // 2. Existing panes [10, 20] stay running in order first; active=Some(20) is preserved.
    let plan_keep = plan_preset_grid(GridPreset::ThreeByTwo, &[10, 20], 100);
    assert_eq!(plan_keep.cols, 3);
    assert_eq!(plan_keep.effective_rows, 2);
    assert_eq!(plan_keep.kept_ids, vec![10, 20]);
    assert_eq!(plan_keep.shells_to_spawn, 4);
    let fin_keep = plan_keep.finalize(&[30, 40, 50, 60], Some(20));
    assert_eq!(fin_keep.ordered_ids, vec![10, 20, 30, 40, 50, 60]);
    assert_eq!(fin_keep.row_counts, vec![3, 3]);
    assert_eq!(fin_keep.active_id, Some(20));

    // 3. More panes than preset slots (5 panes into 2 x 2): adds a 3rd row, spawns 1, closes 0!
    let plan_overflow = plan_preset_grid(GridPreset::TwoByTwo, &[1, 2, 3, 4, 5], 100);
    assert_eq!(plan_overflow.cols, 2);
    assert_eq!(plan_overflow.effective_rows, 3);
    assert_eq!(plan_overflow.shells_to_spawn, 1);
    let fin_overflow = plan_overflow.finalize(&[6], Some(999));
    assert_eq!(fin_overflow.ordered_ids, vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(fin_overflow.row_counts, vec![2, 2, 2]);
    assert_eq!(fin_overflow.active_id, Some(1));

    // 4. Global 100-session cap limits new shells without dropping existing panes.
    let plan_capped = plan_preset_grid(GridPreset::ThreeByThree, &[7, 8], 1);
    assert_eq!(plan_capped.shells_to_spawn, 1);
    let fin_capped = plan_capped.finalize(&[9], Some(8));
    assert_eq!(fin_capped.ordered_ids, vec![7, 8, 9]);
    assert_eq!(fin_capped.row_counts, vec![3]);
    assert_eq!(fin_capped.active_id, Some(8));
}

#[test]
fn broadcast_toolbar_button_accesskit_toggled_and_banner_24px_across_7_themes_en_fr() {
    for theme in THEMES {
        let spec = theme.spec();
        for french in [false, true] {
            for broadcast_on in [false, true] {
                let ctx = egui::Context::default();
                ctx.enable_accesskit();

                let mut bc_btn_rect = Rect::NOTHING;
                let mut layout_btn_rect = Rect::NOTHING;
                let mut banner_rect = Rect::NOTHING;

                let out = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 200.0))),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::TopBottomPanel::top("tb").show(ctx, |ui| {
                            ui.horizontal(|ui| {
                                bc_btn_rect = paint_broadcast_toolbar_button(
                                    ui,
                                    &spec.ui,
                                    spec.radius_px,
                                    french,
                                    broadcast_on,
                                )
                                .rect;
                                layout_btn_rect = paint_layout_toolbar_button(
                                    ui,
                                    &spec.ui,
                                    spec.radius_px,
                                    french,
                                    false,
                                )
                                .rect;
                            });
                        });
                        if broadcast_on {
                            egui::TopBottomPanel::top("bc_banner")
                                .exact_height(BROADCAST_BANNER_HEIGHT)
                                .frame(egui::Frame::NONE)
                                .show(ctx, |ui| {
                                    banner_rect = show_broadcast_banner(ui, &spec.ui, french).rect;
                                });
                        }
                    },
                );

                assert_eq!(bc_btn_rect.size(), Vec2::new(30.0, 28.0));
                assert_eq!(layout_btn_rect.size(), Vec2::new(30.0, 28.0));
                let expected_toggled = if broadcast_on {
                    Toggled::True
                } else {
                    Toggled::False
                };
                assert_eq!(
                    accesskit_node_toggled(&out, Role::Button, broadcast_toolbar_label(french)),
                    Some(expected_toggled),
                    "theme={theme:?} french={french} broadcast_on={broadcast_on}"
                );
                assert!(
                    out.platform_output
                        .accesskit_update
                        .as_ref()
                        .unwrap()
                        .nodes
                        .iter()
                        .any(|(_, n)| n.role() == Role::Button
                            && n.label() == Some(layout_toolbar_label(french)))
                );

                let tip = broadcast_toolbar_tooltip(french, broadcast_on);
                assert!(tip.contains("Ctrl+Shift+B"));

                if broadcast_on {
                    assert!((banner_rect.height() - BROADCAST_BANNER_HEIGHT).abs() <= 0.01);
                    let banner_txt = broadcast_banner_text(french);
                    let found = painted_texts(&out.shapes)
                        .into_iter()
                        .find(|(txt, _, _, _)| txt == banner_txt)
                        .expect("broadcast banner text must be painted");
                    assert_eq!(found.2, BROADCAST_MEMBER_TEXT);
                } else {
                    assert!(!has_painted_text(
                        &out.shapes,
                        broadcast_banner_text(french)
                    ));
                }
            }
        }
    }
    assert_eq!(
        BROADCAST_TOOLBAR_ON_TEXT,
        Color32::from_rgb(0xff, 0xe0, 0xa8)
    );
}

#[test]
fn pane_broadcast_write_checkbox_toggles_and_active_border_uses_warn_only_for_active_member() {
    let spec = ThemeId::Classic.spec();
    let default_active = spec.ui.accent;
    let inactive = spec.ui.border;

    // Active terminal pane in broadcast mode with `write` checked -> `palette.warn`.
    assert_eq!(
        pane_active_stroke_color(&spec.ui, true, true, true, true, default_active, inactive),
        spec.ui.warn
    );
    // Inactive pane -> keeps `inactive` border even when broadcast member!
    assert_eq!(
        pane_active_stroke_color(&spec.ui, false, true, true, true, default_active, inactive),
        inactive
    );
    // Unchecked pane or global broadcast off or native chat pane -> `default_active`.
    assert_eq!(
        pane_active_stroke_color(&spec.ui, true, false, true, true, default_active, inactive),
        default_active
    );
    assert_eq!(
        pane_active_stroke_color(&spec.ui, true, true, false, true, default_active, inactive),
        default_active
    );
    assert_eq!(
        pane_active_stroke_color(&spec.ui, true, true, true, false, default_active, inactive),
        default_active
    );

    for french in [false, true] {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut checked = true;
        let mut toggle_rect = Rect::NOTHING;

        let out1 = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 80.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    toggle_rect = show_pane_broadcast_toggle(
                        ui,
                        &spec.ui,
                        ("bc_write", french),
                        french,
                        &mut checked,
                    )
                    .rect;
                });
            },
        );
        assert!(checked);
        assert!(toggle_rect.width() <= pane_broadcast_toggle_width(french));
        assert!(has_painted_text(&out1.shapes, pane_broadcast_label(french)));
        assert_eq!(
            accesskit_node_toggled(&out1, Role::CheckBox, pane_broadcast_label(french)),
            Some(Toggled::True)
        );

        let mut changed_flag = false;
        let out2 = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 80.0))),
                events: click_events(toggle_rect.center()),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let resp = show_pane_broadcast_toggle(
                        ui,
                        &spec.ui,
                        ("bc_write", french),
                        french,
                        &mut checked,
                    );
                    changed_flag = resp.changed();
                });
            },
        );
        assert!(!checked, "clicking write/écrire checkbox must uncheck it");
        assert!(changed_flag);
        assert_eq!(
            accesskit_node_toggled(&out2, Role::CheckBox, pane_broadcast_label(french)),
            Some(Toggled::False)
        );
    }
}

#[test]
fn terminal_reply_filter_matches_app_vue_regex_corpus() {
    let replies: &[&[u8]] = &[
        b"\x1b[?1;2c",
        b"\x1b[>0;276;0c",
        b"\x1b[c",
        b"\x1b[24;80R",
        b"\x1b[?24;1R",
        b"\x1b[0n",
        b"\x1b[?6n",
        b"\x1b[I",
        b"\x1b[O",
        b"\x1b]10;rgb:ffff/ffff/ffff\x07",
        b"\x1b]11;rgb:1010/1212/1616\x1b\\",
        b"\x1b]52;c;SGVsbG8=\x07",
        b"\x1b]\x07",
        b"\x1b]\x1b\\",
    ];
    for seq in replies {
        assert!(
            is_terminal_reply(seq),
            "expected terminal reply match for {seq:?}"
        );
    }

    let user_inputs: &[&[u8]] = &[
        b"",
        b"a",
        b"ls -la\r",
        b"\x03",
        b"\x1b",
        b"\x1b[A",
        b"\x1bOA",
        b"\x1b[1;5C",
        b"\x1b[200~echo hi\x1b[201~",
        b"\x1b[I\x1b[O",
        b"\x1b[24;80Rextra",
        b"prefix\x1b[0n",
        b"\x1b]10;unterminated",
        b"\x1b]10;inner\x1bmid\x07",
        b"\x1b]10;inner\x07extra",
        b"\x1b[<0;12;8M",
    ];
    for seq in user_inputs {
        assert!(
            !is_terminal_reply(seq),
            "expected user input NOT to be filtered as terminal reply: {seq:?}"
        );
    }
}

#[test]
fn status_bar_and_command_palette_copy_en_fr() {
    assert_eq!(
        format_broadcast_status_target(false, 0),
        "Broadcast → 0 panes"
    );
    assert_eq!(
        format_broadcast_status_target(false, 1),
        "Broadcast → 1 pane"
    );
    assert_eq!(
        format_broadcast_status_target(false, 3),
        "Broadcast → 3 panes"
    );
    assert_eq!(
        format_broadcast_status_target(true, 0),
        "Diffusion → 0 panneaux"
    );
    assert_eq!(
        format_broadcast_status_target(true, 1),
        "Diffusion → 1 panneau"
    );
    assert_eq!(
        format_broadcast_status_target(true, 4),
        "Diffusion → 4 panneaux"
    );

    assert_eq!(broadcast_command_title(false, false), "Turn broadcast on");
    assert_eq!(broadcast_command_title(false, true), "Turn broadcast off");
    assert_eq!(broadcast_command_title(true, false), "Activer la diffusion");
    assert_eq!(
        broadcast_command_title(true, true),
        "Désactiver la diffusion"
    );

    assert_eq!(
        even_grid_command_title(false, GridPreset::ThreeByTwo),
        "Even grid 3 x 2"
    );
    assert_eq!(
        even_grid_command_title(true, GridPreset::ThreeByTwo),
        "Grille régulière 3 x 2"
    );
    assert_eq!(close_active_command_title(false), "Close the active pane");
    assert_eq!(close_active_command_title(true), "Fermer le panneau actif");
}
