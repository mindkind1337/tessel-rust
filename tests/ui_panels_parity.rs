#[allow(dead_code)]
#[path = "../src/empty_state_panel.rs"]
mod empty_state_panel;
#[allow(dead_code)]
#[path = "../src/explorer_scm_model.rs"]
mod explorer_scm_model;
#[allow(dead_code)]
#[path = "../src/explorer_tree.rs"]
mod explorer_tree;
#[allow(dead_code)]
#[path = "../src/i18n.rs"]
mod i18n;
#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/project_panel.rs"]
mod project_panel;
#[allow(dead_code)]
#[path = "../src/shortcuts.rs"]
mod shortcuts;
#[allow(dead_code)]
#[path = "../src/sidebar_model.rs"]
mod sidebar_model;
#[allow(dead_code)]
#[path = "../src/tasks_board_panel.rs"]
mod tasks_board_panel;
#[allow(dead_code)]
#[path = "../src/view_tabs.rs"]
mod view_tabs;

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui::{self, Color32, Event, Key, Modifiers, PointerButton, Pos2, Rect, Shape, Vec2};
use empty_state_panel::{EmptyState, EmptyStateAction, Entry, ShellOption};
use explorer_tree::{ExplorerTree, TreeAction, TreeDirEntry};
use project_panel::{
    ProjectAction, ProjectEntry, ProjectPanel, ProjectPanelInput, SessionEntry, SessionKind,
    SidebarOptions,
};
use sidebar_model::AgentDotState;
use tasks_board_panel::{TaskCard, TasksBoard, TasksBoardAction, TasksBoardInput};
use view_tabs::{Kind, Tab};

fn collect_shape_texts(shape: &Shape, out: &mut Vec<(String, Rect, egui::epaint::Galley)>) {
    match shape {
        Shape::Text(t) => {
            out.push((
                t.galley.job.text.clone(),
                Rect::from_min_size(t.pos, t.galley.size()),
                (*t.galley).clone(),
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

fn painted_texts(
    shapes: &[egui::epaint::ClippedShape],
) -> Vec<(String, Rect, egui::epaint::Galley)> {
    let mut out = Vec::new();
    for cs in shapes {
        collect_shape_texts(&cs.shape, &mut out);
    }
    out
}

fn find_text_rect(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<Rect> {
    painted_texts(shapes)
        .into_iter()
        .find(|(text, _, _)| text == needle)
        .map(|(_, rect, _)| rect)
}

fn has_text(shapes: &[egui::epaint::ClippedShape], needle: &str) -> bool {
    find_text_rect(shapes, needle).is_some()
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

fn double_click_events(pos: Pos2) -> Vec<Event> {
    let mut ev = click_events(pos);
    ev.extend(click_events(pos));
    ev
}

#[test]
fn project_panel_collapsed_rail_options_branches_and_reorder() {
    let ctx = egui::Context::default();
    let mut panel = ProjectPanel::new();
    let projects = vec![
        ProjectEntry {
            id: 1,
            name: "Tessel Alpha".into(),
            path: "C:/alpha".into(),
            sessions: vec![SessionEntry {
                id: 101,
                title: "Sleeping shell".into(),
                kind: SessionKind::Shell,
                sleeping: true,
            }],
        },
        ProjectEntry {
            id: 2,
            name: "Rust Core".into(),
            path: "C:/rust".into(),
            sessions: vec![SessionEntry {
                id: 202,
                title: "Claude".into(),
                kind: SessionKind::Agent,
                sleeping: false,
            }],
        },
    ];

    // 1. Collapsed 52px rail renders initials and emits ExpandSidebar / SelectProject / OpenFolder.
    let mut rail_action = None;
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(52.0, 500.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::SidePanel::left("rail")
                .exact_width(52.0)
                .resizable(false)
                .show(ctx, |ui| {
                    rail_action = panel.show_collapsed_rail(
                        ui,
                        ProjectPanelInput {
                            projects: &projects,
                            global_sessions: &[],
                            active_project: Some(1),
                            active_session: None,
                            french: false,
                        },
                    );
                });
        },
    );
    assert!(has_text(&out.shapes, "TA"));
    assert!(has_text(&out.shapes, "RC"));
    let rc_rect = find_text_rect(&out.shapes, "RC").unwrap();

    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(52.0, 500.0))),
            focused: true,
            events: click_events(rc_rect.center()),
            ..Default::default()
        },
        |ctx| {
            egui::SidePanel::left("rail")
                .exact_width(52.0)
                .resizable(false)
                .show(ctx, |ui| {
                    rail_action = panel.show_collapsed_rail(
                        ui,
                        ProjectPanelInput {
                            projects: &projects,
                            global_sessions: &[],
                            active_project: Some(1),
                            active_session: None,
                            french: false,
                        },
                    );
                });
        },
    );
    assert_eq!(rail_action, Some(ProjectAction::SelectProject(Some(2))));

    // 2. Expanded panel renders branch badge, hides sleeping rows when configured, sorts by activity,
    // and supports Alt+ArrowDown project reordering when a project card is focused.
    let mut branches = HashMap::new();
    branches.insert(2, "feat/panels".to_owned());
    panel.set_project_branches(branches);

    let mut states = HashMap::new();
    states.insert(202, AgentDotState::Waiting);
    panel.set_session_states(states);
    panel.set_options(SidebarOptions {
        sort_by_activity: true,
        compact_rows: true,
        hide_sleeping: true,
        hide_agent_task_line: false,
    });

    let out_open = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 600.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = panel.show(
                    ui,
                    ProjectPanelInput {
                        projects: &projects,
                        global_sessions: &[],
                        active_project: Some(2),
                        active_session: None,
                        french: false,
                    },
                );
            });
        },
    );
    assert!(has_text(&out_open.shapes, "feat/panels"));
    assert!(
        !has_text(&out_open.shapes, "Sleeping shell"),
        "sleeping session should be hidden when hide_sleeping is enabled"
    );
    let rust_y = find_text_rect(&out_open.shapes, "Rust Core")
        .unwrap()
        .center()
        .y;
    let alpha_y = find_text_rect(&out_open.shapes, "Tessel Alpha")
        .unwrap()
        .center()
        .y;
    assert!(
        rust_y < alpha_y,
        "Waiting agent project should sort above sleeping project when sort_by_activity is true"
    );

    // Disable sort_by_activity, click "Tessel Alpha" to focus it, then press Alt+ArrowDown.
    panel.set_options(SidebarOptions::default());
    let out_unsorted = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 600.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = panel.show(
                    ui,
                    ProjectPanelInput {
                        projects: &projects,
                        global_sessions: &[],
                        active_project: Some(1),
                        active_session: None,
                        french: false,
                    },
                );
            });
        },
    );
    let alpha_rect = find_text_rect(&out_unsorted.shapes, "Tessel Alpha").unwrap();
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 600.0))),
            focused: true,
            events: click_events(alpha_rect.center()),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = panel.show(
                    ui,
                    ProjectPanelInput {
                        projects: &projects,
                        global_sessions: &[],
                        active_project: Some(1),
                        active_session: None,
                        french: false,
                    },
                );
            });
        },
    );
    let mut move_action = None;
    let alt = Modifiers {
        alt: true,
        ..Modifiers::NONE
    };
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 600.0))),
            focused: true,
            modifiers: alt,
            events: vec![Event::Key {
                key: Key::ArrowDown,
                physical_key: Some(Key::ArrowDown),
                pressed: true,
                repeat: false,
                modifiers: alt,
            }],
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                move_action = panel.show(
                    ui,
                    ProjectPanelInput {
                        projects: &projects,
                        global_sessions: &[],
                        active_project: Some(1),
                        active_session: None,
                        french: false,
                    },
                );
            });
        },
    );
    assert_eq!(
        move_action,
        Some(ProjectAction::MoveProject { id: 1, delta: 1 })
    );
}

#[test]
fn tasks_board_inline_rename_and_done_bulk_selection() {
    let ctx = egui::Context::default();
    let mut board = TasksBoard::new();
    let tasks = vec![
        TaskCard {
            id: 10,
            title: "Original Card".into(),
            column: 0,
            pane: None,
            timing: String::new(),
            timing_title: String::new(),
        },
        TaskCard {
            id: 30,
            title: "Done A".into(),
            column: 3,
            pane: None,
            timing: String::new(),
            timing_title: String::new(),
        },
        TaskCard {
            id: 31,
            title: "Done B".into(),
            column: 3,
            pane: None,
            timing: String::new(),
            timing_title: String::new(),
        },
    ];

    let run = |ctx: &egui::Context, board: &mut TasksBoard, events: Vec<Event>| {
        let mut action = None;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(920.0, 620.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = board.show(
                        ui,
                        TasksBoardInput {
                            agents: &[],
                            tasks: &tasks,
                            french: false,
                            project_selected: true,
                        },
                    );
                });
            },
        );
        (action, out)
    };

    let (_, out0) = run(&ctx, &mut board, Vec::new());
    let card_rect = find_text_rect(&out0.shapes, "Original Card").unwrap();

    // Double-click enters inline rename; typing + Enter commits TasksBoardAction::Rename.
    let _ = run(&ctx, &mut board, double_click_events(card_rect.center()));
    // Focus frame for TextEdit
    let _ = run(&ctx, &mut board, Vec::new());
    let (rename_action, _) = run(
        &ctx,
        &mut board,
        vec![
            Event::Key {
                key: Key::A,
                physical_key: Some(Key::A),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            },
            Event::Text("Renamed Card".into()),
            Event::Key {
                key: Key::Enter,
                physical_key: Some(Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    assert_eq!(
        rename_action,
        Some(TasksBoardAction::Rename {
            id: 10,
            title: "Renamed Card".into()
        })
    );

    // Done bulk selection: click "Select", then "All", then "Delete (2)".
    let (_, out_select_ready) = run(&ctx, &mut board, Vec::new());
    let select_rect = find_text_rect(&out_select_ready.shapes, "Select").unwrap();
    let (_, out_in_select) = run(&ctx, &mut board, click_events(select_rect.center()));
    let all_rect = find_text_rect(&out_in_select.shapes, "All").unwrap();
    let (_, out_all_checked) = run(&ctx, &mut board, click_events(all_rect.center()));
    let del_rect = find_text_rect(&out_all_checked.shapes, "Delete (2)").unwrap();
    let (bulk_action, _) = run(&ctx, &mut board, click_events(del_rect.center()));
    assert_eq!(
        bulk_action,
        Some(TasksBoardAction::DeleteMany(vec![30, 31]))
    );
}

#[test]
fn explorer_tree_filter_dotfiles_git_badges_and_double_click_pin() {
    let root = PathBuf::from("C:/synthetic-project");
    let src_dir = root.join("src");
    let env_file = root.join(".env");
    let main_file = src_dir.join("main.rs");
    let readme_file = root.join("README.md");

    let mut tree = ExplorerTree::fixture(
        root.clone(),
        vec![
            (
                root.clone(),
                vec![
                    TreeDirEntry {
                        name: "src".into(),
                        path: src_dir.clone(),
                        directory: true,
                    },
                    TreeDirEntry {
                        name: ".env".into(),
                        path: env_file,
                        directory: false,
                    },
                    TreeDirEntry {
                        name: "README.md".into(),
                        path: readme_file,
                        directory: false,
                    },
                ],
            ),
            (
                src_dir.clone(),
                vec![TreeDirEntry {
                    name: "main.rs".into(),
                    path: main_file.clone(),
                    directory: false,
                }],
            ),
        ],
        vec![src_dir],
    );

    let mut badges = HashMap::new();
    badges.insert("src/main.rs".to_owned(), 'M');
    tree.set_git_badges(badges);

    let ctx = egui::Context::default();
    let render = |ctx: &egui::Context, tree: &mut ExplorerTree, events: Vec<Event>| {
        let mut action = None;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(360.0, 520.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = tree.show(ui, Some(root.as_path()), None, true);
                });
            },
        );
        (action, out)
    };

    // Dotfiles visible by default + 'M' git badge rendered on main.rs
    let (_, out0) = render(&ctx, &mut tree, Vec::new());
    assert!(has_text(&out0.shapes, ".env"));
    assert!(has_text(&out0.shapes, "main.rs"));
    assert!(has_text(&out0.shapes, "M"));

    // Double-clicking main.rs emits OpenPinnedFile
    let main_rect = find_text_rect(&out0.shapes, "main.rs").unwrap();
    let (pin_act, _) = render(&ctx, &mut tree, double_click_events(main_rect.center()));
    assert_eq!(pin_act, Some(TreeAction::OpenPinnedFile(main_file)));

    // Hiding dotfiles hides .env
    tree.set_show_dotfiles(false);
    let (_, out_no_dot) = render(&ctx, &mut tree, Vec::new());
    assert!(!has_text(&out_no_dot.shapes, ".env"));

    // Name filter keeps only matching entries
    tree.set_filter_query("read");
    let (_, out_filtered) = render(&ctx, &mut tree, Vec::new());
    assert!(has_text(&out_filtered.shapes, "README.md"));
    assert!(!has_text(&out_filtered.shapes, "main.rs"));

    // Clear filter and collapse_all hides nested main.rs
    tree.set_filter_query("");
    tree.collapse_all();
    let (_, out_collapsed) = render(&ctx, &mut tree, Vec::new());
    assert!(has_text(&out_collapsed.shapes, "src"));
    assert!(!has_text(&out_collapsed.shapes, "main.rs"));
}

#[test]
fn view_tabs_closable_browser_diff_preview_pin_and_deleted_strikethrough() {
    let ctx = egui::Context::default();
    let mut preview_keep_open = false;
    let mut browser_closed = false;

    let out0 = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 80.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    #[cfg(windows)]
                    {
                        let r = view_tabs::show(
                            ui,
                            Tab {
                                title: "Browser",
                                hint: "https://example.com",
                                kind: Kind::Browser,
                                selected: true,
                                dirty: false,
                                closable: true,
                                preview: false,
                                deleted: false,
                                has_file_menu: false,
                            },
                        );
                        browser_closed = r.close;
                    }
                    let r_prev = view_tabs::show(
                        ui,
                        Tab {
                            title: "preview.rs",
                            hint: "C:/preview.rs",
                            kind: Kind::Editor,
                            selected: false,
                            dirty: false,
                            closable: true,
                            preview: true,
                            deleted: false,
                            has_file_menu: true,
                        },
                    );
                    preview_keep_open = r_prev.keep_open;
                    let _ = view_tabs::show(
                        ui,
                        Tab {
                            title: "app.rs (Working tree)",
                            hint: "C:/app.rs",
                            kind: Kind::Diff,
                            selected: false,
                            dirty: false,
                            closable: true,
                            preview: true,
                            deleted: false,
                            has_file_menu: true,
                        },
                    );
                    let _ = view_tabs::show(
                        ui,
                        Tab {
                            title: "removed.ts",
                            hint: "C:/removed.ts",
                            kind: Kind::Editor,
                            selected: false,
                            dirty: true,
                            closable: true,
                            preview: false,
                            deleted: true,
                            has_file_menu: true,
                        },
                    );
                });
            });
        },
    );

    let texts = painted_texts(&out0.shapes);
    let (_, prev_rect, prev_galley) = texts
        .iter()
        .find(|(t, _, _)| t == "preview.rs")
        .expect("preview.rs painted");
    assert!(
        prev_galley.job.sections[0].format.italics,
        "unpinned preview tab must render in italics"
    );

    let (_, _, del_galley) = texts
        .iter()
        .find(|(t, _, _)| t == "removed.ts")
        .expect("removed.ts painted");
    assert!(
        del_galley.job.sections[0].format.strikethrough.width > 0.0,
        "deleted dirty tab must render with strikethrough"
    );
    assert_eq!(
        del_galley.job.sections[0].format.color,
        Color32::from_rgb(0xe0, 0x6c, 0x75)
    );

    // Double-clicking preview.rs emits keep_open = true
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 80.0))),
            focused: true,
            events: double_click_events(prev_rect.center()),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    #[cfg(windows)]
                    let _ = view_tabs::show(
                        ui,
                        Tab {
                            title: "Browser",
                            hint: "https://example.com",
                            kind: Kind::Browser,
                            selected: true,
                            dirty: false,
                            closable: true,
                            preview: false,
                            deleted: false,
                            has_file_menu: false,
                        },
                    );
                    let r_prev = view_tabs::show(
                        ui,
                        Tab {
                            title: "preview.rs",
                            hint: "C:/preview.rs",
                            kind: Kind::Editor,
                            selected: false,
                            dirty: false,
                            closable: true,
                            preview: true,
                            deleted: false,
                            has_file_menu: true,
                        },
                    );
                    preview_keep_open = r_prev.keep_open;
                });
            });
        },
    );
    assert!(preview_keep_open);
    let _ = browser_closed;
}

#[test]
fn empty_state_launcher_fallback_heading_shell_pills_and_remember_checkbox() {
    let ctx = egui::Context::default();
    let mut launcher = EmptyState::default();
    let entries = vec![
        Entry {
            id: "agent:claude".into(),
            label: "Claude Code".into(),
            group: "Agents".into(),
            enabled: true,
        },
        Entry {
            id: "terminal".into(),
            label: "Terminal".into(),
            group: "Terminals & pages".into(),
            enabled: true,
        },
        Entry {
            id: "browser".into(),
            label: "Browser page".into(),
            group: "Terminals & pages".into(),
            enabled: true,
        },
    ];
    let shells = [
        ShellOption {
            id: "powershell",
            label: "PowerShell",
        },
        ShellOption {
            id: "cmd",
            label: "CMD",
        },
        ShellOption {
            id: "wsl",
            label: "WSL",
        },
    ];

    let run = |ctx: &egui::Context, launcher: &mut EmptyState, events: Vec<Event>| {
        let mut act = None;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(820.0, 620.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    act = launcher.show_full(ui, "", "", &entries, &shells, "powershell", false);
                });
            },
        );
        (act, out)
    };

    let (_, out0) = run(&ctx, &mut launcher, Vec::new());
    assert!(
        has_text(&out0.shapes, "No project folder"),
        "empty title must render 'No project folder' fallback heading"
    );
    assert!(has_text(&out0.shapes, "WSL"));
    assert!(has_text(&out0.shapes, "Remember for new projects"));

    let wsl_rect = find_text_rect(&out0.shapes, "WSL").unwrap();
    let (shell_act, _) = run(&ctx, &mut launcher, click_events(wsl_rect.center()));
    assert_eq!(shell_act, Some(EmptyStateAction::SelectShell("wsl".into())));

    let rem_rect = find_text_rect(&out0.shapes, "Remember for new projects").unwrap();
    let (rem_act, _) = run(&ctx, &mut launcher, click_events(rem_rect.center()));
    assert_eq!(rem_act, Some(EmptyStateAction::SetRememberDefault(true)));
}
