#[allow(dead_code)]
#[path = "../src/help_panel.rs"]
mod help_panel;
#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/pane_header.rs"]
mod pane_header;
#[allow(dead_code)]
#[path = "../src/pane_menu_model.rs"]
mod pane_menu_model;
#[allow(dead_code)]
#[path = "../src/shortcuts.rs"]
mod shortcuts;

use eframe::egui;
use help_panel::{HelpPanel, filter_supported_shortcuts, help_footer_text};
use pane_menu_model::{OtherPaneTarget, PaneAgentActivity, PaneMenuAction, PaneMenuLang};
use shortcuts::{AppKeyContext, AppShortcut, KeyInput, route_app_key};

const FULL_SUPPORTED_KEYS: &[&str] = &[
    // Panes
    "Ctrl+Shift+T",
    "Ctrl+Shift+Space",
    "Ctrl+Shift+P",
    "Ctrl+Shift+J",
    "Ctrl+Shift+E",
    "Ctrl+Shift+O",
    "Ctrl+Shift+W",
    "Ctrl+Shift+R",
    "Alt+Arrows",
    "Esc",
    "Ctrl+`",
    // Terminal
    "Ctrl+Shift+F",
    "Ctrl+Shift+C",
    "Ctrl+Shift+V",
    "Ctrl+=",
    "Ctrl+-",
    "Ctrl+0",
    "Shift+PageUp",
    "Shift+PageDown",
    // Workspaces
    "Ctrl+Shift+N",
    "Ctrl+PageUp",
    "Ctrl+PageDown",
    // Editor
    "Ctrl+S",
    "Ctrl+W",
    "Ctrl+F",
    "Ctrl+H",
    "Ctrl+G",
    "Alt+Z",
    "F7 / Shift+F7",
    // App
    "Ctrl+Shift+B",
    "Alt+A",
    "Ctrl+Shift+K",
    "Ctrl+Shift+X",
    "Ctrl+Shift+G",
    "Ctrl+,",
    #[cfg(windows)]
    "Win+H",
    "F1",
];

fn collect_shape_texts(shape: &egui::epaint::Shape, out: &mut Vec<(String, egui::Rect)>) {
    match shape {
        egui::epaint::Shape::Text(t) => {
            out.push((
                t.galley.job.text.clone(),
                egui::Rect::from_min_size(t.pos, t.galley.size()),
            ));
        }
        egui::epaint::Shape::Vec(list) => {
            for inner in list {
                collect_shape_texts(inner, out);
            }
        }
        _ => {}
    }
}

fn painted_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Rect)> {
    let mut out = Vec::new();
    for cs in shapes {
        collect_shape_texts(&cs.shape, &mut out);
    }
    out
}

fn has_painted_text(shapes: &[egui::epaint::ClippedShape], exact: &str) -> bool {
    painted_texts(shapes).iter().any(|(text, _)| text == exact)
}

fn find_painted_text_center(
    shapes: &[egui::epaint::ClippedShape],
    exact: &str,
) -> Option<egui::Pos2> {
    painted_texts(shapes)
        .into_iter()
        .find(|(text, _)| text == exact)
        .map(|(_, rect)| rect.center())
}

#[test]
fn full_supported_help_catalog_renders_all_five_groups_and_footer_in_english_and_french() {
    let en_groups = filter_supported_shortcuts(false, FULL_SUPPORTED_KEYS, "");
    let en_titles: Vec<&str> = en_groups.iter().map(|g| g.title).collect();
    assert_eq!(
        en_titles,
        vec!["Panes", "Terminal", "Workspaces", "Editor", "App"]
    );

    let fr_groups = filter_supported_shortcuts(true, FULL_SUPPORTED_KEYS, "");
    let fr_titles: Vec<&str> = fr_groups.iter().map(|g| g.title).collect();
    assert_eq!(
        fr_titles,
        vec![
            "Panneaux",
            "Terminal",
            "Espaces de travail",
            "Éditeur",
            "Application"
        ]
    );

    for (query, expected) in [
        ("bigger", "Ctrl+="),
        ("smaller", "Ctrl+-"),
        ("reset text", "Ctrl+0"),
        ("previous workspace", "Ctrl+PageUp"),
        ("next workspace", "Ctrl+PageDown"),
        ("new workspace", "Ctrl+Shift+N"),
        ("replace", "Ctrl+H"),
        ("go to line", "Ctrl+G"),
        ("word wrap", "Alt+Z"),
        ("broadcast", "Ctrl+Shift+B"),
        ("waiting for your answer", "Alt+A"),
    ] {
        let hits = filter_supported_shortcuts(false, FULL_SUPPORTED_KEYS, query);
        let keys: Vec<&str> = hits
            .iter()
            .flat_map(|g| g.rows.iter().map(|r| r.keys))
            .collect();
        assert!(
            keys.contains(&expected),
            "query {query:?} expected {expected}, got {keys:?}"
        );
    }

    let ctx = egui::Context::default();
    let mut panel = HelpPanel::new();
    let viewport = egui::vec2(960.0_f32, 900.0_f32);
    for _ in 0..2 {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                egui::Modal::new(egui::Id::new("test-full-help")).show(ctx, |ui| {
                    ui.set_width(880.0_f32);
                    panel.show(ui, false, FULL_SUPPORTED_KEYS)
                });
            },
        );
    }
    let out_en = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::Modal::new(egui::Id::new("test-full-help")).show(ctx, |ui| {
                ui.set_width(880.0_f32);
                panel.show(ui, false, FULL_SUPPORTED_KEYS)
            });
        },
    );
    for heading in ["PANES", "TERMINAL", "WORKSPACES", "EDITOR", "APP"] {
        assert!(
            has_painted_text(&out_en.shapes, heading),
            "missing heading {heading}"
        );
    }
    assert!(has_painted_text(&out_en.shapes, help_footer_text(false)));
}

#[test]
fn app_shortcut_routing_handles_azerty_zoom_workspace_cycling_and_terminal_withholding() {
    let ctx = AppKeyContext::default();
    assert_eq!(
        route_app_key(&KeyInput::new("à", "Digit0").with_ctrl(), ctx),
        AppShortcut::Zoom(0)
    );
    assert_eq!(
        route_app_key(&KeyInput::new("+", "Equal").with_ctrl(), ctx),
        AppShortcut::Zoom(1)
    );
    assert_eq!(
        route_app_key(&KeyInput::new("PageUp", "PageUp").with_ctrl(), ctx),
        AppShortcut::CycleWorkspace(-1)
    );
    assert_eq!(
        route_app_key(&KeyInput::new("PageDown", "PageDown").with_ctrl(), ctx),
        AppShortcut::CycleWorkspace(1)
    );
    assert_eq!(
        route_app_key(&KeyInput::new("N", "KeyN").with_ctrl().with_shift(), ctx),
        AppShortcut::CreateWorkspace
    );
}

#[test]
fn pane_header_menu_renders_facts_send_selection_and_triggers_clicked_action() {
    let ctx = egui::Context::default();
    let state = pane_header::MenuState {
        title: "Ada",
        session_id: Some("abcdef12-3456-7890-abcd-ef1234567890"),
        can_restart: true,
        multiple_panes: true,
        terminal_output: true,
        can_rename: true,
        is_agent: true,
        activity: Some(PaneAgentActivity::Busy),
        worktree: Some(("feat/shortcuts".into(), "C:/repo".into())),
        has_selection: true,
        voice_typing: true,
        other_panes: vec![OtherPaneTarget {
            id: "42".into(),
            title: "Bohr".into(),
            where_hint: "shell".into(),
            is_agent: false,
        }],
        language: PaneMenuLang::En,
        ..Default::default()
    };

    let viewport = egui::vec2(640.0_f32, 760.0_f32);
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = pane_header::show(ui, &state);
            });
        },
    );
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = pane_header::show(ui, &state);
            });
        },
    );

    assert!(has_painted_text(&out.shapes, "working"));
    assert!(has_painted_text(&out.shapes, "feat/shortcuts"));
    assert!(has_painted_text(&out.shapes, "Send selection to"));
    assert!(has_painted_text(&out.shapes, "Bohr"));
    assert!(has_painted_text(&out.shapes, "Reset Terminal"));

    let bohr_pos = find_painted_text_center(&out.shapes, "Bohr").expect("Bohr target rendered");
    let mut picked = None;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
            focused: true,
            events: vec![
                egui::Event::PointerMoved(bohr_pos),
                egui::Event::PointerButton {
                    pos: bohr_pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: bohr_pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                picked = pane_header::show(ui, &state);
            });
        },
    );
    assert_eq!(picked, Some(PaneMenuAction::SendSelectionTo("42".into())));
}
