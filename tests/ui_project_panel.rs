#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[path = "../src/project_panel.rs"]
mod project_panel;
#[allow(dead_code)]
#[path = "../src/shortcuts.rs"]
mod shortcuts;
#[allow(dead_code)]
#[path = "../src/sidebar_model.rs"]
mod sidebar_model;

use eframe::egui::{self, Color32, Key, Pos2, Rect, Shape, Vec2};
use project_panel::{
    ProjectAction, ProjectEntry, ProjectPanel, ProjectPanelInput, SessionEntry, SessionKind,
};

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

fn count_vector_primitives(shape: &Shape) -> (usize, usize) {
    match shape {
        Shape::Path(p) => (usize::from(p.points.len() >= 2), 0),
        Shape::LineSegment { .. } => (1, 0),
        Shape::Circle(c) => (0, usize::from(c.radius > 0.0)),
        Shape::Vec(list) => list.iter().fold((0, 0), |(lp, lc), s| {
            let (p, c) = count_vector_primitives(s);
            (lp + p, lc + c)
        }),
        _ => (0, 0),
    }
}

fn has_circle_with_fill(shapes: &[egui::epaint::ClippedShape], target: Color32) -> bool {
    fn check(shape: &Shape, target: Color32) -> bool {
        match shape {
            Shape::Circle(c) => c.fill == target && c.radius >= 3.5,
            Shape::Vec(list) => list.iter().any(|s| check(s, target)),
            _ => false,
        }
    }
    shapes.iter().any(|cs| check(&cs.shape, target))
}

fn key_event(key: Key, pressed: bool) -> egui::Event {
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

fn sample_catalog() -> (Vec<ProjectEntry>, Vec<SessionEntry>) {
    let projects = vec![
        ProjectEntry {
            id: 10,
            name: "tessel-rust".into(),
            path: "C:/Tessel-rust".into(),
            sessions: vec![
                SessionEntry {
                    id: 101,
                    title: "PowerShell 7".into(),
                    kind: SessionKind::Shell,
                    sleeping: false,
                },
                SessionEntry {
                    id: 102,
                    title: "Native Codex".into(),
                    kind: SessionKind::Chat,
                    sleeping: false,
                },
            ],
        },
        ProjectEntry {
            id: 20,
            name: "dormant-repo".into(),
            path: "C:/repos/dormant-repo".into(),
            sessions: vec![SessionEntry {
                id: 201,
                title: "claude-cli".into(),
                kind: SessionKind::Agent,
                sleeping: true,
            }],
        },
    ];
    let global_sessions = vec![SessionEntry {
        id: 901,
        title: "Scratch Shell".into(),
        kind: SessionKind::Shell,
        sleeping: false,
    }];
    (projects, global_sessions)
}

#[test]
fn top_search_and_add_project_buttons_emit_actions_and_localize_en_fr_without_dummy_controls() {
    let ctx = egui::Context::default();
    let mut panel = ProjectPanel::new();
    let (projects, global_sessions) = sample_catalog();

    let run_sidebar = |panel: &mut ProjectPanel, french: bool, events: Vec<egui::Event>| {
        let mut action = None;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 640.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::SidePanel::left("projects")
                    .exact_width(280.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        action = panel.show(
                            ui,
                            ProjectPanelInput {
                                projects: &projects,
                                global_sessions: &global_sessions,
                                active_project: Some(10),
                                active_session: Some(101),
                                french,
                            },
                        );
                    });
                egui::CentralPanel::default().show(ctx, |_ui| {});
            },
        );
        (action, out.shapes)
    };

    // English pass: verify labels, shortcut keycaps, truthful statuses, and zero dummy buttons.
    let (act_en, shapes_en) = run_sidebar(&mut panel, false, Vec::new());
    assert_eq!(act_en, None);
    assert!(has_painted_text(&shapes_en, "Search"));
    assert!(has_painted_text(&shapes_en, "Ctrl"));
    assert!(has_painted_text(&shapes_en, "Shift"));
    assert!(has_painted_text(&shapes_en, "P"));
    assert!(has_painted_text(&shapes_en, "Projects"));
    assert!(has_painted_text(&shapes_en, "tessel-rust"));
    assert!(has_painted_text(&shapes_en, "C:/Tessel-rust"));
    assert!(has_painted_text(&shapes_en, "PowerShell 7"));
    assert!(has_painted_text(&shapes_en, "Native Codex"));
    assert!(has_painted_text(&shapes_en, "chat"));
    assert!(has_painted_text(&shapes_en, "Sleeping"));
    assert!(has_painted_text(&shapes_en, "Sessions without a project"));

    for forbidden in [
        "+",
        "View activity",
        "Workspace options",
        "New workspace",
        "Hidden Worktrees",
        "Working",
        "Waiting for your reply",
    ] {
        assert!(
            !has_painted_text(&shapes_en, forbidden),
            "unimplemented or glyph fallback text must not appear: {forbidden}"
        );
    }

    // Click Search button.
    let search_rect = find_painted_text_rect(&shapes_en, "Search").expect("Search label rect");
    let (act_search, _) = run_sidebar(&mut panel, false, click_events(search_rect.center()));
    assert_eq!(act_search, Some(ProjectAction::Search));

    // Click Add project (FolderPlus) button aligned on the right of the "Projects" header row.
    let projects_hdr_rect =
        find_painted_text_rect(&shapes_en, "Projects").expect("Projects header rect");
    let add_btn_pos = Pos2::new(262.0, projects_hdr_rect.center().y);
    let (act_open, _) = run_sidebar(&mut panel, false, click_events(add_btn_pos));
    assert_eq!(act_open, Some(ProjectAction::OpenFolder));

    // French pass: verify exact localized strings & keycaps (`Maj`).
    let (act_fr, shapes_fr) = run_sidebar(&mut panel, true, Vec::new());
    assert_eq!(act_fr, None);
    assert!(has_painted_text(&shapes_fr, "Rechercher"));
    assert!(has_painted_text(&shapes_fr, "Ctrl"));
    assert!(has_painted_text(&shapes_fr, "Maj"));
    assert!(has_painted_text(&shapes_fr, "P"));
    assert!(has_painted_text(&shapes_fr, "Projets"));
    assert!(has_painted_text(&shapes_fr, "En veille"));
    assert!(has_painted_text(&shapes_fr, "Sessions sans projet"));
}

#[test]
fn project_cards_and_full_width_session_rows_select_correctly_including_global_sessions() {
    let ctx = egui::Context::default();
    let mut panel = ProjectPanel::new();
    let (projects, global_sessions) = sample_catalog();

    let run_sidebar = |panel: &mut ProjectPanel, events: Vec<egui::Event>| {
        let mut action = None;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 640.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::SidePanel::left("projects")
                    .exact_width(280.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        action = panel.show(
                            ui,
                            ProjectPanelInput {
                                projects: &projects,
                                global_sessions: &global_sessions,
                                active_project: Some(10),
                                active_session: Some(101),
                                french: false,
                            },
                        );
                    });
                egui::CentralPanel::default().show(ctx, |_ui| {});
            },
        );
        (action, out.shapes)
    };

    let (_, shapes) = run_sidebar(&mut panel, Vec::new());

    // Click second project's card body.
    let dormant_rect =
        find_painted_text_rect(&shapes, "dormant-repo").expect("dormant-repo title rect");
    let (act_proj, _) = run_sidebar(&mut panel, click_events(dormant_rect.center()));
    assert_eq!(act_proj, Some(ProjectAction::SelectProject(Some(20))));

    // Click near the FAR RIGHT edge (`x = 258.0`) of session row `102` ("Native Codex")
    // to prove full-row hit-testing rather than narrow label-only hit-testing.
    let codex_rect =
        find_painted_text_rect(&shapes, "Native Codex").expect("Native Codex row rect");
    let right_edge_click = Pos2::new(258.0, codex_rect.center().y);
    let (act_sess, _) = run_sidebar(&mut panel, click_events(right_edge_click));
    assert_eq!(
        act_sess,
        Some(ProjectAction::SelectSession {
            project: Some(10),
            id: 102,
        })
    );

    // Click "Sessions without a project" header body -> SelectProject(None).
    let global_hdr_rect = find_painted_text_rect(&shapes, "Sessions without a project")
        .expect("global sessions header");
    let (act_global_hdr, _) = run_sidebar(&mut panel, click_events(global_hdr_rect.center()));
    assert_eq!(act_global_hdr, Some(ProjectAction::SelectProject(None)));

    // Click near the far right edge of global session `901` ("Scratch Shell").
    let scratch_rect =
        find_painted_text_rect(&shapes, "Scratch Shell").expect("Scratch Shell row rect");
    let (act_global_sess, _) = run_sidebar(
        &mut panel,
        click_events(Pos2::new(258.0, scratch_rect.center().y)),
    );
    assert_eq!(
        act_global_sess,
        Some(ProjectAction::SelectSession {
            project: None,
            id: 901,
        })
    );
}

#[test]
fn collapse_and_expand_persist_by_project_id_and_prune_stale_ids_on_catalog_mutation() {
    let ctx = egui::Context::default();
    let mut panel = ProjectPanel::new();
    let (mut projects, mut global_sessions) = sample_catalog();

    let run_frame = |panel: &mut ProjectPanel,
                     projects: &[ProjectEntry],
                     global_sessions: &[SessionEntry],
                     events: Vec<egui::Event>| {
        let mut action = None;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 640.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::SidePanel::left("projects")
                    .exact_width(280.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        action = panel.show(
                            ui,
                            ProjectPanelInput {
                                projects,
                                global_sessions,
                                active_project: Some(10),
                                active_session: Some(101),
                                french: false,
                            },
                        );
                    });
                egui::CentralPanel::default().show(ctx, |_ui| {});
            },
        );
        (action, out.shapes)
    };

    let (_, shapes0) = run_frame(&mut panel, &projects, &global_sessions, Vec::new());
    assert!(!panel.is_project_collapsed(10));
    assert!(has_painted_text(&shapes0, "PowerShell 7"));

    // Click collapse chevron at the right end of project 10's header row (`x = 260.0`).
    let proj10_rect = find_painted_text_rect(&shapes0, "tessel-rust").expect("tessel-rust rect");
    let collapse_pos = Pos2::new(260.0, proj10_rect.center().y);
    let (act_collapse, _) = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        click_events(collapse_pos),
    );
    assert_eq!(
        act_collapse, None,
        "collapsing a project must not emit SelectProject"
    );
    assert!(panel.is_project_collapsed(10));
    assert_eq!(panel.collapsed_project_count(), 1);

    // Subsequent frame: project 10 stays collapsed (sessions + path hidden), project 20 stays expanded.
    let (_, shapes1) = run_frame(&mut panel, &projects, &global_sessions, Vec::new());
    assert!(!has_painted_text(&shapes1, "PowerShell 7"));
    assert!(!has_painted_text(&shapes1, "C:/Tessel-rust"));
    assert!(has_painted_text(&shapes1, "claude-cli"));

    // Collapse global sessions group too.
    let global_rect =
        find_painted_text_rect(&shapes1, "Sessions without a project").expect("global rect");
    let _ = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        click_events(Pos2::new(260.0, global_rect.center().y)),
    );
    assert!(panel.is_global_collapsed());
    let (_, shapes2) = run_frame(&mut panel, &projects, &global_sessions, Vec::new());
    assert!(!has_painted_text(&shapes2, "Scratch Shell"));

    // Remove project 10 and clear global_sessions: stale collapsed state MUST be pruned automatically.
    projects.retain(|p| p.id != 10);
    global_sessions.clear();
    let _ = run_frame(&mut panel, &projects, &global_sessions, Vec::new());
    assert!(!panel.is_project_collapsed(10));
    assert_eq!(panel.collapsed_project_count(), 0);
    assert!(!panel.is_global_collapsed());

    // Re-adding project ID 10 and a global session starts expanded cleanly.
    let (fresh_projects, fresh_globals) = sample_catalog();
    let (_, shapes3) = run_frame(&mut panel, &fresh_projects, &fresh_globals, Vec::new());
    assert!(!panel.is_project_collapsed(10));
    assert!(!panel.is_global_collapsed());
    assert!(has_painted_text(&shapes3, "PowerShell 7"));
    assert!(has_painted_text(&shapes3, "Scratch Shell"));
}

#[test]
fn keyboard_navigation_is_strictly_focus_scoped_and_never_steals_keys_from_outside_widgets() {
    let ctx = egui::Context::default();
    let mut panel = ProjectPanel::new();
    let (projects, global_sessions) = sample_catalog();
    let outside_id = egui::Id::new("simulated-terminal-pane");

    let run_frame = |panel: &mut ProjectPanel, focus_outside: bool, events: Vec<egui::Event>| {
        let mut action = None;
        let mut central_saw_down = false;
        let mut central_saw_enter = false;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 640.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                if focus_outside {
                    ctx.memory_mut(|m| m.request_focus(outside_id));
                }
                egui::SidePanel::left("projects")
                    .exact_width(280.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        action = panel.show(
                            ui,
                            ProjectPanelInput {
                                projects: &projects,
                                global_sessions: &global_sessions,
                                active_project: Some(10),
                                active_session: Some(101),
                                french: false,
                            },
                        );
                    });
                egui::CentralPanel::default().show(ctx, |ui| {
                    let (_, rect) = ui.allocate_space(Vec2::new(200.0, 80.0));
                    let _ = ui.interact(rect, outside_id, egui::Sense::click());
                    central_saw_down = ui.input(|i| i.key_pressed(Key::ArrowDown));
                    central_saw_enter = ui.input(|i| i.key_pressed(Key::Enter));
                });
            },
        );
        (action, central_saw_down, central_saw_enter)
    };

    // 1. Focus is on `simulated-terminal-pane` in CentralPanel:
    // ArrowDown, ArrowLeft, and Enter MUST pass straight through to CentralPanel untouched!
    let _ = run_frame(&mut panel, true, Vec::new());
    let (act_out1, saw_down, saw_enter) = run_frame(
        &mut panel,
        false,
        vec![
            key_event(Key::ArrowDown, true),
            key_event(Key::ArrowLeft, true),
            key_event(Key::Enter, true),
        ],
    );
    assert_eq!(act_out1, None);
    assert!(
        saw_down,
        "focused terminal/editor must still receive ArrowDown"
    );
    assert!(
        saw_enter,
        "focused terminal/editor must still receive Enter"
    );
    assert!(!panel.is_project_collapsed(10));
    let _ = run_frame(
        &mut panel,
        false,
        vec![
            key_event(Key::ArrowDown, false),
            key_event(Key::ArrowLeft, false),
            key_event(Key::Enter, false),
        ],
    );

    // 2. Request focus into `ProjectPanel` and drive keyboard navigation:
    // `request_focus` focuses the active item (`Session(10, 101)`).
    panel.request_focus(&ctx);
    let _ = run_frame(&mut panel, false, Vec::new());

    // First ArrowDown moves from Session(10, 101) -> Session(10, 102) and consumes the key.
    let (act_d1, saw_down_in_sidebar, _) =
        run_frame(&mut panel, false, vec![key_event(Key::ArrowDown, true)]);
    assert_eq!(act_d1, None);
    assert!(
        !saw_down_in_sidebar,
        "ProjectPanel must consume ArrowDown when focused"
    );
    let _ = run_frame(&mut panel, false, vec![key_event(Key::ArrowDown, false)]);

    // Second ArrowDown moves to Project(20); ArrowUp steps back to Session(10, 102).
    let _ = run_frame(&mut panel, false, vec![key_event(Key::ArrowDown, true)]);
    let _ = run_frame(&mut panel, false, vec![key_event(Key::ArrowDown, false)]);
    let _ = run_frame(&mut panel, false, vec![key_event(Key::ArrowUp, true)]);
    let _ = run_frame(&mut panel, false, vec![key_event(Key::ArrowUp, false)]);

    // Fresh Enter activates Session(10, 102).
    let (act_enter, _, _) = run_frame(&mut panel, false, vec![key_event(Key::Enter, true)]);
    assert_eq!(
        act_enter,
        Some(ProjectAction::SelectSession {
            project: Some(10),
            id: 102,
        })
    );
    // Held Enter repeat on next frame without release must NOT re-fire!
    let (act_repeat, _, _) = run_frame(&mut panel, false, vec![key_event(Key::Enter, true)]);
    assert_eq!(act_repeat, None);
    let _ = run_frame(&mut panel, false, vec![key_event(Key::Enter, false)]);

    // ArrowLeft on Session(10, 102) jumps focus up to parent Project(10);
    // a second ArrowLeft collapses Project(10); ArrowRight expands it again!
    let _ = run_frame(&mut panel, false, vec![key_event(Key::ArrowLeft, true)]);
    let _ = run_frame(&mut panel, false, vec![key_event(Key::ArrowLeft, false)]);
    assert!(!panel.is_project_collapsed(10));

    let (act_proj_enter, _, _) = run_frame(&mut panel, false, vec![key_event(Key::Enter, true)]);
    assert_eq!(act_proj_enter, Some(ProjectAction::SelectProject(Some(10))));
    let _ = run_frame(&mut panel, false, vec![key_event(Key::Enter, false)]);

    let _ = run_frame(&mut panel, false, vec![key_event(Key::ArrowLeft, true)]);
    let _ = run_frame(&mut panel, false, vec![key_event(Key::ArrowLeft, false)]);
    assert!(panel.is_project_collapsed(10));

    let _ = run_frame(&mut panel, false, vec![key_event(Key::ArrowRight, true)]);
    let _ = run_frame(&mut panel, false, vec![key_event(Key::ArrowRight, false)]);
    assert!(!panel.is_project_collapsed(10));

    // 3. When an overlay disables the sidebar (`ui.add_enabled_ui(false, ...)`), even if
    // the sidebar previously held focus, it MUST neither trigger actions nor consume keys!
    let mut disabled_action = None;
    let mut overlay_saw_down = false;
    let mut overlay_saw_left = false;
    let mut overlay_saw_enter = false;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(820.0, 560.0))),
            focused: true,
            events: vec![
                key_event(Key::ArrowDown, true),
                key_event(Key::ArrowLeft, true),
                key_event(Key::Enter, true),
            ],
            ..Default::default()
        },
        |ctx| {
            egui::SidePanel::left("projects")
                .exact_width(280.0)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.add_enabled_ui(false, |ui| {
                        disabled_action = panel.show(
                            ui,
                            ProjectPanelInput {
                                projects: &projects,
                                global_sessions: &global_sessions,
                                active_project: Some(10),
                                active_session: Some(101),
                                french: false,
                            },
                        );
                    });
                });
            egui::CentralPanel::default().show(ctx, |ui| {
                overlay_saw_down = ui.input(|i| i.key_pressed(Key::ArrowDown));
                overlay_saw_left = ui.input(|i| i.key_pressed(Key::ArrowLeft));
                overlay_saw_enter = ui.input(|i| i.key_pressed(Key::Enter));
            });
        },
    );
    assert_eq!(disabled_action, None);
    assert!(!panel.is_project_collapsed(10));
    assert!(
        overlay_saw_down && overlay_saw_left && overlay_saw_enter,
        "disabled ProjectPanel must never consume ArrowDown, ArrowLeft, or Enter"
    );
}

#[test]
fn long_names_paths_and_titles_truncate_cleanly_within_narrow_220px_sidebar_bounds() {
    let ctx = egui::Context::default();
    let mut panel = ProjectPanel::new();

    let long_projects = vec![ProjectEntry {
        id: 1,
        name: "extremely-long-monorepo-workspace-project-name-that-must-truncate-cleanly-with-ellipsis"
            .into(),
        path: "C:/Users/developer/workspaces/organization/deeply/nested/repository/directory/path/that/exceeds/sidebar"
            .into(),
        sessions: vec![SessionEntry {
            id: 11,
            title: "claude-code --resume very-long-agent-conversation-prompt-describing-a-complex-refactoring-task"
                .into(),
            kind: SessionKind::Agent,
            sleeping: false,
        }],
    }];

    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 540.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::SidePanel::left("projects")
                .exact_width(220.0)
                .resizable(false)
                .show(ctx, |ui| {
                    let _ = panel.show(
                        ui,
                        ProjectPanelInput {
                            projects: &long_projects,
                            global_sessions: &[],
                            active_project: Some(1),
                            active_session: Some(11),
                            french: true,
                        },
                    );
                });
            egui::CentralPanel::default().show(ctx, |_ui| {});
        },
    );

    let texts = painted_texts(&out.shapes);
    assert!(!texts.is_empty());
    for (text, rect, clip, rows) in &texts {
        let visible = rect.intersect(*clip);
        if visible.is_positive() {
            assert!(
                visible.max.x <= 226.0,
                "text `{text}` overflowed 220px sidebar bounds: visible={visible:?}"
            );
        }
        assert_eq!(
            *rows, 1,
            "sidebar text `{text}` must stay single-line without wrapping"
        );
    }

    // Empty state renders cleanly in EN and FR without panicking.
    for (french, expected) in [
        (false, "No project open yet."),
        (true, "Aucun projet ouvert pour l'instant."),
    ] {
        let empty_out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 540.0))),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                egui::SidePanel::left("projects")
                    .exact_width(220.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        let _ = panel.show(
                            ui,
                            ProjectPanelInput {
                                projects: &[],
                                global_sessions: &[],
                                active_project: None,
                                active_session: None,
                                french,
                            },
                        );
                    });
                egui::CentralPanel::default().show(ctx, |_ui| {});
            },
        );
        assert!(has_painted_text(&empty_out.shapes, expected));
    }
}

#[test]
fn vector_icons_and_sleeping_moon_render_non_empty_geometry_without_glyph_fallbacks() {
    let ctx = egui::Context::default();
    let mut panel = ProjectPanel::new();
    let (projects, global_sessions) = sample_catalog();

    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 640.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::SidePanel::left("projects")
                .exact_width(280.0)
                .resizable(false)
                .show(ctx, |ui| {
                    let _ = panel.show(
                        ui,
                        ProjectPanelInput {
                            projects: &projects,
                            global_sessions: &global_sessions,
                            active_project: Some(10),
                            active_session: Some(101),
                            french: false,
                        },
                    );
                });
            egui::CentralPanel::default().show(ctx, |_ui| {});
        },
    );

    let (total_paths, total_circles) = out.shapes.iter().fold((0, 0), |(acc_p, acc_c), cs| {
        let (p, c) = count_vector_primitives(&cs.shape);
        (acc_p + p, acc_c + c)
    });

    assert!(
        total_paths >= 12,
        "expected vector paths for Search, FolderPlus, Folder, ChevronDown, Moon, Terminal, and Chat icons; got {total_paths}"
    );
    assert!(
        total_circles >= 3,
        "expected vector circles for Search lens, Active status dot, and live session dots; got {total_circles}"
    );
    assert!(
        has_circle_with_fill(&out.shapes, Color32::from_rgb(0x10, 0xb9, 0x81)),
        "expected active project status dot (#10b981) for live sessions"
    );
}

#[test]
fn tab_and_shift_tab_cycle_cleanly_across_header_list_and_outside_widgets_including_empty_state() {
    let ctx = egui::Context::default();
    let mut panel = ProjectPanel::new();
    let (projects, global_sessions) = sample_catalog();
    let outside_id = egui::Id::new("central-pane-target");

    let tab_press = |shift: bool| {
        let modifiers = if shift {
            egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::NONE
        };
        vec![
            egui::Event::Key {
                key: Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
            egui::Event::Key {
                key: Key::Tab,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers,
            },
        ]
    };

    let run_frame = |panel: &mut ProjectPanel,
                     projs: &[ProjectEntry],
                     globals: &[SessionEntry],
                     events: Vec<egui::Event>| {
        let mut action = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 640.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::SidePanel::left("projects")
                    .exact_width(280.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        action = panel.show(
                            ui,
                            ProjectPanelInput {
                                projects: projs,
                                global_sessions: globals,
                                active_project: projs.first().map(|p| p.id),
                                active_session: projs
                                    .first()
                                    .and_then(|p| p.sessions.first())
                                    .map(|s| s.id),
                                french: false,
                            },
                        );
                    });
                egui::CentralPanel::default().show(ctx, |ui| {
                    let (_, rect) = ui.allocate_space(Vec2::new(180.0, 48.0));
                    let _ = ui.interact(rect, outside_id, egui::Sense::click());
                });
            },
        );
        action
    };

    // Start focused on `outside_id`, then Tab into ProjectPanel:
    // outside -> Search -> Add project -> List (1 stop for all cards/rows!) -> outside.
    let _ = run_frame(&mut panel, &projects, &global_sessions, Vec::new());
    ctx.memory_mut(|m| m.request_focus(outside_id));
    let _ = run_frame(&mut panel, &projects, &global_sessions, Vec::new());
    assert_eq!(ctx.memory(|m| m.focused()), Some(outside_id));

    // Tab 1 -> Search button
    let _ = run_frame(&mut panel, &projects, &global_sessions, tab_press(false));
    let act_search = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        vec![key_event(Key::Enter, true), key_event(Key::Enter, false)],
    );
    assert_eq!(act_search, Some(ProjectAction::Search));

    // Tab 2 -> Add project (`+`) button
    let _ = run_frame(&mut panel, &projects, &global_sessions, tab_press(false));
    let act_open = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        vec![key_event(Key::Enter, true), key_event(Key::Enter, false)],
    );
    assert_eq!(act_open, Some(ProjectAction::OpenFolder));

    // Tab 3 -> Virtualized project/session list container (single Tab stop!)
    let _ = run_frame(&mut panel, &projects, &global_sessions, tab_press(false));
    let act_list = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        vec![key_event(Key::Enter, true), key_event(Key::Enter, false)],
    );
    assert_eq!(
        act_list,
        Some(ProjectAction::SelectSession {
            project: Some(10),
            id: 101,
        })
    );

    // Tab 4 -> Exits ProjectPanel straight to `outside_id` (proving cards/chevrons/rows add 0 extra Tab stops!)
    let _ = run_frame(&mut panel, &projects, &global_sessions, tab_press(false));
    assert_eq!(ctx.memory(|m| m.focused()), Some(outside_id));

    // Shift+Tab reverses cleanly: outside -> List -> Add project -> Search
    let _ = run_frame(&mut panel, &projects, &global_sessions, tab_press(true));
    let act_rev_list = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        vec![key_event(Key::Enter, true), key_event(Key::Enter, false)],
    );
    assert_eq!(
        act_rev_list,
        Some(ProjectAction::SelectSession {
            project: Some(10),
            id: 101,
        })
    );

    let _ = run_frame(&mut panel, &projects, &global_sessions, tab_press(true));
    let act_rev_open = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        vec![key_event(Key::Enter, true), key_event(Key::Enter, false)],
    );
    assert_eq!(act_rev_open, Some(ProjectAction::OpenFolder));

    let _ = run_frame(&mut panel, &projects, &global_sessions, tab_press(true));
    let act_rev_search = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        vec![key_event(Key::Enter, true), key_event(Key::Enter, false)],
    );
    assert_eq!(act_rev_search, Some(ProjectAction::Search));

    // In empty state (`projects = []`, `global_sessions = []`), Tab goes Search -> Add project -> outside_id!
    let _ = run_frame(&mut panel, &[], &[], tab_press(false));
    let act_empty_open = run_frame(
        &mut panel,
        &[],
        &[],
        vec![key_event(Key::Enter, true), key_event(Key::Enter, false)],
    );
    assert_eq!(act_empty_open, Some(ProjectAction::OpenFolder));
    let _ = run_frame(&mut panel, &[], &[], tab_press(false));
    assert_eq!(
        ctx.memory(|m| m.focused()),
        Some(outside_id),
        "empty ProjectPanel must not expose an empty list Tab stop"
    );
}

#[test]
fn focused_row_deletion_held_enter_repeat_popup_guard_and_accesskit_labels_work_cleanly() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut panel = ProjectPanel::new();
    let (mut projects, mut global_sessions) = sample_catalog();
    let popup_anchor_id = egui::Id::new("toolbar-popup-anchor");

    let run_frame = |panel: &mut ProjectPanel,
                     projs: &[ProjectEntry],
                     globals: &[SessionEntry],
                     open_popup: bool,
                     events: Vec<egui::Event>| {
        let mut action = None;
        let mut popup_saw_down = false;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 640.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                if open_popup {
                    egui::Popup::open_id(ctx, popup_anchor_id);
                } else {
                    egui::Popup::close_id(ctx, popup_anchor_id);
                }
                egui::SidePanel::left("projects")
                    .exact_width(280.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        action = panel.show(
                            ui,
                            ProjectPanelInput {
                                projects: projs,
                                global_sessions: globals,
                                active_project: projs.first().map(|p| p.id),
                                active_session: projs
                                    .first()
                                    .and_then(|p| p.sessions.first())
                                    .map(|s| s.id),
                                french: false,
                            },
                        );
                    });
                egui::CentralPanel::default().show(ctx, |ui| {
                    let resp = ui.button("Menu");
                    if open_popup {
                        let _ = egui::Popup::from_response(&resp)
                            .id(popup_anchor_id)
                            .show(|ui| {
                                ui.label("Popup item");
                                popup_saw_down = ui.input(|i| i.key_pressed(Key::ArrowDown));
                            });
                    }
                });
            },
        );
        let labels: Vec<String> = out
            .platform_output
            .accesskit_update
            .as_ref()
            .map(|u| {
                u.nodes
                    .iter()
                    .filter_map(|(_, n)| n.label().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        (action, popup_saw_down, labels, out.shapes)
    };

    // 1. Focus list and verify AccessKit labels exist on list root AND clickable rows/buttons!
    panel.request_focus(&ctx);
    let (_, _, labels0, shapes0) =
        run_frame(&mut panel, &projects, &global_sessions, false, Vec::new());
    for expected in [
        "Search (Ctrl+Shift+P)",
        "Add project",
        "Projects — tessel-rust / PowerShell 7",
        "tessel-rust",
        "Collapse tessel-rust",
        "PowerShell 7",
        "Native Codex",
        "Sessions without a project",
    ] {
        assert!(
            labels0.iter().any(|l| l == expected),
            "expected AccessKit label `{expected}` in {labels0:?}"
        );
    }

    // Verify mouse click on `Native Codex` row still works and updates virtual focus!
    let codex_rect = find_painted_text_rect(&shapes0, "Native Codex").expect("Native Codex rect");
    let (click_act, _, labels_after_click, _) = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        false,
        click_events(codex_rect.center()),
    );
    assert_eq!(
        click_act,
        Some(ProjectAction::SelectSession {
            project: Some(10),
            id: 102,
        })
    );
    assert!(
        labels_after_click
            .iter()
            .any(|l| l == "Projects — tessel-rust / Native Codex"),
        "clicking session row must update list accessible focus label: {labels_after_click:?}"
    );

    // 2. Held Enter: fresh press fires once, OS `repeat: true` AND unreleased hold do NOT re-fire!
    let (enter1, _, _, _) = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        false,
        vec![key_event(Key::Enter, true)],
    );
    assert_eq!(
        enter1,
        Some(ProjectAction::SelectSession {
            project: Some(10),
            id: 102,
        })
    );
    let (enter_os_repeat, _, _, _) = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        false,
        vec![egui::Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(enter_os_repeat, None);
    let (enter_held, _, _, _) = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        false,
        vec![key_event(Key::Enter, true)],
    );
    assert_eq!(enter_held, None);
    let _ = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        false,
        vec![key_event(Key::Enter, false)],
    );

    // 3. Open popup guard: while an `egui::Popup` is open, ProjectPanel must not consume ArrowDown/Enter!
    let _ = run_frame(&mut panel, &projects, &global_sessions, true, Vec::new());
    let (popup_act, popup_saw_down, _, _) = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        true,
        vec![
            key_event(Key::ArrowDown, true),
            key_event(Key::Enter, true),
            key_event(Key::ArrowDown, false),
            key_event(Key::Enter, false),
        ],
    );
    assert_eq!(popup_act, None);
    assert!(
        popup_saw_down,
        "open Popup must receive ArrowDown without ProjectPanel stealing it"
    );
    let _ = run_frame(&mut panel, &projects, &global_sessions, false, Vec::new());

    // 4. Deletion of focused row (`Session(10, 102)`):
    // Focus stays inside Project 10 on sibling `Session(10, 101)` (`PowerShell 7`), NOT lost or reset!
    projects[0].sessions.retain(|s| s.id != 102);
    let (_, _, labels_after_del, _) =
        run_frame(&mut panel, &projects, &global_sessions, false, Vec::new());
    assert!(
        labels_after_del
            .iter()
            .any(|l| l == "Projects — tessel-rust / PowerShell 7"),
        "deleting focused session 102 must recover virtual focus to sibling 101: {labels_after_del:?}"
    );
    let (after_del_enter, _, _, _) = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        false,
        vec![key_event(Key::Enter, true), key_event(Key::Enter, false)],
    );
    assert_eq!(
        after_del_enter,
        Some(ProjectAction::SelectSession {
            project: Some(10),
            id: 101,
        })
    );

    // 5. Deleting ALL projects and global sessions while list holds focus moves focus cleanly
    // to `Add project` (`OpenFolder`) so the keyboard user is never stranded!
    projects.clear();
    global_sessions.clear();
    let _ = run_frame(&mut panel, &projects, &global_sessions, false, Vec::new());
    let (empty_enter, _, _, _) = run_frame(
        &mut panel,
        &projects,
        &global_sessions,
        false,
        vec![key_event(Key::Enter, true), key_event(Key::Enter, false)],
    );
    assert_eq!(empty_enter, Some(ProjectAction::OpenFolder));
}
