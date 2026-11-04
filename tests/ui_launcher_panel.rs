#[allow(dead_code)]
#[path = "../src/finder_model.rs"]
mod finder_model;
#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[path = "../src/launcher_panel.rs"]
mod launcher_panel;

use eframe::egui::{self, Color32, LayerId, Order, Pos2, Rect, Shape, Vec2};
use finder_model::{LaunchMenuAction, LaunchMenuItem, LaunchPlacement, LauncherLang};
use launcher_panel::{LauncherEntry, LauncherInput, LauncherPanel, paint_placement_icon};

fn collect_shape_texts(shape: &Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Text(t) => {
            out.push((
                t.galley.job.text.clone(),
                Rect::from_min_size(t.pos, t.galley.size()),
            ));
        }
        Shape::Vec(list) => {
            for inner in list {
                collect_shape_texts(inner, out);
            }
        }
        _ => {}
    }
}

fn painted_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Rect)> {
    let mut out = Vec::new();
    for cs in shapes {
        collect_shape_texts(&cs.shape, &mut out);
    }
    out
}

fn has_painted_text(shapes: &[egui::epaint::ClippedShape], exact: &str) -> bool {
    painted_texts(shapes).iter().any(|(text, _)| text == exact)
}

fn find_painted_text_center(shapes: &[egui::epaint::ClippedShape], exact: &str) -> Option<Pos2> {
    painted_texts(shapes)
        .into_iter()
        .find(|(text, _)| text == exact)
        .map(|(_, rect)| rect.center())
}

fn key_press(key: egui::Key, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat,
        modifiers: egui::Modifiers::NONE,
    }
}

fn key_event(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn click_events(pos: Pos2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

fn sample_entries() -> Vec<LauncherEntry> {
    vec![
        LauncherEntry {
            item: LaunchMenuItem::Shell {
                id: "default".into(),
                name: "PowerShell".into(),
            },
            available: true,
            reason: None,
        },
        LauncherEntry {
            item: LaunchMenuItem::ChatAgent {
                id: "codex".into(),
                name: "Native Codex chat".into(),
            },
            available: false,
            reason: Some("Select a project for native chat.".into()),
        },
        LauncherEntry {
            item: LaunchMenuItem::ChatAgent {
                id: "claude".into(),
                name: "Native Claude chat".into(),
            },
            available: false,
            reason: Some("Select a project for native chat.".into()),
        },
        LauncherEntry {
            item: LaunchMenuItem::Agent {
                id: "claude".into(),
                name: "claude".into(),
            },
            available: true,
            reason: None,
        },
        LauncherEntry {
            item: LaunchMenuItem::Agent {
                id: "kimi".into(),
                name: "kimi".into(),
            },
            available: false,
            reason: Some("not on PATH".into()),
        },
        LauncherEntry {
            item: LaunchMenuItem::Agent {
                id: "codex".into(),
                name: "codex".into(),
            },
            available: true,
            reason: None,
        },
    ]
}

#[test]
fn keyboard_navigation_skips_disabled_entries_wraps_and_respects_enter_and_escape() {
    let ctx = egui::Context::default();
    let mut panel = LauncherPanel::new();
    let entries = sample_entries();
    let placements = [
        LaunchPlacement::Auto,
        LaunchPlacement::Left,
        LaunchPlacement::Right,
        LaunchPlacement::Down,
    ];

    let run_frame = |panel: &mut LauncherPanel, events: Vec<egui::Event>| {
        let mut action = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 560.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = panel.show(
                        ui,
                        LauncherInput {
                            entries: &entries,
                            placements: &placements,
                            target_title: Some("PowerShell"),
                            lang: LauncherLang::En,
                        },
                    );
                });
            },
        );
        action
    };

    assert_eq!(run_frame(&mut panel, vec![]), None);
    assert_eq!(panel.selected_entry_index(), Some(0));

    // ArrowDown skips both disabled ChatAgent entries (1, 2) and lands on `claude` (3):
    assert_eq!(
        run_frame(&mut panel, vec![key_press(egui::Key::ArrowDown, false)]),
        None
    );
    assert_eq!(panel.selected_entry_index(), Some(3));

    // Next ArrowDown skips disabled `kimi` (4) and lands on `codex` (5):
    assert_eq!(
        run_frame(&mut panel, vec![key_press(egui::Key::ArrowDown, false)]),
        None
    );
    assert_eq!(panel.selected_entry_index(), Some(5));

    // Next ArrowDown wraps back to `PowerShell` (0):
    assert_eq!(
        run_frame(&mut panel, vec![key_press(egui::Key::ArrowDown, false)]),
        None
    );
    assert_eq!(panel.selected_entry_index(), Some(0));

    // ArrowUp from `PowerShell` wraps to `codex` (5):
    assert_eq!(
        run_frame(&mut panel, vec![key_press(egui::Key::ArrowUp, false)]),
        None
    );
    assert_eq!(panel.selected_entry_index(), Some(5));

    // Fresh Enter launches `codex` with Auto placement and `in_worktree: false`:
    assert_eq!(
        run_frame(&mut panel, vec![key_press(egui::Key::Enter, false)]),
        Some(LaunchMenuAction::Launch {
            item: LaunchMenuItem::Agent {
                id: "codex".into(),
                name: "codex".into(),
            },
            placement: LaunchPlacement::Auto,
            in_worktree: false,
        })
    );

    // egui derives repeat from held-key state: exercise a real held press,
    // release, then another fresh press instead of setting repeat on a first key.
    assert_eq!(
        run_frame(&mut panel, vec![key_press(egui::Key::Enter, true)]),
        None
    );
    assert_eq!(
        run_frame(
            &mut panel,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]
        ),
        None
    );
    assert!(matches!(
        run_frame(&mut panel, vec![key_press(egui::Key::Enter, false)]),
        Some(LaunchMenuAction::Launch { .. })
    ));

    // Fresh Escape closes:
    assert_eq!(
        run_frame(&mut panel, vec![key_press(egui::Key::Escape, false)]),
        Some(LaunchMenuAction::Close)
    );
}

#[test]
fn pointer_clicks_select_placement_launch_enabled_row_and_refuse_disabled_rows() {
    let ctx = egui::Context::default();
    let mut panel = LauncherPanel::default();
    let entries = sample_entries();
    let placements = [
        LaunchPlacement::Auto,
        LaunchPlacement::Left,
        LaunchPlacement::Right,
        LaunchPlacement::Down,
    ];

    let run_modal =
        |panel: &mut LauncherPanel, pls: &[LaunchPlacement], events: Vec<egui::Event>| {
            let mut action = None;
            let out = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 620.0))),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::Modal::new(egui::Id::new("launcher-modal-test")).show(ctx, |ui| {
                        ui.set_width(360.0);
                        action = panel.show(
                            ui,
                            LauncherInput {
                                entries: &entries,
                                placements: pls,
                                target_title: Some("PowerShell"),
                                lang: LauncherLang::En,
                            },
                        );
                    });
                },
            );
            (action, out.shapes)
        };

    // Frame 0: Modal sizing pass; Frame 1: visible shapes.
    let _ = run_modal(&mut panel, &placements, vec![]);
    let (_, shapes) = run_modal(&mut panel, &placements, vec![]);
    assert!(has_painted_text(&shapes, "Joins the grid: panes re-tile"));

    // Click "Left" placement button -> updates summary to "Opens left of PowerShell":
    let left_btn = find_painted_text_center(&shapes, "Left").expect("Left placement button");
    let (act, _) = run_modal(&mut panel, &placements, click_events(left_btn));
    assert_eq!(act, None);
    let (_, shapes) = run_modal(&mut panel, &placements, vec![]);
    assert!(has_painted_text(&shapes, "Opens left of PowerShell"));

    // Clicking disabled "kimi" row must be refused (`None`, selection stays on index 0):
    let kimi_pos = find_painted_text_center(&shapes, "kimi").expect("kimi disabled row");
    let (act, _) = run_modal(&mut panel, &placements, click_events(kimi_pos));
    assert_eq!(act, None);
    assert_eq!(panel.selected_entry_index(), Some(0));

    // Clicking enabled "claude" row launches with `LaunchPlacement::Left` and `in_worktree: false`:
    let claude_pos = find_painted_text_center(&shapes, "claude").expect("claude enabled row");
    let (act, _) = run_modal(&mut panel, &placements, click_events(claude_pos));
    assert_eq!(
        act,
        Some(LaunchMenuAction::Launch {
            item: LaunchMenuItem::Agent {
                id: "claude".into(),
                name: "claude".into(),
            },
            placement: LaunchPlacement::Left,
            in_worktree: false,
        })
    );

    // `reset()` restores Auto when Auto is offered, while remembering `Left` when Auto is off:
    panel.reset();
    assert_eq!(
        panel.effective_placement(&placements),
        LaunchPlacement::Auto
    );
    assert_eq!(
        panel.effective_placement(&[
            LaunchPlacement::Left,
            LaunchPlacement::Right,
            LaunchPlacement::Down
        ]),
        LaunchPlacement::Left
    );
}

#[test]
fn dynamic_catalog_changes_clamp_selection_and_handle_all_unavailable_gracefully() {
    let ctx = egui::Context::default();
    let mut panel = LauncherPanel::default();

    let run = |panel: &mut LauncherPanel, entries: &[LauncherEntry], events: Vec<egui::Event>| {
        let mut action = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(480.0, 480.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = panel.show(
                        ui,
                        LauncherInput {
                            entries,
                            placements: &[],
                            target_title: None,
                            lang: LauncherLang::En,
                        },
                    );
                });
            },
        );
        action
    };

    let mut catalog = vec![
        LauncherEntry {
            item: LaunchMenuItem::Shell {
                id: "default".into(),
                name: "PowerShell".into(),
            },
            available: true,
            reason: None,
        },
        LauncherEntry {
            item: LaunchMenuItem::Agent {
                id: "claude".into(),
                name: "claude".into(),
            },
            available: true,
            reason: None,
        },
        LauncherEntry {
            item: LaunchMenuItem::Agent {
                id: "codex".into(),
                name: "codex".into(),
            },
            available: true,
            reason: None,
        },
    ];

    run(&mut panel, &catalog, vec![]);
    // Move selection to last entry (`codex`, index 2):
    run(
        &mut panel,
        &catalog,
        vec![
            key_event(egui::Key::ArrowUp, true),
            key_event(egui::Key::ArrowUp, false),
        ],
    );
    assert_eq!(panel.selected_entry_index(), Some(2));

    // Shrink catalog to 2 entries -> clamps cleanly out of index 2:
    catalog.pop();
    run(&mut panel, &catalog, vec![]);
    assert!(matches!(panel.selected_entry_index(), Some(0 | 1)));

    // Explicitly select index 1 (`claude`), then mark index 1 unavailable -> clamps to index 0:
    if panel.selected_entry_index() == Some(0) {
        run(
            &mut panel,
            &catalog,
            vec![
                key_event(egui::Key::ArrowDown, true),
                key_event(egui::Key::ArrowDown, false),
            ],
        );
    }
    assert_eq!(panel.selected_entry_index(), Some(1));
    catalog[1].available = false;
    run(&mut panel, &catalog, vec![]);
    assert_eq!(panel.selected_entry_index(), Some(0));

    // Mark ALL entries unavailable -> selection becomes `None`, keys never panic or launch:
    catalog[0].available = false;
    assert_eq!(
        run(
            &mut panel,
            &catalog,
            vec![
                key_event(egui::Key::ArrowDown, true),
                key_event(egui::Key::ArrowDown, false),
                key_event(egui::Key::Enter, true),
                key_event(egui::Key::Enter, false),
            ],
        ),
        None
    );
    assert_eq!(panel.selected_entry_index(), None);
    assert_eq!(
        run(
            &mut panel,
            &catalog,
            vec![
                key_event(egui::Key::Escape, true),
                key_event(egui::Key::Escape, false),
            ]
        ),
        Some(LaunchMenuAction::Close)
    );
}

#[test]
fn french_localization_and_strict_omission_of_unimplemented_controls() {
    let ctx = egui::Context::default();
    let mut panel = LauncherPanel::default();
    let entries = vec![
        LauncherEntry {
            item: LaunchMenuItem::Shell {
                id: "default".into(),
                name: "PowerShell".into(),
            },
            available: true,
            reason: None,
        },
        LauncherEntry {
            item: LaunchMenuItem::ChatAgent {
                id: "codex".into(),
                name: "Codex (chat)".into(),
            },
            available: true,
            reason: None,
        },
    ];

    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(500.0, 520.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = panel.show(
                    ui,
                    LauncherInput {
                        entries: &entries,
                        placements: &[LaunchPlacement::Down],
                        target_title: Some("Terminal 1"),
                        lang: LauncherLang::Fr,
                    },
                );
            });
        },
    );

    assert!(has_painted_text(&out.shapes, "Dessous"));
    assert!(has_painted_text(&out.shapes, "S'ouvre sous Terminal 1"));
    assert!(has_painted_text(&out.shapes, "TERMINAUX"));
    assert!(has_painted_text(&out.shapes, "CHATS NATIFS"));
    assert!(has_painted_text(&out.shapes, "AGENTS IA"));
    assert!(has_painted_text(
        &out.shapes,
        "Aucun agent IA installé pour l'instant."
    ));
    assert!(has_painted_text(&out.shapes, "Par défaut"));
    assert!(has_painted_text(
        &out.shapes,
        "Installer des agents ou ajouter le vôtre…"
    ));
    assert!(has_painted_text(&out.shapes, "Entrée"));
    assert!(has_painted_text(&out.shapes, "Échap"));

    // Strictly omit unimplemented worktree / accounts controls:
    let all_text: String = painted_texts(&out.shapes)
        .into_iter()
        .map(|(t, _)| t)
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in [
        "Copie séparée",
        "Separate copy",
        "Définir par défaut",
        "Set default",
    ] {
        assert!(
            !all_text.contains(forbidden),
            "unimplemented control leaked into LauncherPanel: {forbidden}"
        );
    }

    // Empty workspace (`placements: &[]`, `target_title: None`) in FR:
    let empty_out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(500.0, 520.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = panel.show(
                    ui,
                    LauncherInput {
                        entries: &entries,
                        placements: &[],
                        target_title: None,
                        lang: LauncherLang::Fr,
                    },
                );
            });
        },
    );
    assert!(has_painted_text(
        &empty_out.shapes,
        "S'ouvre dans cet espace de travail"
    ));
}

#[test]
fn placement_vector_icons_match_reference_svg_shades_and_dividers() {
    let capture = |placement: LaunchPlacement, rect: Rect| -> Vec<Shape> {
        let ctx = egui::Context::default();
        let layer = LayerId::new(Order::Foreground, egui::Id::new("place-icon-probe"));
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(120.0))),
                ..Default::default()
            },
            |ctx| {
                paint_placement_icon(
                    &ctx.layer_painter(layer),
                    rect,
                    placement,
                    Color32::from_rgb(0xd4, 0xd7, 0xdd),
                );
            },
        );
        let mut flat = Vec::new();
        for cs in out.shapes {
            match cs.shape {
                Shape::Vec(v) => flat.extend(v),
                Shape::Noop => {}
                other => flat.push(other),
            }
        }
        flat
    };

    let rect = Rect::from_min_size(Pos2::new(16.0, 16.0), Vec2::splat(16.0));
    for place in [
        LaunchPlacement::Auto,
        LaunchPlacement::Left,
        LaunchPlacement::Right,
        LaunchPlacement::Down,
    ] {
        let shapes = capture(place, rect);
        assert!(shapes.len() >= 2);
        for s in &shapes {
            assert!(!matches!(s, Shape::Text(_)));
            assert!(rect.expand(0.25).contains_rect(s.visual_bounding_rect()));
        }
    }

    let auto_shapes = capture(LaunchPlacement::Auto, rect);
    assert!(
        auto_shapes
            .iter()
            .all(|s| !matches!(s, Shape::Rect(r) if r.fill != Color32::TRANSPARENT))
    );
    assert_eq!(
        auto_shapes
            .iter()
            .filter(|s| matches!(s, Shape::LineSegment { .. }))
            .count(),
        2
    );

    let shaded_rect_of = |place: LaunchPlacement| -> Rect {
        capture(place, rect)
            .into_iter()
            .find_map(|s| match s {
                Shape::Rect(r) if r.fill != Color32::TRANSPARENT => Some(r.rect),
                _ => None,
            })
            .expect("shaded split half")
    };
    assert!(shaded_rect_of(LaunchPlacement::Left).center().x < rect.center().x);
    assert!(shaded_rect_of(LaunchPlacement::Right).center().x > rect.center().x);
    assert!(shaded_rect_of(LaunchPlacement::Down).center().y > rect.center().y);

    assert!(
        capture(
            LaunchPlacement::Auto,
            Rect::from_min_size(Pos2::ZERO, Vec2::ZERO)
        )
        .is_empty()
    );
}

#[test]
fn popup_close_on_click_outside_keeps_open_on_placement_and_disabled_clicks_and_closes_on_launch() {
    let ctx = egui::Context::default();
    let mut panel = LauncherPanel::default();
    let entries = sample_entries();
    let placements = [
        LaunchPlacement::Auto,
        LaunchPlacement::Left,
        LaunchPlacement::Right,
        LaunchPlacement::Down,
    ];
    let mut open = true;

    let run_popup = |panel: &mut LauncherPanel, open: &mut bool, events: Vec<egui::Event>| {
        let mut action = None;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 640.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                if !*open {
                    return;
                }
                let bounds = ctx.content_rect();
                let position = egui::pos2(bounds.center().x - 170.0, bounds.top() + 80.0);
                egui::Popup::new(
                    egui::Id::new("workspace-launcher-popup-test"),
                    ctx.clone(),
                    position,
                    LayerId::background(),
                )
                .open_bool(open)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .width(340.0)
                .show(|ui| {
                    let next = panel.show(
                        ui,
                        LauncherInput {
                            entries: &entries,
                            placements: &placements,
                            target_title: Some("PowerShell"),
                            lang: LauncherLang::En,
                        },
                    );
                    if matches!(
                        next,
                        Some(LaunchMenuAction::Close | LaunchMenuAction::Launch { .. })
                    ) {
                        ui.close();
                    }
                    action = next;
                });
            },
        );
        (action, out.shapes)
    };

    // Frame 0 sizing pass -> Frame 1 visible popup shapes:
    let _ = run_popup(&mut panel, &mut open, vec![]);
    let (_, shapes) = run_popup(&mut panel, &mut open, vec![]);
    assert!(open);

    // Clicking "Below" updates placement WITHOUT closing the Popup:
    let below_pos = find_painted_text_center(&shapes, "Below").expect("Below button in Popup");
    let (act, _) = run_popup(&mut panel, &mut open, click_events(below_pos));
    assert_eq!(act, None);
    assert!(open, "Popup must stay open after clicking placement radio");
    let (_, shapes) = run_popup(&mut panel, &mut open, vec![]);
    assert!(open);
    assert!(has_painted_text(&shapes, "Opens below PowerShell"));

    // Clicking disabled "kimi" keeps Popup open and emits no launch:
    let kimi_pos = find_painted_text_center(&shapes, "kimi").expect("kimi disabled row in Popup");
    let (act, _) = run_popup(&mut panel, &mut open, click_events(kimi_pos));
    assert_eq!(act, None);
    assert!(open, "Popup must stay open when clicking disabled entry");

    // Pressing Enter inside the still-open Popup launches with `LaunchPlacement::Down` and closes:
    let _ = run_popup(
        &mut panel,
        &mut open,
        vec![
            key_event(egui::Key::ArrowDown, true),
            key_event(egui::Key::ArrowDown, false),
        ],
    );
    let (act, _) = run_popup(
        &mut panel,
        &mut open,
        vec![
            key_event(egui::Key::Enter, true),
            key_event(egui::Key::Enter, false),
        ],
    );
    assert_eq!(
        act,
        Some(LaunchMenuAction::Launch {
            item: LaunchMenuItem::Agent {
                id: "claude".into(),
                name: "claude".into(),
            },
            placement: LaunchPlacement::Down,
            in_worktree: false,
        })
    );
    let _ = run_popup(&mut panel, &mut open, vec![]);
    assert!(
        !open,
        "Popup must close after Enter Launch calls ui.close()"
    );

    // Re-open Popup, click enabled "codex" row directly -> launches and closes Popup:
    open = true;
    panel.reset();
    let _ = run_popup(&mut panel, &mut open, vec![]);
    let (_, shapes) = run_popup(&mut panel, &mut open, vec![]);
    assert!(open);
    let codex_pos = find_painted_text_center(&shapes, "codex").expect("codex row in Popup");
    let (act, _) = run_popup(&mut panel, &mut open, click_events(codex_pos));
    assert_eq!(
        act,
        Some(LaunchMenuAction::Launch {
            item: LaunchMenuItem::Agent {
                id: "codex".into(),
                name: "codex".into(),
            },
            placement: LaunchPlacement::Auto,
            in_worktree: false,
        })
    );
    let _ = run_popup(&mut panel, &mut open, vec![]);
    assert!(!open, "Popup must close after clicking enabled row");

    // Re-open Popup and click outside (`(20, 20)`) -> `CloseOnClickOutside` closes it:
    open = true;
    panel.reset();
    let _ = run_popup(&mut panel, &mut open, vec![]);
    let _ = run_popup(&mut panel, &mut open, vec![]);
    assert!(open);
    let _ = run_popup(&mut panel, &mut open, click_events(Pos2::new(20.0, 20.0)));
    let _ = run_popup(&mut panel, &mut open, vec![]);
    assert!(!open, "Popup must close when clicking outside");
}
