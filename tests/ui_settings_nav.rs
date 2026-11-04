#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/settings_model.rs"]
mod settings_model;
#[path = "../src/settings_nav.rs"]
mod settings_nav;

use eframe::egui::{self, Color32, Key, Modifiers, Pos2, Rect, Shape, Vec2};
use icons::Icon;
use settings_model::{MAX_SETTINGS_SEARCH_CHARS, SettingsGroupId, SettingsLang};
use settings_nav::{SettingsNav, SettingsNavAction, SettingsNavEntry, SettingsNavInput};

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

fn find_clear_button_center(shapes: &[egui::epaint::ClippedShape], min_x: f32) -> Option<Pos2> {
    fn walk(shape: &Shape, min_x: f32, pts: &mut Vec<Pos2>) {
        match shape {
            Shape::LineSegment { points, stroke } if stroke.width > 0.0 => {
                let mid = Pos2::new(
                    (points[0].x + points[1].x) * 0.5,
                    (points[0].y + points[1].y) * 0.5,
                );
                if mid.x >= min_x && mid.y <= 90.0 && (points[0].x - points[1].x).abs() > 1.0 {
                    pts.push(mid);
                }
            }
            Shape::Path(p) if p.points.len() >= 2 && p.stroke.width > 0.0 => {
                let mid = Pos2::new(
                    (p.points[0].x + p.points[1].x) * 0.5,
                    (p.points[0].y + p.points[1].y) * 0.5,
                );
                if mid.x >= min_x && mid.y <= 90.0 && (p.points[0].x - p.points[1].x).abs() > 1.0 {
                    pts.push(mid);
                }
            }
            Shape::Vec(list) => {
                for inner in list {
                    walk(inner, min_x, pts);
                }
            }
            _ => {}
        }
    }
    let mut pts = Vec::new();
    for cs in shapes {
        walk(&cs.shape, min_x, &mut pts);
    }
    if pts.is_empty() {
        return None;
    }
    let sx: f32 = pts.iter().map(|p| p.x).sum();
    let sy: f32 = pts.iter().map(|p| p.y).sum();
    let n = pts.len() as f32;
    Some(Pos2::new(sx / n, sy / n))
}

fn count_vector_primitives_with_color(
    shapes: &[egui::epaint::ClippedShape],
    target: Option<Color32>,
) -> usize {
    fn walk(shape: &Shape, target: Option<Color32>) -> usize {
        match shape {
            Shape::LineSegment { stroke, .. } => {
                usize::from(stroke.width > 0.0 && target.is_none_or(|c| stroke.color == c))
            }
            Shape::Path(p) => usize::from(
                p.points.len() >= 2
                    && p.stroke.width > 0.0
                    && target.is_none_or(|c| p.stroke.color == egui::epaint::ColorMode::Solid(c)),
            ),
            Shape::Circle(c) => usize::from(
                c.radius > 0.0 && target.is_none_or(|col| c.stroke.color == col || c.fill == col),
            ),
            Shape::Rect(r) => {
                usize::from(target.is_some_and(|col| r.fill == col && r.rect.width() <= 16.0))
            }
            Shape::Vec(list) => list.iter().map(|s| walk(s, target)).sum(),
            _ => 0,
        }
    }
    shapes.iter().map(|cs| walk(&cs.shape, target)).sum()
}

fn key_event(key: Key, pressed: bool, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
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

fn sample_six_entries(french: bool) -> Vec<SettingsNavEntry> {
    // Deliberately supplied out of canonical group order to verify `SettingsNav` groups
    // strictly via `ALL_SETTINGS_GROUPS` (`AI` -> `Workspace` -> `Workflows`).
    vec![
        SettingsNavEntry {
            id: "ssh".into(),
            title: if french {
                "Hôtes SSH".into()
            } else {
                "SSH hosts".into()
            },
            group: SettingsGroupId::Workflows,
            keywords: "ssh remote distant server serveur".into(),
            icon: Icon::GitBranch,
        },
        SettingsNavEntry {
            id: "appearance".into(),
            title: if french {
                "Apparence".into()
            } else {
                "Appearance".into()
            },
            group: SettingsGroupId::Workspace,
            keywords: "theme thème language langue police font".into(),
            icon: Icon::Layout,
        },
        SettingsNavEntry {
            id: "agents".into(),
            title: if french {
                "Agents en ligne de commande".into()
            } else {
                "Agent CLIs".into()
            },
            group: SettingsGroupId::Ai,
            keywords: "agent codex claude".into(),
            icon: Icon::Terminal,
        },
        SettingsNavEntry {
            id: "passwords".into(),
            title: if french {
                "Mots de passe".into()
            } else {
                "Passwords".into()
            },
            group: SettingsGroupId::Workspace,
            keywords: "vault coffre csv import".into(),
            icon: Icon::Settings,
        },
        SettingsNavEntry {
            id: "tessel-account".into(),
            title: if french {
                "Compte".into()
            } else {
                "Account".into()
            },
            group: SettingsGroupId::Workspace,
            keywords: "login connexion tessel".into(),
            icon: Icon::Share,
        },
        SettingsNavEntry {
            id: "backup".into(),
            title: if french {
                "Sauvegarde des réglages".into()
            } else {
                "Settings backup".into()
            },
            group: SettingsGroupId::Workflows,
            keywords: "export import sauvegarde restore restaurer".into(),
            icon: Icon::Files,
        },
    ]
}

#[test]
fn renders_only_injected_sections_in_canonical_group_order_with_vector_icons_and_pointer_actions() {
    let ctx = egui::Context::default();
    let mut nav = SettingsNav::new();
    let entries = sample_six_entries(false);

    let run_rail = |nav: &mut SettingsNav, selected: &str, events: Vec<egui::Event>| {
        let mut action = None;
        let mut accent = Color32::TRANSPARENT;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 680.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                accent = ctx.style().visuals.selection.stroke.color;
                egui::SidePanel::left("settings-side")
                    .exact_width(250.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        action = nav.show(
                            ui,
                            SettingsNavInput {
                                entries: &entries,
                                selected,
                                lang: SettingsLang::En,
                            },
                        );
                    });
            },
        );
        (action, out.shapes, accent)
    };

    let (act0, shapes0, accent) = run_rail(&mut nav, "appearance", Vec::new());
    assert_eq!(act0, None);
    assert!(has_painted_text(&shapes0, "Back to app"));

    // Canonical group headers: AI < WORKSPACE < WORKFLOWS vertically;
    // empty groups (INSIGHTS, ABOUT) and unimplemented pages MUST NOT appear!
    let ai_rect = find_painted_text_rect(&shapes0, "AI").expect("AI group header");
    let ws_rect = find_painted_text_rect(&shapes0, "WORKSPACE").expect("WORKSPACE group header");
    let wf_rect = find_painted_text_rect(&shapes0, "WORKFLOWS").expect("WORKFLOWS group header");
    assert!(ai_rect.min.y < ws_rect.min.y, "AI must precede WORKSPACE");
    assert!(
        ws_rect.min.y < wf_rect.min.y,
        "WORKSPACE must precede WORKFLOWS"
    );
    assert!(!has_painted_text(&shapes0, "INSIGHTS"));
    assert!(!has_painted_text(&shapes0, "ABOUT"));

    for unimplemented in [
        "Skills",
        "MCP servers",
        "Connections",
        "Orchestration",
        "Voice typing",
        "Automations",
        "Quick commands",
        "Stats & Usage",
        "About & updates",
    ] {
        assert!(
            !has_painted_text(&shapes0, unimplemented),
            "unimplemented page `{unimplemented}` must never be advertised"
        );
    }

    // Vector primitives painted for back arrow, search magnifier, and all 6 section icons,
    // including accent-tinted vector strokes on the active `.set-nav-item.on` ("Appearance").
    assert!(
        count_vector_primitives_with_color(&shapes0, None) >= 14,
        "expected crisp vector geometry for back arrow, search icon, and section icons"
    );
    assert!(
        count_vector_primitives_with_color(&shapes0, Some(accent)) >= 1,
        "selected `.set-nav-item.on` icon must paint with accent color"
    );

    // Click "SSH hosts" -> emits `Select("ssh")`.
    let ssh_rect = find_painted_text_rect(&shapes0, "SSH hosts").expect("SSH hosts item");
    let (act_ssh, _, _) = run_rail(&mut nav, "appearance", click_events(ssh_rect.center()));
    assert_eq!(act_ssh, Some(SettingsNavAction::Select("ssh".into())));

    // Click "Back to app" -> emits `Back`.
    let back_rect = find_painted_text_rect(&shapes0, "Back to app").expect("Back to app button");
    let (act_back, _, _) = run_rail(&mut nav, "ssh", click_events(back_rect.center()));
    assert_eq!(act_back, Some(SettingsNavAction::Back));
}

#[test]
fn bounded_multi_word_en_fr_search_filters_groups_preserves_spaces_and_shows_localized_no_match() {
    let ctx = egui::Context::default();
    let mut nav = SettingsNav::default();
    let fr_entries = sample_six_entries(true);

    let run_fr = |nav: &mut SettingsNav, events: Vec<egui::Event>| {
        let mut action = None;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 680.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::SidePanel::left("settings-side")
                    .exact_width(260.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        action = nav.show(
                            ui,
                            SettingsNavInput {
                                entries: &fr_entries,
                                selected: "appearance",
                                lang: SettingsLang::Fr,
                            },
                        );
                    });
            },
        );
        (action, out.shapes)
    };

    let (_, shapes_init) = run_fr(&mut nav, Vec::new());
    assert!(has_painted_text(&shapes_init, "Retour à l’application"));
    assert!(has_painted_text(&shapes_init, "IA"));
    assert!(has_painted_text(&shapes_init, "ESPACE DE TRAVAIL"));
    assert!(has_painted_text(&shapes_init, "WORKFLOWS"));

    // Press Ctrl+F to focus the search input, then type multi-word query with a real Space key!
    let _ = run_fr(&mut nav, vec![key_event(Key::F, true, Modifiers::CTRL)]);
    let _ = run_fr(&mut nav, vec![key_event(Key::F, false, Modifiers::CTRL)]);
    let (act_type, shapes_filtered) = run_fr(
        &mut nav,
        vec![
            egui::Event::Text("police".into()),
            key_event(Key::Space, true, Modifiers::NONE),
            egui::Event::Text(" ".into()),
            key_event(Key::Space, false, Modifiers::NONE),
            egui::Event::Text("thème".into()),
        ],
    );
    assert_eq!(act_type, None);
    assert_eq!(nav.query(), "police thème");
    assert!(has_painted_text(&shapes_filtered, "ESPACE DE TRAVAIL"));
    assert!(has_painted_text(&shapes_filtered, "Apparence"));
    assert!(!has_painted_text(&shapes_filtered, "IA"));
    assert!(!has_painted_text(&shapes_filtered, "WORKFLOWS"));
    assert!(!has_painted_text(&shapes_filtered, "Mots de passe"));

    // English canonical page title ("appearance") also finds "Apparence" in FR mode.
    nav.set_query("appearance");
    let (_, shapes_en_alias) = run_fr(&mut nav, Vec::new());
    assert!(has_painted_text(&shapes_en_alias, "Apparence"));

    // Oversized query is strictly bounded to `MAX_SETTINGS_SEARCH_CHARS` (256 chars)
    // and displays the localized FR no-match message.
    let huge = "z".repeat(420);
    nav.set_query(huge);
    assert_eq!(nav.query().chars().count(), MAX_SETTINGS_SEARCH_CHARS);
    let (_, shapes_nomatch_fr) = run_fr(&mut nav, Vec::new());
    assert!(
        painted_texts(&shapes_nomatch_fr)
            .iter()
            .any(|(t, _, _, _)| t.starts_with("Aucun paramètre ne correspond à « z")),
        "expected localized French no-match message"
    );

    // Clicking the inline clear (`×`) button at the right end of `.set-search` clears `query`.
    let clear_pos =
        find_clear_button_center(&shapes_nomatch_fr, 180.0).expect("clear search icon geometry");
    let _ = run_fr(&mut nav, click_events(clear_pos));
    assert_eq!(nav.query(), "");
}

#[test]
fn two_step_escape_clears_query_before_closing_and_held_escape_never_double_fires() {
    let ctx = egui::Context::default();
    let mut nav = SettingsNav::new();
    let entries = sample_six_entries(false);

    nav.set_query("ssh");

    let run_esc = |nav: &mut SettingsNav, events: Vec<egui::Event>| {
        let mut action = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(840.0, 600.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::SidePanel::left("settings-side")
                    .exact_width(240.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        action = nav.show(
                            ui,
                            SettingsNavInput {
                                entries: &entries,
                                selected: "appearance",
                                lang: SettingsLang::En,
                            },
                        );
                    });
            },
        );
        action
    };

    // First fresh Escape clears `"ssh"` -> `""` and returns `None`.
    let act1 = run_esc(
        &mut nav,
        vec![key_event(Key::Escape, true, Modifiers::NONE)],
    );
    assert_eq!(act1, None);
    assert_eq!(nav.query(), "");

    // Holding Escape across subsequent frames without release MUST NOT close Settings!
    let act_held_1 = run_esc(
        &mut nav,
        vec![key_event(Key::Escape, true, Modifiers::NONE)],
    );
    assert_eq!(act_held_1, None);
    let act_held_2 = run_esc(
        &mut nav,
        vec![egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(act_held_2, None);

    // Release Escape, then press Escape cleanly -> emits `SettingsNavAction::Back` once!
    let _ = run_esc(
        &mut nav,
        vec![key_event(Key::Escape, false, Modifiers::NONE)],
    );
    let act2 = run_esc(
        &mut nav,
        vec![key_event(Key::Escape, true, Modifiers::NONE)],
    );
    assert_eq!(act2, Some(SettingsNavAction::Back));
    let act2_held = run_esc(
        &mut nav,
        vec![key_event(Key::Escape, true, Modifiers::NONE)],
    );
    assert_eq!(act2_held, None);
}

#[test]
fn external_focus_and_disabled_state_never_steal_keys_while_nav_focus_drives_arrows_and_enter() {
    let ctx = egui::Context::default();
    let mut nav = SettingsNav::new();
    let entries = sample_six_entries(false);
    let body_input_id = egui::Id::new("settings-body-ssh-alias");

    let run_frame =
        |nav: &mut SettingsNav, focus_body: bool, enabled: bool, events: Vec<egui::Event>| {
            let mut action = None;
            let mut body_saw_down = false;
            let mut body_saw_enter = false;
            let mut body_saw_escape = false;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 640.0))),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    if focus_body {
                        ctx.memory_mut(|m| m.request_focus(body_input_id));
                    }
                    egui::SidePanel::left("settings-side")
                        .exact_width(250.0)
                        .resizable(false)
                        .show(ctx, |ui| {
                            ui.add_enabled_ui(enabled, |ui| {
                                action = nav.show(
                                    ui,
                                    SettingsNavInput {
                                        entries: &entries,
                                        selected: "appearance",
                                        lang: SettingsLang::En,
                                    },
                                );
                            });
                        });
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let (_, rect) = ui.allocate_space(Vec2::new(220.0, 32.0));
                        let _ = ui.interact(rect, body_input_id, egui::Sense::click());
                        body_saw_down = ui.input(|i| i.key_pressed(Key::ArrowDown));
                        body_saw_enter = ui.input(|i| i.key_pressed(Key::Enter));
                        body_saw_escape = ui.input(|i| i.key_pressed(Key::Escape));
                    });
                },
            );
            (action, body_saw_down, body_saw_enter, body_saw_escape)
        };

    // 1. Focus is on a control in the Settings page body (`settings-body-ssh-alias`):
    // ArrowDown and Enter MUST pass straight through to the body without triggering nav selection!
    let _ = run_frame(&mut nav, true, true, Vec::new());
    let (act_ext, saw_down, saw_enter, _) = run_frame(
        &mut nav,
        false,
        true,
        vec![
            key_event(Key::ArrowDown, true, Modifiers::NONE),
            key_event(Key::Enter, true, Modifiers::NONE),
        ],
    );
    assert_eq!(act_ext, None);
    assert!(
        saw_down,
        "Settings body widget must still receive ArrowDown"
    );
    assert!(saw_enter, "Settings body widget must still receive Enter");
    let _ = run_frame(
        &mut nav,
        false,
        true,
        vec![
            key_event(Key::ArrowDown, false, Modifiers::NONE),
            key_event(Key::Enter, false, Modifiers::NONE),
        ],
    );

    // 2. Focus search (`focus_search(&ctx)`): canonical visible order is:
    // [0] agents (AI), [1] appearance (Workspace, selected), [2] passwords, [3] tessel-account,
    // [4] ssh (Workflows), [5] backup.
    // Pressing ArrowDown from selected "appearance" moves to "passwords", and Enter selects it!
    nav.focus_search(&ctx);
    let _ = run_frame(&mut nav, false, true, Vec::new());
    let (act_down, saw_down_nav, _, _) = run_frame(
        &mut nav,
        false,
        true,
        vec![key_event(Key::ArrowDown, true, Modifiers::NONE)],
    );
    assert_eq!(act_down, None);
    assert!(
        !saw_down_nav,
        "SettingsNav must consume ArrowDown when focused"
    );
    let _ = run_frame(
        &mut nav,
        false,
        true,
        vec![key_event(Key::ArrowDown, false, Modifiers::NONE)],
    );

    let (act_enter, _, _, _) = run_frame(
        &mut nav,
        false,
        true,
        vec![key_event(Key::Enter, true, Modifiers::NONE)],
    );
    assert_eq!(
        act_enter,
        Some(SettingsNavAction::Select("passwords".into()))
    );
    // Held Enter without release must NOT re-fire!
    let (act_repeat, _, _, _) = run_frame(
        &mut nav,
        false,
        true,
        vec![key_event(Key::Enter, true, Modifiers::NONE)],
    );
    assert_eq!(act_repeat, None);
    let _ = run_frame(
        &mut nav,
        false,
        true,
        vec![key_event(Key::Enter, false, Modifiers::NONE)],
    );

    // 3. When disabled (`ui.add_enabled_ui(false, ...)` while `restore_preview` confirmation is open),
    // even with nav focused and a non-empty query, neither Escape, ArrowDown, nor Enter is consumed!
    nav.set_query("ssh");
    let (act_dis, dis_down, dis_enter, dis_esc) = run_frame(
        &mut nav,
        false,
        false,
        vec![
            key_event(Key::Escape, true, Modifiers::NONE),
            key_event(Key::ArrowDown, true, Modifiers::NONE),
            key_event(Key::Enter, true, Modifiers::NONE),
        ],
    );
    assert_eq!(act_dis, None);
    assert_eq!(
        nav.query(),
        "ssh",
        "disabled SettingsNav must not clear query"
    );
    assert!(
        dis_down && dis_enter && dis_esc,
        "disabled SettingsNav must leave Escape, ArrowDown, and Enter for foreground modals"
    );
}

#[test]
fn dynamic_catalog_mutation_empty_catalog_and_reset_stay_consistent() {
    let ctx = egui::Context::default();
    let mut nav = SettingsNav::new();
    let full = sample_six_entries(false);

    let run_with =
        |nav: &mut SettingsNav, catalog: &[SettingsNavEntry], events: Vec<egui::Event>| {
            let mut action = None;
            let out = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(820.0, 560.0))),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::SidePanel::left("settings-side")
                        .exact_width(240.0)
                        .resizable(false)
                        .show(ctx, |ui| {
                            action = nav.show(
                                ui,
                                SettingsNavInput {
                                    entries: catalog,
                                    selected: "appearance",
                                    lang: SettingsLang::En,
                                },
                            );
                        });
                },
            );
            (action, out.shapes)
        };

    // Focus search and step onto "passwords" (`idx = 2`).
    let _ = run_with(&mut nav, &full, Vec::new());
    nav.focus_search(&ctx);
    let _ = run_with(&mut nav, &full, Vec::new());
    let _ = run_with(
        &mut nav,
        &full,
        vec![key_event(Key::ArrowDown, true, Modifiers::NONE)],
    );
    let _ = run_with(
        &mut nav,
        &full,
        vec![key_event(Key::ArrowDown, false, Modifiers::NONE)],
    );

    // Mutate catalog to remove `passwords` (`idx = 2`) and `tessel-account`: focused item
    // recovers cleanly in-place onto the adjacent surviving item at `idx = 2` (`"ssh"`)!
    let reduced: Vec<SettingsNavEntry> = full
        .into_iter()
        .filter(|e| e.id != "passwords" && e.id != "tessel-account")
        .collect();
    let (act_recover, _) = run_with(
        &mut nav,
        &reduced,
        vec![key_event(Key::Enter, true, Modifiers::NONE)],
    );
    assert_eq!(act_recover, Some(SettingsNavAction::Select("ssh".into())));
    let _ = run_with(
        &mut nav,
        &reduced,
        vec![key_event(Key::Enter, false, Modifiers::NONE)],
    );

    // Empty catalog renders graceful fallback without panicking, and `reset()` clears query.
    nav.set_query("backup");
    assert_eq!(nav.query(), "backup");
    nav.reset();
    assert_eq!(nav.query(), "");
    let (act_empty, shapes_empty) = run_with(&mut nav, &[], Vec::new());
    assert_eq!(act_empty, None);
    assert!(has_painted_text(&shapes_empty, "No settings available."));
}

#[test]
fn narrow_200px_rail_truncates_long_titles_and_headers_within_clip_bounds() {
    let ctx = egui::Context::default();
    let mut nav = SettingsNav::new();
    let long_entries = vec![
        SettingsNavEntry {
            id: "agents".into(),
            title: "Agents en ligne de commande et configuration avancée des exécutables locaux"
                .into(),
            group: SettingsGroupId::Workspace,
            keywords: "agents cli".into(),
            icon: Icon::Terminal,
        },
        SettingsNavEntry {
            id: "backup".into(),
            title: "Sauvegarde et restauration complète des préférences de l’espace de travail"
                .into(),
            group: SettingsGroupId::Workflows,
            keywords: "sauvegarde".into(),
            icon: Icon::Files,
        },
    ];

    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 520.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::SidePanel::left("settings-side")
                .exact_width(200.0)
                .resizable(false)
                .show(ctx, |ui| {
                    let _ = nav.show(
                        ui,
                        SettingsNavInput {
                            entries: &long_entries,
                            selected: "agents",
                            lang: SettingsLang::Fr,
                        },
                    );
                });
        },
    );

    let texts = painted_texts(&out.shapes);
    assert!(!texts.is_empty());
    for (text, _, clip, rows) in &texts {
        assert!(
            *rows <= 1,
            "expected single-line truncated galley for `{text}`, got {rows} rows"
        );
        assert!(
            clip.max.x <= 201.0,
            "expected `{text}` clip rect ({clip:?}) to stay inside the 200px rail"
        );
    }
}

#[test]
fn focus_ownership_distinguishes_settings_search_from_page_body() {
    let ctx = egui::Context::default();
    let mut nav = SettingsNav::new();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = nav.show(
                ui,
                SettingsNavInput {
                    entries: &[],
                    selected: "",
                    lang: SettingsLang::En,
                },
            );
            nav.focus_search(ctx);
            assert!(nav.owns_focus(ctx));
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("page-body-search")));
            assert!(!nav.owns_focus(ctx));
        });
    });
}
