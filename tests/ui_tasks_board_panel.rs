#[path = "../src/tasks_board_panel.rs"]
mod tasks_board_panel;

use eframe::egui::{self, Color32, Key, Modifiers, Pos2, Rect, Shape, Vec2};
use tasks_board_panel::{
    MAX_TASK_TITLE_CHARS, TaskCard, TasksBoard, TasksBoardAction, TasksBoardInput,
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

fn has_circle_with_fill(shapes: &[egui::epaint::ClippedShape], target: Color32) -> bool {
    fn walk(shape: &Shape, target: Color32) -> bool {
        match shape {
            Shape::Circle(c) => c.fill == target && c.radius >= 3.5,
            Shape::Vec(list) => list.iter().any(|s| walk(s, target)),
            _ => false,
        }
    }
    shapes.iter().any(|cs| walk(&cs.shape, target))
}

fn has_left_stripe_with_fill(shapes: &[egui::epaint::ClippedShape], target: Color32) -> bool {
    fn walk(shape: &Shape, target: Color32) -> bool {
        match shape {
            Shape::Rect(r) => r.fill == target && r.rect.width() <= 4.5 && r.rect.height() >= 20.0,
            Shape::Vec(list) => list.iter().any(|s| walk(s, target)),
            _ => false,
        }
    }
    shapes.iter().any(|cs| walk(&cs.shape, target))
}

fn count_vector_lines_and_paths(shapes: &[egui::epaint::ClippedShape]) -> usize {
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

fn sample_tasks() -> Vec<TaskCard> {
    vec![
        TaskCard {
            id: 1,
            title: "Audit reference CSS".into(),
            column: 0,
            timing: String::new(),
            timing_title: String::new(),
        },
        TaskCard {
            id: 2,
            title: "Build native TasksBoard".into(),
            column: 1,
            timing: String::new(),
            timing_title: String::new(),
        },
        TaskCard {
            id: 3,
            title: "Verify Clippy zero warnings".into(),
            column: 2,
            timing: String::new(),
            timing_title: String::new(),
        },
    ]
}

#[test]
fn renders_four_columns_exact_colors_counts_empty_placeholders_and_no_invented_fields() {
    let ctx = egui::Context::default();
    let mut board = TasksBoard::new();
    let tasks = sample_tasks();

    // 1. No project selected -> localized message in EN and FR, zero actions.
    for (french, expected) in [
        (false, "Select a project first."),
        (true, "Sélectionnez d’abord un projet."),
    ] {
        let mut action = None;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                focused: true,
                events: vec![key_event(Key::Enter, true, Modifiers::NONE)],
                ..Default::default()
            },
            |ctx| {
                egui::SidePanel::right("tasks")
                    .exact_width(280.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        action = board.show(
                            ui,
                            TasksBoardInput {
                                tasks: &tasks,
                                french,
                                project_selected: false,
                            },
                        );
                    });
                egui::CentralPanel::default().show(ctx, |_ui| {});
            },
        );
        assert_eq!(action, None);
        assert!(has_painted_text(&out.shapes, expected));
        assert!(!has_painted_text(&out.shapes, "To do"));
        assert!(!has_painted_text(&out.shapes, "À faire"));
    }

    // 2. English active project: 4 columns, exact `--col` dots & card stripes, `"No tasks"` in Done.
    let out_en = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 720.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::SidePanel::right("tasks")
                .exact_width(300.0)
                .resizable(false)
                .show(ctx, |ui| {
                    let _ = board.show(
                        ui,
                        TasksBoardInput {
                            tasks: &tasks,
                            french: false,
                            project_selected: true,
                        },
                    );
                });
            egui::CentralPanel::default().show(ctx, |_ui| {});
        },
    );

    for heading in ["To do", "Doing", "Review", "Done", "Add", "No tasks"] {
        assert!(
            has_painted_text(&out_en.shapes, heading),
            "missing EN label `{heading}`"
        );
    }
    // Zero ASCII arrow glyphs (`←` / `→`) and zero invented fields (`Unassigned`, `Priority`, etc.).
    for forbidden in ["←", "→", "Unassigned", "No agent", "Priority", "New task…"] {
        assert!(
            !has_painted_text(&out_en.shapes, forbidden),
            "unexpected forbidden/invented text `{forbidden}`"
        );
    }

    // Exact `--col` status dots (`style.css:1474-1485`):
    for col_hex in [
        Color32::from_rgb(0x8b, 0x93, 0xa1), // todo
        Color32::from_rgb(0x6c, 0x9c, 0xff), // doing
        Color32::from_rgb(0xf5, 0xa5, 0x24), // review
        Color32::from_rgb(0x3d, 0xdc, 0x84), // done
    ] {
        assert!(
            has_circle_with_fill(&out_en.shapes, col_hex),
            "missing column header dot {col_hex:?}"
        );
    }
    // Non-empty columns (`0`, `1`, `2`) render 3px left card stripes in `--col` color.
    for stripe_col in [
        Color32::from_rgb(0x8b, 0x93, 0xa1),
        Color32::from_rgb(0x6c, 0x9c, 0xff),
        Color32::from_rgb(0xf5, 0xa5, 0x24),
    ] {
        assert!(
            has_left_stripe_with_fill(&out_en.shapes, stripe_col),
            "missing 3px left card accent stripe {stripe_col:?}"
        );
    }
    assert!(
        count_vector_lines_and_paths(&out_en.shapes) >= 24,
        "expected vector chevrons and dashed empty-column outline"
    );

    // 3. French localization (`tasks.json`):
    let out_fr = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 720.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::SidePanel::right("tasks")
                .exact_width(300.0)
                .resizable(false)
                .show(ctx, |ui| {
                    let _ = board.show(
                        ui,
                        TasksBoardInput {
                            tasks: &tasks,
                            french: true,
                            project_selected: true,
                        },
                    );
                });
            egui::CentralPanel::default().show(ctx, |_ui| {});
        },
    );
    for fr_heading in [
        "À faire",
        "En cours",
        "À examiner",
        "Terminé",
        "Ajouter",
        "Aucune tâche",
    ] {
        assert!(
            has_painted_text(&out_fr.shapes, fr_heading),
            "missing FR label `{fr_heading}`"
        );
    }
}

#[test]
fn adding_tasks_trims_whitespace_refuses_blank_clamps_to_1000_chars_and_clears_draft() {
    let ctx = egui::Context::default();
    let mut board = TasksBoard::new();
    let tasks = sample_tasks();

    let run_frame = |board: &mut TasksBoard, events: Vec<egui::Event>| {
        let mut action = None;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(880.0, 640.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::SidePanel::right("tasks")
                    .exact_width(280.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        action = board.show(
                            ui,
                            TasksBoardInput {
                                tasks: &tasks,
                                french: false,
                                project_selected: true,
                            },
                        );
                    });
                egui::CentralPanel::default().show(ctx, |_ui| {});
            },
        );
        (action, out.shapes)
    };

    // Focus input; pressing Enter on empty or whitespace-only draft is refused!
    let (_, shapes0) = run_frame(&mut board, Vec::new());
    let add_btn_rect = find_painted_text_rect(&shapes0, "Add").expect("Add button text");

    board.focus_input(&ctx);
    let _ = run_frame(&mut board, Vec::new());
    let (act_empty_enter, _) = run_frame(
        &mut board,
        vec![
            key_event(Key::Enter, true, Modifiers::NONE),
            key_event(Key::Enter, false, Modifiers::NONE),
        ],
    );
    assert_eq!(act_empty_enter, None);

    board.set_draft("    \t   ");
    let (act_ws_click, _) = run_frame(&mut board, click_events(add_btn_rect.center()));
    assert_eq!(act_ws_click, None);

    board.focus_input(&ctx);
    let _ = run_frame(&mut board, Vec::new());
    let (act_ws_enter, _) = run_frame(
        &mut board,
        vec![
            key_event(Key::Enter, true, Modifiers::NONE),
            key_event(Key::Enter, false, Modifiers::NONE),
        ],
    );
    assert_eq!(act_ws_enter, None);

    // Valid title via Enter trims outer whitespace, emits `Add`, clears draft, and blocks held Enter!
    board.set_draft("  Wire SettingsNav in app.rs  ");
    let (act_enter, _) = run_frame(
        &mut board,
        vec![key_event(Key::Enter, true, Modifiers::NONE)],
    );
    assert_eq!(
        act_enter,
        Some(TasksBoardAction::Add("Wire SettingsNav in app.rs".into()))
    );
    assert_eq!(board.draft(), "");
    let (act_held_enter, _) = run_frame(
        &mut board,
        vec![key_event(Key::Enter, true, Modifiers::NONE)],
    );
    assert_eq!(act_held_enter, None);
    let _ = run_frame(
        &mut board,
        vec![key_event(Key::Enter, false, Modifiers::NONE)],
    );

    // Oversized 1300-char title is strictly clamped to `MAX_TASK_TITLE_CHARS` (1000 chars) on click!
    let long_input = "x".repeat(1300);
    board.set_draft(long_input);
    assert_eq!(board.draft().chars().count(), MAX_TASK_TITLE_CHARS);
    let (act_click_add, _) = run_frame(&mut board, click_events(add_btn_rect.center()));
    match act_click_add {
        Some(TasksBoardAction::Add(submitted)) => {
            assert_eq!(submitted.chars().count(), MAX_TASK_TITLE_CHARS);
        }
        other => panic!("expected Add(1000 chars), got {other:?}"),
    }
    assert_eq!(board.draft(), "");
}

#[test]
fn moving_tasks_respects_0_to_3_bounds_via_vector_buttons_drag_drop_and_out_of_range_clamping() {
    let ctx = egui::Context::default();
    let mut board = TasksBoard::new();
    let tasks = vec![
        TaskCard {
            id: 10,
            title: "First todo card".into(),
            column: 0,
            timing: String::new(),
            timing_title: String::new(),
        },
        TaskCard {
            id: 40,
            title: "Corrupt column card".into(),
            column: 99,
            timing: String::new(),
            timing_title: String::new(), // clamped to column 3 (Done)
        },
    ];

    let popup_id = egui::Id::new("drag-interrupt-popup");
    let run_frame_ex = |board: &mut TasksBoard,
                        window_focused: bool,
                        open_popup: bool,
                        events: Vec<egui::Event>| {
        let mut action = None;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 760.0))),
                focused: window_focused,
                events,
                ..Default::default()
            },
            |ctx| {
                if open_popup {
                    egui::Popup::open_id(ctx, popup_id);
                } else {
                    egui::Popup::close_id(ctx, popup_id);
                }
                egui::SidePanel::left("tasks")
                    .exact_width(280.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        action = board.show(
                            ui,
                            TasksBoardInput {
                                tasks: &tasks,
                                french: false,
                                project_selected: true,
                            },
                        );
                    });
                egui::CentralPanel::default().show(ctx, |ui| {
                    let resp = ui.button("Popup anchor");
                    if open_popup {
                        let _ = egui::Popup::from_response(&resp).id(popup_id).show(|ui| {
                            ui.label("Interrupting popup");
                        });
                    }
                });
            },
        );
        (action, out.shapes)
    };
    let run_frame =
        |board: &mut TasksBoard, events: Vec<egui::Event>| run_frame_ex(board, true, false, events);

    let (_, shapes) = run_frame(&mut board, Vec::new());
    let todo_title = find_painted_text_rect(&shapes, "First todo card").expect("todo card");
    let done_title = find_painted_text_rect(&shapes, "Corrupt column card").expect("done card");

    // Card layout: right button center is near `x = 280 - 12 - 8 - 13 = 247`, left is `x - 30 = 217`,
    // `y` is ~26px below the single-line title top.
    let todo_btn_y = todo_title.max.y + 20.0;
    let done_btn_y = done_title.max.y + 20.0;
    let right_btn_x = 247.0;
    let left_btn_x = 217.0;

    // Boundary check: clicking left button on column 0 (`todo`) must emit `None`!
    let (act_todo_left, _) = run_frame(&mut board, click_events(Pos2::new(left_btn_x, todo_btn_y)));
    assert_eq!(act_todo_left, None);

    // Clicking right button on column 0 (`todo`) moves `10` to column 1 (`Doing`)!
    let (act_todo_right, _) =
        run_frame(&mut board, click_events(Pos2::new(right_btn_x, todo_btn_y)));
    assert_eq!(
        act_todo_right,
        Some(TasksBoardAction::Move { id: 10, column: 1 })
    );

    // Boundary check on clamped column 99 -> 3 (`Done`): clicking right emits `None`, clicking left emits `column: 2`!
    let (act_done_right, _) =
        run_frame(&mut board, click_events(Pos2::new(right_btn_x, done_btn_y)));
    assert_eq!(act_done_right, None);

    let (act_done_left, _) = run_frame(&mut board, click_events(Pos2::new(left_btn_x, done_btn_y)));
    assert_eq!(
        act_done_left,
        Some(TasksBoardAction::Move { id: 40, column: 2 })
    );

    // Drag-and-drop: drag `First todo card` (`col 0`) onto the empty `Review` (`col 2`) column!
    let review_head = find_painted_text_rect(&shapes, "Review").expect("Review header");
    let review_drop_pos = Pos2::new(140.0, review_head.max.y + 20.0);
    let drag_start = todo_title.center();

    let _ = run_frame(
        &mut board,
        vec![
            egui::Event::PointerMoved(drag_start),
            egui::Event::PointerButton {
                pos: drag_start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    let _ = run_frame(
        &mut board,
        vec![egui::Event::PointerMoved(Pos2::new(
            drag_start.x,
            drag_start.y + 24.0,
        ))],
    );
    let _ = run_frame(&mut board, vec![egui::Event::PointerMoved(review_drop_pos)]);
    let (act_drop, _) = run_frame(
        &mut board,
        vec![egui::Event::PointerButton {
            pos: review_drop_pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(act_drop, Some(TasksBoardAction::Move { id: 10, column: 2 }));

    // Opening a `Popup` between drag press and release MUST cancel the drag and emit `None`!
    let _ = run_frame(
        &mut board,
        vec![
            egui::Event::PointerMoved(drag_start),
            egui::Event::PointerButton {
                pos: drag_start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    let _ = run_frame(
        &mut board,
        vec![egui::Event::PointerMoved(Pos2::new(
            drag_start.x,
            drag_start.y + 24.0,
        ))],
    );
    let _ = run_frame_ex(
        &mut board,
        true,
        true,
        vec![egui::Event::PointerMoved(review_drop_pos)],
    );
    let (act_popup_drop, _) = run_frame_ex(
        &mut board,
        true,
        true,
        vec![egui::Event::PointerButton {
            pos: review_drop_pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(
        act_popup_drop, None,
        "popup opened mid-drag must cancel drag and block Move on release"
    );
    let _ = run_frame_ex(&mut board, true, false, Vec::new());

    // Losing window focus (`focused: false`) mid-drag MUST also cancel the drag and emit `None`!
    let _ = run_frame(
        &mut board,
        vec![
            egui::Event::PointerMoved(drag_start),
            egui::Event::PointerButton {
                pos: drag_start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    let _ = run_frame(
        &mut board,
        vec![egui::Event::PointerMoved(Pos2::new(
            drag_start.x,
            drag_start.y + 24.0,
        ))],
    );
    let _ = run_frame_ex(
        &mut board,
        false,
        false,
        vec![egui::Event::PointerMoved(review_drop_pos)],
    );
    let (act_unfocused_drop, _) = run_frame_ex(
        &mut board,
        true,
        false,
        vec![egui::Event::PointerButton {
            pos: review_drop_pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(
        act_unfocused_drop, None,
        "losing window focus mid-drag must cancel drag even after focus returns on release"
    );
}

#[test]
fn keyboard_navigation_is_strictly_focus_scoped_and_respects_disabled_popup_and_tab_order() {
    let ctx = egui::Context::default();
    let mut board = TasksBoard::new();
    let mut tasks = sample_tasks();
    let outside_id = egui::Id::new("terminal-pane-focus");
    let popup_id = egui::Id::new("test-popup");

    let run_frame = |board: &mut TasksBoard,
                     cur_tasks: &[TaskCard],
                     enabled: bool,
                     open_popup: bool,
                     events: Vec<egui::Event>| {
        let mut action = None;
        let mut outside_saw_arrows = false;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 680.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                if open_popup {
                    egui::Popup::open_id(ctx, popup_id);
                } else {
                    egui::Popup::close_id(ctx, popup_id);
                }
                egui::SidePanel::right("tasks")
                    .exact_width(280.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.add_enabled_ui(enabled, |ui| {
                            action = board.show(
                                ui,
                                TasksBoardInput {
                                    tasks: cur_tasks,
                                    french: false,
                                    project_selected: true,
                                },
                            );
                        });
                    });
                egui::CentralPanel::default().show(ctx, |ui| {
                    let resp = ui.button("Anchor");
                    let (_, r) = ui.allocate_space(Vec2::new(160.0, 60.0));
                    let _ = ui.interact(r, outside_id, egui::Sense::click());
                    outside_saw_arrows = ui
                        .input(|i| i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::ArrowRight));
                    if open_popup {
                        let _ = egui::Popup::from_response(&resp).id(popup_id).show(|ui| {
                            ui.label("Open menu");
                        });
                    }
                });
            },
        );
        (action, outside_saw_arrows)
    };

    // 1. Focus on `outside_id`: ArrowDown, ArrowRight, Alt+ArrowRight, Enter must pass straight through!
    let _ = run_frame(&mut board, &tasks, true, false, Vec::new());
    ctx.memory_mut(|m| m.request_focus(outside_id));
    let _ = run_frame(&mut board, &tasks, true, false, Vec::new());

    let (act_ext, saw_ext) = run_frame(
        &mut board,
        &tasks,
        true,
        false,
        vec![
            key_event(Key::ArrowDown, true, Modifiers::NONE),
            key_event(Key::ArrowRight, true, Modifiers::ALT),
            key_event(Key::Enter, true, Modifiers::NONE),
            key_event(Key::ArrowDown, false, Modifiers::NONE),
            key_event(Key::ArrowRight, false, Modifiers::ALT),
            key_event(Key::Enter, false, Modifiers::NONE),
        ],
    );
    assert_eq!(act_ext, None);
    assert!(
        saw_ext,
        "external terminal/editor must still receive arrows"
    );

    // 2. Focus add input, then Tab once (since draft is empty, Add button is skipped) -> focuses card list!
    board.focus_input(&ctx);
    let _ = run_frame(&mut board, &tasks, true, false, Vec::new());
    let _ = run_frame(
        &mut board,
        &tasks,
        true,
        false,
        vec![
            key_event(Key::Tab, true, Modifiers::NONE),
            key_event(Key::Tab, false, Modifiers::NONE),
        ],
    );
    // Run the immediate repaint frame requested by `egui` when `Tab` moves focus to `list_id`
    // so `list_id` installs its `EventFilter` just as `eframe` does on repaint.
    let _ = run_frame(&mut board, &tasks, true, false, Vec::new());

    // Card 1 (`col 0`) is focused: Alt+ArrowLeft at col 0 emits None; Alt+ArrowRight emits Move { id: 1, column: 1 }!
    let (act_bound_left, _) = run_frame(
        &mut board,
        &tasks,
        true,
        false,
        vec![
            key_event(Key::ArrowLeft, true, Modifiers::ALT),
            key_event(Key::ArrowLeft, false, Modifiers::ALT),
        ],
    );
    assert_eq!(act_bound_left, None);

    let (act_move_r, _) = run_frame(
        &mut board,
        &tasks,
        true,
        false,
        vec![key_event(Key::ArrowRight, true, Modifiers::ALT)],
    );
    assert_eq!(
        act_move_r,
        Some(TasksBoardAction::Move { id: 1, column: 1 })
    );
    tasks[0].column = 1;
    // Held ArrowRight on next frame without release must NOT skip to column 2!
    let (act_repeat_r, _) = run_frame(
        &mut board,
        &tasks,
        true,
        false,
        vec![key_event(Key::ArrowRight, true, Modifiers::ALT)],
    );
    assert_eq!(act_repeat_r, None);
    let _ = run_frame(
        &mut board,
        &tasks,
        true,
        false,
        vec![key_event(Key::ArrowRight, false, Modifiers::ALT)],
    );

    // ArrowDown steps to card 2 (`id = 2`), Alt+ArrowLeft moves it from col 1 -> col 0!
    let _ = run_frame(
        &mut board,
        &tasks,
        true,
        false,
        vec![
            key_event(Key::ArrowDown, true, Modifiers::NONE),
            key_event(Key::ArrowDown, false, Modifiers::NONE),
        ],
    );
    let (act_card2_l, _) = run_frame(
        &mut board,
        &tasks,
        true,
        false,
        vec![
            key_event(Key::ArrowLeft, true, Modifiers::ALT),
            key_event(Key::ArrowLeft, false, Modifiers::ALT),
        ],
    );
    assert_eq!(
        act_card2_l,
        Some(TasksBoardAction::Move { id: 2, column: 0 })
    );

    // 3. Disabled UI (`enabled = false`) and open Popup (`open_popup = true`) never consume keys!
    let (act_disabled, saw_when_disabled) = run_frame(
        &mut board,
        &tasks,
        false,
        false,
        vec![
            key_event(Key::ArrowRight, true, Modifiers::NONE),
            key_event(Key::ArrowRight, false, Modifiers::NONE),
        ],
    );
    assert_eq!(act_disabled, None);
    assert!(saw_when_disabled);

    let _ = run_frame(&mut board, &tasks, true, true, Vec::new());
    let (act_popup, saw_when_popup) = run_frame(
        &mut board,
        &tasks,
        true,
        true,
        vec![
            key_event(Key::ArrowRight, true, Modifiers::NONE),
            key_event(Key::ArrowRight, false, Modifiers::NONE),
        ],
    );
    assert_eq!(act_popup, None);
    assert!(saw_when_popup);
}

#[test]
fn dynamic_catalog_mutation_deleted_focused_card_and_reset_stay_consistent() {
    let ctx = egui::Context::default();
    let mut board = TasksBoard::new();
    let mut tasks = sample_tasks(); // ids: 1 (col 0), 2 (col 1), 3 (col 2)

    let run_frame = |board: &mut TasksBoard, cur: &[TaskCard], events: Vec<egui::Event>| {
        let mut action = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 640.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::SidePanel::right("tasks")
                    .exact_width(280.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        action = board.show(
                            ui,
                            TasksBoardInput {
                                tasks: cur,
                                french: false,
                                project_selected: true,
                            },
                        );
                    });
                egui::CentralPanel::default().show(ctx, |_ui| {});
            },
        );
        action
    };

    // Focus list and select middle card `id = 2` (`idx = 1`).
    board.focus_input(&ctx);
    let _ = run_frame(&mut board, &tasks, Vec::new());
    let _ = run_frame(
        &mut board,
        &tasks,
        vec![
            key_event(Key::Tab, true, Modifiers::NONE),
            key_event(Key::Tab, false, Modifiers::NONE),
        ],
    );
    let _ = run_frame(
        &mut board,
        &tasks,
        vec![
            key_event(Key::ArrowDown, true, Modifiers::NONE),
            key_event(Key::ArrowDown, false, Modifiers::NONE),
        ],
    );

    // Delete focused card `id = 2`: focus recovers in-place onto `id = 3` (`col 2`),
    // so pressing Alt+ArrowRight moves `id = 3` to `col 3`!
    tasks.retain(|t| t.id != 2);
    let _ = run_frame(&mut board, &tasks, Vec::new());
    let act_after_del = run_frame(
        &mut board,
        &tasks,
        vec![
            key_event(Key::ArrowRight, true, Modifiers::ALT),
            key_event(Key::ArrowRight, false, Modifiers::ALT),
        ],
    );
    assert_eq!(
        act_after_del,
        Some(TasksBoardAction::Move { id: 3, column: 3 })
    );

    // Delete ALL tasks while list holds focus: focus hands off cleanly to `add-input`!
    tasks.clear();
    let _ = run_frame(&mut board, &tasks, Vec::new());
    board.set_draft("Fresh task after clear");
    let act_add = run_frame(
        &mut board,
        &tasks,
        vec![
            key_event(Key::Enter, true, Modifiers::NONE),
            key_event(Key::Enter, false, Modifiers::NONE),
        ],
    );
    assert_eq!(
        act_add,
        Some(TasksBoardAction::Add("Fresh task after clear".into()))
    );

    board.set_draft("Temporary draft");
    board.reset();
    assert_eq!(board.draft(), "");
}

#[test]
fn narrow_280px_and_wide_920px_layouts_never_overflow_or_force_880px_parent_width() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut board = TasksBoard::new();
    let tasks = vec![
        TaskCard {
            id: 1,
            title: "super_long_unbroken_rust_identifier_that_must_wrap_cleanly_inside_a_280px_side_panel_without_forcing_880px_width".into(),
            column: 0,
            timing: String::new(),
            timing_title: String::new(),
        },
        TaskCard {
            id: 2,
            title: "Vérifier le rendu étroit en français sans débordement horizontal".into(),
            column: 2,
            timing: String::new(),
            timing_title: String::new(),
        },
    ];

    // 1. Narrow 280px right SidePanel: parent width stays strictly ~280px (never 880px!) and no text overflows.
    let mut panel_rect = Rect::NOTHING;
    let out_narrow = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 680.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            let resp = egui::SidePanel::right("tasks_narrow")
                .default_width(280.0)
                .max_width(280.0)
                .resizable(false)
                .show(ctx, |ui| {
                    let _ = board.show(
                        ui,
                        TasksBoardInput {
                            tasks: &tasks,
                            french: true,
                            project_selected: true,
                        },
                    );
                });
            panel_rect = resp.response.rect;
            egui::CentralPanel::default().show(ctx, |_ui| {});
        },
    );

    assert!(
        panel_rect.width() <= 284.0,
        "TasksBoard must never force an 880px parent width; got {}",
        panel_rect.width()
    );
    for (text, rect, clip, _) in painted_texts(&out_narrow.shapes) {
        let vis = rect.intersect(clip);
        if vis.is_positive() && vis.min.x >= panel_rect.min.x - 2.0 {
            assert!(
                vis.max.x <= panel_rect.max.x + 2.0,
                "text `{text}` overflowed 280px panel bounds: vis={vis:?}, panel={panel_rect:?}"
            );
        }
    }

    let labels: Vec<String> = out_narrow
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
    assert!(
        labels.iter().any(|l| l.starts_with("Tableau des tâches —")),
        "expected French AccessKit landmark label in {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l == "Ajouter"),
        "expected French Add button AccessKit label in {labels:?}"
    );

    // 2. Wide panels keep the same stacked sections as the current reference.
    let out_wide = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 680.0))),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = board.show(
                    ui,
                    TasksBoardInput {
                        tasks: &tasks,
                        french: false,
                        project_selected: true,
                    },
                );
            });
        },
    );
    let todo_r = find_painted_text_rect(&out_wide.shapes, "To do").expect("To do");
    let done_r = find_painted_text_rect(&out_wide.shapes, "Done").expect("Done");
    assert!(
        done_r.min.y > todo_r.max.y && (done_r.min.x - todo_r.min.x).abs() <= 2.0,
        "wide layout must preserve stacked status sections: todo={todo_r:?}, done={done_r:?}"
    );
}

#[test]
fn card_dates_wrap_inside_narrow_cards_without_hiding_controls() {
    let ctx = egui::Context::default();
    let mut board = TasksBoard::new();
    let mut tasks = sample_tasks();
    tasks.truncate(1);
    tasks[0].timing = "Commencée 12 nov. 14 h 05 · terminée 13 nov. 15 h 06 · 1 j 1 h".into();
    tasks[0].timing_title = "Ajoutée 12 novembre 2026\nTerminée 13 novembre 2026".into();
    let mut out = None;
    for _ in 0..2 {
        out = Some(ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(280., 800.))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert!(
                        board
                            .show(
                                ui,
                                TasksBoardInput {
                                    tasks: &tasks,
                                    french: true,
                                    project_selected: true
                                }
                            )
                            .is_none()
                    );
                });
            },
        ));
    }
    let texts = painted_texts(&out.unwrap().shapes);
    let (_, rect, clip, rows) = texts
        .iter()
        .find(|(text, _, _, _)| text == &tasks[0].timing)
        .expect("date label is painted");
    assert!(*rows >= 2);
    assert!(rect.min.x >= clip.min.x && rect.max.x <= clip.max.x + 1.0);
    assert!(rect.max.y <= clip.max.y + 1.0);
}
