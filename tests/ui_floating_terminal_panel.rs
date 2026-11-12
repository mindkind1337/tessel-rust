//! Standalone headless `egui` + `AccessKit` parity tests for `src/floating_terminal_panel.rs`
//! (slide-down `FloatingTerminal`: `15%..=90%` height bounds with `45%` default and `.10..=.95`
//! drag preview, `±5%` keyboard handle resize, `Hide` chevron vs `Close` (`×`), `More options`
//! floating-only menu, focus capture/restoration lifecycle, `Escape` shell passthrough,
//! `Ctrl+\`` AZERTY/QWERTY + `floatingPaneKey` shortcut filtering, and 7-theme EN/FR parity).

#[path = "../src/floating_terminal_panel.rs"]
mod floating_terminal_panel;
#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/palette.rs"]
mod palette;
#[allow(dead_code)]
#[path = "../src/pane_menu_model.rs"]
mod pane_menu_model;
#[allow(dead_code)]
#[path = "../src/shortcuts.rs"]
mod shortcuts;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Event, Id, Key, Modifiers, PointerButton, Pos2, Rect, Shape, Vec2};
use floating_terminal_panel::{
    FLOATING_DEFAULT_HEIGHT, FLOATING_HANDLE_HEIGHT, FLOATING_HEADER_HEIGHT,
    FLOATING_KEYBOARD_STEP, FLOATING_MAX_HEIGHT, FLOATING_MIN_HEIGHT, FloatingBodyContext,
    FloatingTerminalAction, FloatingTerminalInput, FloatingTerminalOutput, FloatingTerminalPanel,
    FloatingTerminalState, clamp_floating_height,
};
use theme_palettes::{THEMES, ThemeId};

fn collect_shape_texts(shape: &Shape, clip: Rect, out: &mut Vec<(String, Rect, Rect)>) {
    match shape {
        Shape::Text(t) => {
            out.push((
                t.galley.job.text.clone(),
                Rect::from_min_size(t.pos, t.galley.size()),
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

fn painted_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Rect, Rect)> {
    let mut out = Vec::new();
    for cs in shapes {
        collect_shape_texts(&cs.shape, cs.clip_rect, &mut out);
    }
    out
}

fn has_painted_text(shapes: &[egui::epaint::ClippedShape], exact: &str) -> bool {
    painted_texts(shapes)
        .iter()
        .any(|(text, _, _)| text == exact)
}

fn find_painted_text_center(shapes: &[egui::epaint::ClippedShape], exact: &str) -> Option<Pos2> {
    painted_texts(shapes)
        .into_iter()
        .find(|(text, rect, clip)| text == exact && clip.intersects(*rect))
        .map(|(_, rect, clip)| rect.intersect(clip).center())
}

fn pointer_down(pos: Pos2, button: PointerButton) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn pointer_up(pos: Pos2, button: PointerButton) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn key_press(key: Key, physical_key: Option<Key>, modifiers: Modifiers, repeat: bool) -> Event {
    Event::Key {
        key,
        physical_key,
        pressed: true,
        repeat,
        modifiers,
    }
}

fn key_release(key: Key, physical_key: Option<Key>, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key,
        pressed: false,
        repeat: false,
        modifiers,
    }
}

fn key_tap(key: Key, physical_key: Option<Key>, modifiers: Modifiers) -> Vec<Event> {
    vec![
        key_press(key, physical_key, modifiers, false),
        key_release(key, physical_key, modifiers),
    ]
}

fn accesskit_center(out: &egui::FullOutput, role: Role, label: &str) -> Option<Pos2> {
    let update = out.platform_output.accesskit_update.as_ref()?;
    for (_, node) in &update.nodes {
        if node.role() == role
            && node.label() == Some(label)
            && let Some(bounds) = node.bounds()
        {
            return Some(Pos2::new(
                ((bounds.x0 + bounds.x1) * 0.5) as f32,
                ((bounds.y0 + bounds.y1) * 0.5) as f32,
            ));
        }
    }
    None
}

fn accesskit_button_labels(out: &egui::FullOutput) -> Vec<String> {
    let Some(update) = out.platform_output.accesskit_update.as_ref() else {
        return Vec::new();
    };
    update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::Button)
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect()
}

struct StepConfig<'a> {
    viewport: Vec2,
    workspace_rect: Rect,
    title: &'a str,
    has_terminal: bool,
    starting: bool,
    can_restart: bool,
    has_output: bool,
    dialog_open: bool,
    typing_in_field: bool,
    french: bool,
    theme: ThemeId,
    render_outside_btn: bool,
    render_grid_pane: bool,
    render_toolbar_toggle: bool,
}

impl Default for StepConfig<'_> {
    fn default() -> Self {
        Self {
            viewport: Vec2::new(1000.0, 500.0),
            workspace_rect: Rect::from_min_size(Pos2::new(0.0, 40.0), Vec2::new(1000.0, 400.0)),
            title: "PowerShell",
            has_terminal: true,
            starting: false,
            can_restart: true,
            has_output: true,
            dialog_open: false,
            typing_in_field: false,
            french: false,
            theme: ThemeId::Classic,
            render_outside_btn: true,
            render_grid_pane: true,
            render_toolbar_toggle: true,
        }
    }
}

struct StepResult {
    shortcut_actions: Vec<FloatingTerminalAction>,
    panel_out: FloatingTerminalOutput,
    body_ctx: Option<FloatingBodyContext>,
    body_mounts: usize,
    open_terminal_clicked: bool,
    shell_escape_count: usize,
    remaining_ctrl_shift_keys: Vec<Key>,
    remaining_alt_arrows: usize,
    grid_clicked: bool,
    outside_id: Id,
    grid_pane_id: Id,
    content_id: Id,
    toggle_id: Option<Id>,
    full_output: egui::FullOutput,
}

fn run_step(
    ctx: &egui::Context,
    state: &mut FloatingTerminalState,
    cfg: &StepConfig<'_>,
    time: f64,
    events: Vec<Event>,
) -> StepResult {
    let outside_id = Id::new("outside_widget");
    let grid_pane_id = Id::new("grid_pane_underneath");
    let content_id = Id::new("floating_xterm_textarea");

    let mut shortcut_actions = Vec::new();
    let mut panel_out = FloatingTerminalOutput::default();
    let mut body_ctx = None;
    let mut body_mounts = 0_usize;
    let mut open_terminal_clicked = false;
    let mut shell_escape_count = 0_usize;
    let mut remaining_ctrl_shift_keys = Vec::new();
    let mut remaining_alt_arrows = 0_usize;
    let mut grid_clicked = false;
    let mut toggle_id = None;

    let full_output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, cfg.viewport)),
            focused: true,
            time: Some(time),
            events,
            ..Default::default()
        },
        |ctx| {
            body_mounts = 0;
            open_terminal_clicked = false;
            shell_escape_count = 0;
            remaining_ctrl_shift_keys.clear();
            remaining_alt_arrows = 0;
            grid_clicked = false;
            shortcut_actions.extend(FloatingTerminalPanel::handle_shortcuts(
                ctx,
                state,
                cfg.dialog_open,
                cfg.typing_in_field,
            ));

            // Inspect what shortcuts remain visible to `App` after `handle_shortcuts`.
            ctx.input(|inp| {
                for ev in &inp.events {
                    if let Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } = ev
                    {
                        if modifiers.ctrl && modifiers.shift && !modifiers.alt {
                            remaining_ctrl_shift_keys.push(*key);
                        }
                        if modifiers.alt
                            && !modifiers.ctrl
                            && !modifiers.shift
                            && matches!(
                                key,
                                Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown
                            )
                        {
                            remaining_alt_arrows += 1;
                        }
                    }
                }
            });

            egui::TopBottomPanel::top("test_toolbar")
                .exact_height(40.0)
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if cfg.render_outside_btn {
                            let r = ui.interact(
                                Rect::from_min_size(Pos2::new(8.0, 6.0), Vec2::new(80.0, 28.0)),
                                outside_id,
                                egui::Sense::click(),
                            );
                            if r.clicked() {
                                ctx.memory_mut(|m| m.request_focus(outside_id));
                            }
                        }
                        if cfg.render_toolbar_toggle {
                            let toggle_resp = FloatingTerminalPanel::toolbar_button(
                                ui,
                                state.is_open(),
                                cfg.french,
                            );
                            toggle_id = Some(toggle_resp.id);
                            if toggle_resp.clicked() {
                                // Simulate browser/egui click focusing the button right before toggle.
                                ctx.memory_mut(|m| m.request_focus(toggle_resp.id));
                                state.toggle();
                            }
                        }
                    });
                });

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    if cfg.render_grid_pane {
                        let grid_resp =
                            ui.interact(cfg.workspace_rect, grid_pane_id, egui::Sense::click());
                        if grid_resp.clicked() {
                            grid_clicked = true;
                            ctx.memory_mut(|m| m.request_focus(grid_pane_id));
                        }
                    }
                });

            panel_out = FloatingTerminalPanel::show(
                ctx,
                "floating_terminal_test",
                state,
                FloatingTerminalInput {
                    workspace_rect: cfg.workspace_rect,
                    title: cfg.title,
                    has_terminal: cfg.has_terminal,
                    starting: cfg.starting,
                    can_restart: cfg.can_restart,
                    has_output: cfg.has_output,
                    dialog_open: cfg.dialog_open,
                    typing_in_field: cfg.typing_in_field,
                    french: cfg.french,
                    palette: cfg.theme.spec().ui,
                    content_focus_id: Some(content_id),
                    toggle_button_id: toggle_id,
                },
                |ui, bctx| {
                    body_mounts += 1;
                    body_ctx = Some(bctx);
                    if cfg.has_terminal {
                        let resp = ui.interact(bctx.rect, bctx.focus_id, egui::Sense::click());
                        if resp.clicked() {
                            ui.ctx().memory_mut(|m| m.request_focus(bctx.focus_id));
                        }
                        if bctx.accepts_input {
                            shell_escape_count = ui.input(|i| {
                                i.events
                                    .iter()
                                    .filter(|ev| {
                                        matches!(
                                            ev,
                                            Event::Key {
                                                key: Key::Escape,
                                                pressed: true,
                                                ..
                                            }
                                        )
                                    })
                                    .count()
                            });
                        }
                    } else if !cfg.starting {
                        if ui.button("Open terminal").clicked() {
                            open_terminal_clicked = true;
                        }
                    } else {
                        ui.spinner();
                    }
                },
            );
        },
    );

    StepResult {
        shortcut_actions,
        panel_out,
        body_ctx,
        body_mounts,
        open_terminal_clicked,
        shell_escape_count,
        remaining_ctrl_shift_keys,
        remaining_alt_arrows,
        grid_clicked,
        outside_id,
        grid_pane_id,
        content_id,
        toggle_id,
        full_output,
    }
}

/// Only the first opening has egui's invisible sizing pass. Reopenings and dialog
/// returns must be tested in a single frame, not hidden by this helper.
fn run_open_settled(
    ctx: &egui::Context,
    state: &mut FloatingTerminalState,
    cfg: &StepConfig<'_>,
    t0: f64,
    t1: f64,
) -> StepResult {
    assert!(egui::AreaState::load(ctx, Id::new("floating_terminal_test").with("area")).is_none());
    let first = run_step(ctx, state, cfg, t0, Vec::new());
    assert!(first.body_ctx.is_none_or(|body| !body.accepts_input));
    let mut second = run_step(ctx, state, cfg, t1, Vec::new());
    for act in first.panel_out.actions {
        if !second.panel_out.actions.contains(&act) {
            second.panel_out.actions.insert(0, act);
        }
    }
    second
}

#[test]
fn state_defaults_saved_clamping_reload_keyboard_and_serde_roundtrip() {
    let default_state = FloatingTerminalState::default();
    assert!(!default_state.is_open());
    assert_eq!(default_state.height, FLOATING_DEFAULT_HEIGHT);
    assert_eq!(default_state.shown_height(), 0.45);
    assert!(!default_state.has_keyboard());
    assert!(!default_state.is_resizing());

    // Reloaded open state immediately owns the keyboard (`floatingTerminal.spec.js:113-121`).
    let reopened = FloatingTerminalState::from_saved(true, 0.5);
    assert!(reopened.is_open());
    assert_eq!(reopened.height, 0.5);
    assert!(reopened.has_keyboard());

    // Bad saved values clamp safely (`0.15..=0.90`, `NaN -> 0.45`).
    assert_eq!(clamp_floating_height(0.0), FLOATING_MIN_HEIGHT);
    assert_eq!(clamp_floating_height(5.0), FLOATING_MAX_HEIGHT);
    assert_eq!(clamp_floating_height(f64::NAN), FLOATING_DEFAULT_HEIGHT);

    let from_bad_json: FloatingTerminalState =
        serde_json::from_str(r#"{"open":true,"height":9.5}"#).expect("valid JSON");
    assert!(from_bad_json.is_open());
    assert_eq!(from_bad_json.height, FLOATING_MAX_HEIGHT);
    assert!(from_bad_json.has_keyboard());

    let serialized = serde_json::to_string(&from_bad_json).expect("serialize");
    assert_eq!(serialized, r#"{"open":true,"height":0.9}"#);
}

#[test]
fn slides_in_and_out_blocks_background_clicks_and_renders_starting_placeholder() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut state = FloatingTerminalState::new();
    let mut cfg = StepConfig {
        has_terminal: false,
        starting: true,
        ..Default::default()
    };

    // Closed: zero overlay geometry, background grid receives clicks directly at top of workspace.
    let top_click_pos = Pos2::new(200.0, 80.0);
    let _ = run_step(&ctx, &mut state, &cfg, 0.0, Vec::new());
    let s_down = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.1,
        pointer_down(top_click_pos, PointerButton::Primary),
    );
    assert_eq!(s_down.panel_out.panel_rect, None);
    let s_up = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.2,
        pointer_up(top_click_pos, PointerButton::Primary),
    );
    assert!(s_up.grid_clicked);

    // Open while starting: `45%` height (`180px` of `400px`), invokes `render_body` + EN/FR starting text.
    state.show();
    let s_starting = run_open_settled(&ctx, &mut state, &cfg, 0.25, 0.3);
    let panel_rect = s_starting.panel_out.panel_rect.expect("open panel rect");
    assert_eq!(panel_rect.top(), 40.0);
    assert_eq!(panel_rect.width(), 1000.0);
    assert_eq!(panel_rect.height(), 180.0);
    assert_eq!(s_starting.body_mounts, 1);
    assert!(has_painted_text(
        &s_starting.full_output.shapes,
        "Starting the terminal..."
    ));

    cfg.french = true;
    let s_starting_fr = run_step(&ctx, &mut state, &cfg, 0.4, Vec::new());
    assert_eq!(s_starting_fr.body_mounts, 1);
    assert!(has_painted_text(
        &s_starting_fr.full_output.shapes,
        "Démarrage du terminal..."
    ));

    // Closed/unstarted runtime phase (`!has_terminal && !starting`): `render_body` renders `"Open terminal"`
    // and clicking it keeps keyboard ownership inside the floating panel.
    cfg.french = false;
    cfg.starting = false;
    let s_closed_phase = run_step(&ctx, &mut state, &cfg, 0.44, Vec::new());
    assert_eq!(s_closed_phase.body_mounts, 1);
    let open_btn_pos = accesskit_center(&s_closed_phase.full_output, Role::Button, "Open terminal")
        .expect("Open terminal button in closed phase");
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.46,
        pointer_down(open_btn_pos, PointerButton::Primary),
    );
    let s_open_clicked = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.48,
        pointer_up(open_btn_pos, PointerButton::Primary),
    );
    assert!(s_open_clicked.open_terminal_clicked);
    assert!(state.has_keyboard());

    // Terminal attaches (`has_terminal: false -> true`): immediately focuses terminal and blocks clicks through to grid.
    cfg.has_terminal = true;
    let s_attached = run_step(&ctx, &mut state, &cfg, 0.5, Vec::new());
    assert_eq!(s_attached.body_mounts, 1);
    assert!(
        s_attached
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::FocusTerminal)
    );
    assert_eq!(ctx.memory(|m| m.focused()), Some(s_attached.content_id));
    let bctx = s_attached.body_ctx.expect("body context");
    assert_eq!(
        bctx.rect.height(),
        180.0 - FLOATING_HEADER_HEIGHT - FLOATING_HANDLE_HEIGHT
    );
    assert!(bctx.accepts_input);

    // Clicking inside the open floating panel never clicks the grid pane underneath.
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.6,
        pointer_down(top_click_pos, PointerButton::Primary),
    );
    let s_click_inside = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.7,
        pointer_up(top_click_pos, PointerButton::Primary),
    );
    assert!(!s_click_inside.grid_clicked);
}

#[test]
fn bottom_handle_supports_mouse_drag_preview_clamping_and_keyboard_five_percent_steps() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut state = FloatingTerminalState::from_saved(true, FLOATING_DEFAULT_HEIGHT);
    let cfg = StepConfig::default();

    let s0 = run_open_settled(&ctx, &mut state, &cfg, 0.0, 0.05);
    let handle_center = accesskit_center(
        &s0.full_output,
        Role::Slider,
        "Resize the floating terminal",
    )
    .expect("resize handle in AccessKit");

    // 1. Drag to `60%` (`40 + 240 = 280`): live preview `0.60`, commits `0.60` on release.
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.1,
        pointer_down(handle_center, PointerButton::Primary),
    );
    assert!(state.is_resizing());

    let s_drag_60 = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.2,
        vec![Event::PointerMoved(Pos2::new(handle_center.x, 280.0))],
    );
    assert!((state.shown_height() - 0.60).abs() < 1e-6);
    assert_eq!(s_drag_60.panel_out.panel_rect.unwrap().height(), 240.0);

    let s_rel_60 = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.3,
        pointer_up(Pos2::new(handle_center.x, 280.0), PointerButton::Primary),
    );
    assert!(!state.is_resizing());
    assert_eq!(state.height, 0.60);
    assert!(
        s_rel_60
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::HeightChanged(0.60))
    );

    // 2. Drag above top (`y = 44.0` -> `1%`): live preview stops at `10%` (`40px`), release clamps to `15%` (`0.15`).
    let handle_60 = accesskit_center(
        &s_rel_60.full_output,
        Role::Slider,
        "Resize the floating terminal",
    )
    .unwrap();
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.4,
        pointer_down(handle_60, PointerButton::Primary),
    );
    let s_drag_low = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.5,
        vec![Event::PointerMoved(Pos2::new(handle_60.x, 44.0))],
    );
    assert!((state.shown_height() - 0.10).abs() < 1e-6);
    assert_eq!(s_drag_low.panel_out.panel_rect.unwrap().height(), 40.0);

    let s_rel_low = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.6,
        pointer_up(Pos2::new(handle_60.x, 44.0), PointerButton::Primary),
    );
    assert_eq!(state.height, FLOATING_MIN_HEIGHT);
    assert!(
        s_rel_low
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::HeightChanged(0.15))
    );

    // 3. Drag past bottom (`y = 438.0` -> `99.5%`): live preview stops at `95%` (`380px`), release clamps to `90%` (`0.90`).
    let handle_15 = accesskit_center(
        &s_rel_low.full_output,
        Role::Slider,
        "Resize the floating terminal",
    )
    .unwrap();
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.7,
        pointer_down(handle_15, PointerButton::Primary),
    );
    let s_drag_high = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.8,
        vec![Event::PointerMoved(Pos2::new(handle_15.x, 438.0))],
    );
    assert!((state.shown_height() - 0.95).abs() < 1e-6);
    assert_eq!(s_drag_high.panel_out.panel_rect.unwrap().height(), 380.0);

    let s_rel_high = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.9,
        pointer_up(Pos2::new(handle_15.x, 438.0), PointerButton::Primary),
    );
    assert_eq!(state.height, FLOATING_MAX_HEIGHT);
    assert!(
        s_rel_high
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::HeightChanged(0.90))
    );

    // 4. Keyboard `ArrowUp` / `ArrowDown` on focused handle steps by `±5%` (`FLOATING_KEYBOARD_STEP`).
    let handle_id = Id::new("floating_terminal_test").with("resize_handle");
    ctx.memory_mut(|m| m.request_focus(handle_id));
    let s_up_key = run_step(
        &ctx,
        &mut state,
        &cfg,
        1.0,
        key_tap(Key::ArrowUp, Some(Key::ArrowUp), Modifiers::NONE),
    );
    assert!((state.height - (0.90 - FLOATING_KEYBOARD_STEP)).abs() < 1e-6);
    assert!(
        s_up_key
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::HeightChanged(0.85))
    );

    let s_down_key = run_step(
        &ctx,
        &mut state,
        &cfg,
        1.1,
        key_tap(Key::ArrowDown, Some(Key::ArrowDown), Modifiers::NONE),
    );
    assert_eq!(state.height, 0.90);
    assert!(
        s_down_key
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::HeightChanged(0.90))
    );

    // Pressing `ArrowDown` again at `0.90` ceiling stays clamped at `0.90` with no spurious action.
    let s_saturated = run_step(
        &ctx,
        &mut state,
        &cfg,
        1.2,
        key_tap(Key::ArrowDown, Some(Key::ArrowDown), Modifiers::NONE),
    );
    assert_eq!(state.height, 0.90);
    assert!(s_saturated.panel_out.actions.is_empty());
}

#[test]
fn hide_chevron_hides_without_closing_while_header_omits_maximize_and_split() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut state = FloatingTerminalState::from_saved(true, 0.45);
    let cfg = StepConfig::default();

    let s0 = run_open_settled(&ctx, &mut state, &cfg, 0.0, 0.05);
    let labels = accesskit_button_labels(&s0.full_output);
    assert!(labels.iter().any(|l| l == "More options"));
    assert!(labels.iter().any(|l| l == "Close pane"));
    assert!(labels.iter().any(|l| l == "Hide the floating terminal"));
    assert!(
        !labels
            .iter()
            .any(|l| l.contains("Maximize") || l.contains("Restore"))
    );

    // Open `More options` menu: contains `Copy output`, `Restart`, `Close pane`, NEVER `Split` or `Arrange`.
    let more_pos =
        accesskit_center(&s0.full_output, Role::Button, "More options").expect("More options");
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.1,
        pointer_down(more_pos, PointerButton::Primary),
    );
    let s_menu = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.2,
        pointer_up(more_pos, PointerButton::Primary),
    );
    let _ = run_step(&ctx, &mut state, &cfg, 0.25, Vec::new());
    let s_menu_open = run_step(&ctx, &mut state, &cfg, 0.3, Vec::new());
    assert!(has_painted_text(
        &s_menu_open.full_output.shapes,
        "Copy output"
    ));
    assert!(has_painted_text(&s_menu_open.full_output.shapes, "Restart"));
    assert!(has_painted_text(
        &s_menu_open.full_output.shapes,
        "Close pane"
    ));
    assert!(!has_painted_text(
        &s_menu_open.full_output.shapes,
        "Split right"
    ));
    assert!(!has_painted_text(
        &s_menu_open.full_output.shapes,
        "Split down"
    ));
    assert!(!has_painted_text(
        &s_menu_open.full_output.shapes,
        "Arrange panes in a grid"
    ));
    let _ = s_menu;

    // Click `Copy output` in the menu: emits `CopyOutput` and keeps panel open.
    let copy_pos = accesskit_center(&s_menu_open.full_output, Role::Button, "Copy output")
        .or_else(|| find_painted_text_center(&s_menu_open.full_output.shapes, "Copy output"))
        .expect("Copy output item");
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.34,
        pointer_down(copy_pos, PointerButton::Primary),
    );
    let s_copy = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.38,
        pointer_up(copy_pos, PointerButton::Primary),
    );
    assert!(
        s_copy
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::CopyOutput)
    );
    assert!(state.is_open());
    // Next frame after popup closes restores terminal focus and keyboard ownership.
    let s_after_copy = run_step(&ctx, &mut state, &cfg, 0.39, Vec::new());
    assert!(state.has_keyboard());
    assert_eq!(ctx.memory(|m| m.focused()), Some(s_after_copy.content_id));

    // Re-open `More options` and click `Restart`: emits `Restart` and keeps panel open.
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.40,
        pointer_down(more_pos, PointerButton::Primary),
    );
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.42,
        pointer_up(more_pos, PointerButton::Primary),
    );
    let _ = run_step(&ctx, &mut state, &cfg, 0.43, Vec::new());
    let s_menu_open2 = run_step(&ctx, &mut state, &cfg, 0.44, Vec::new());
    let restart_pos = accesskit_center(&s_menu_open2.full_output, Role::Button, "Restart")
        .or_else(|| find_painted_text_center(&s_menu_open2.full_output.shapes, "Restart"))
        .expect("Restart item");
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.46,
        pointer_down(restart_pos, PointerButton::Primary),
    );
    let s_restart = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.5,
        pointer_up(restart_pos, PointerButton::Primary),
    );
    assert!(
        s_restart
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::Restart)
    );
    assert!(state.is_open());

    // Click `Hide` chevron on the handle: hides WITHOUT `Close` or starting a drag.
    let s_before_hide = run_step(&ctx, &mut state, &cfg, 0.6, Vec::new());
    let hide_pos = accesskit_center(
        &s_before_hide.full_output,
        Role::Button,
        "Hide the floating terminal",
    )
    .expect("Hide button");
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.7,
        pointer_down(hide_pos, PointerButton::Primary),
    );
    assert!(!state.is_resizing());
    let s_hidden = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.8,
        pointer_up(hide_pos, PointerButton::Primary),
    );
    assert!(!state.is_open());
    assert_eq!(state.height, 0.45);
    assert!(
        s_hidden
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::Hide)
    );
    assert!(
        !s_hidden
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::Close)
    );

    // Reopen and click header `Close pane`: emits `Close` and hides.
    state.show();
    let s_reopened = run_step(&ctx, &mut state, &cfg, 0.9, Vec::new());
    let close_pos = accesskit_center(&s_reopened.full_output, Role::Button, "Close pane")
        .expect("Close pane button");
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        1.0,
        pointer_down(close_pos, PointerButton::Primary),
    );
    let s_closed = run_step(
        &ctx,
        &mut state,
        &cfg,
        1.1,
        pointer_up(close_pos, PointerButton::Primary),
    );
    assert!(!state.is_open());
    assert!(
        s_closed
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::Close)
    );
}

#[test]
fn focus_lifecycle_captures_outside_focus_handles_unmounted_or_toolbar_and_respects_dialogs_and_grid()
 {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut state = FloatingTerminalState::new();
    let mut cfg = StepConfig::default();

    // 1. Focus an outside widget, open floating terminal -> terminal takes focus -> hide -> outside regains focus.
    let s_init = run_step(&ctx, &mut state, &cfg, 0.0, Vec::new());
    ctx.memory_mut(|m| m.request_focus(s_init.outside_id));
    let _ = run_step(&ctx, &mut state, &cfg, 0.1, Vec::new());
    assert_eq!(ctx.memory(|m| m.focused()), Some(s_init.outside_id));

    state.show();
    let s_open = run_open_settled(&ctx, &mut state, &cfg, 0.15, 0.2);
    assert_eq!(ctx.memory(|m| m.focused()), Some(s_open.content_id));
    assert!(state.has_keyboard());

    state.hide();
    let s_hide = run_step(&ctx, &mut state, &cfg, 0.3, Vec::new());
    assert_eq!(ctx.memory(|m| m.focused()), Some(s_init.outside_id));
    assert!(
        s_hide
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::RestoreFocus(Some(
                s_init.outside_id
            )))
    );

    // 2. Old focused widget disappears while floating terminal is open -> hide emits `RestoreFocus(None)`.
    state.show();
    let _ = run_step(&ctx, &mut state, &cfg, 0.4, Vec::new());
    cfg.render_outside_btn = false;
    let _ = run_step(&ctx, &mut state, &cfg, 0.5, Vec::new());
    let _ = run_step(&ctx, &mut state, &cfg, 0.6, Vec::new());
    state.hide();
    let s_gone = run_step(&ctx, &mut state, &cfg, 0.7, Vec::new());
    assert!(
        s_gone
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::RestoreFocus(None))
    );

    // 3. Toggled open & closed via toolbar button -> keyboard goes to active pane (`RestoreFocus(None)`), NOT toolbar button.
    cfg.render_outside_btn = true;
    let s_tb0 = run_step(&ctx, &mut state, &cfg, 0.8, Vec::new());
    let tb_pos = accesskit_center(&s_tb0.full_output, Role::Button, "Floating terminal")
        .expect("toolbar toggle");
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.9,
        pointer_down(tb_pos, PointerButton::Primary),
    );
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        1.0,
        pointer_up(tb_pos, PointerButton::Primary),
    );
    let s_tb_open = run_step(&ctx, &mut state, &cfg, 1.05, Vec::new());
    assert!(state.is_open());
    assert_eq!(ctx.memory(|m| m.focused()), Some(s_tb_open.content_id));

    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        1.1,
        pointer_down(tb_pos, PointerButton::Primary),
    );
    let s_tb_hide = run_step(
        &ctx,
        &mut state,
        &cfg,
        1.2,
        pointer_up(tb_pos, PointerButton::Primary),
    );
    assert!(!state.is_open());
    assert_ne!(ctx.memory(|m| m.focused()), s_tb_hide.toggle_id);
    assert!(
        s_tb_hide
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::RestoreFocus(None))
    );

    // 4. Dialog borrows focus -> `has_keyboard()` stays `true`; closing dialog returns focus to floating terminal.
    state.show();
    let _ = run_step(&ctx, &mut state, &cfg, 1.3, Vec::new());
    let dialog_input = Id::new("modal_dialog_search");
    cfg.dialog_open = true;
    ctx.memory_mut(|m| m.request_focus(dialog_input));
    let s_dialog = run_step(&ctx, &mut state, &cfg, 1.4, Vec::new());
    assert!(state.has_keyboard());
    assert!(s_dialog.panel_out.has_keyboard);

    cfg.dialog_open = false;
    let s_dialog_closed = run_step(&ctx, &mut state, &cfg, 1.5, Vec::new());
    assert_eq!(
        ctx.memory(|m| m.focused()),
        Some(s_dialog_closed.content_id)
    );
    assert!(
        s_dialog_closed
            .panel_out
            .actions
            .contains(&FloatingTerminalAction::FocusTerminal)
    );

    // 5. Clicking visible grid pane below open drawer transfers keyboard to grid; hiding preserves grid focus.
    let below_drawer = Pos2::new(500.0, 360.0);
    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        1.6,
        pointer_down(below_drawer, PointerButton::Primary),
    );
    let s_grid_focus = run_step(
        &ctx,
        &mut state,
        &cfg,
        1.7,
        pointer_up(below_drawer, PointerButton::Primary),
    );
    assert!(s_grid_focus.grid_clicked);
    assert!(!state.has_keyboard());
    assert!(!s_grid_focus.panel_out.has_keyboard);
    assert_eq!(ctx.memory(|m| m.focused()), Some(s_grid_focus.grid_pane_id));

    state.hide();
    let s_hide_from_grid = run_step(&ctx, &mut state, &cfg, 1.8, Vec::new());
    assert_eq!(
        ctx.memory(|m| m.focused()),
        Some(s_hide_from_grid.grid_pane_id)
    );
    assert!(s_hide_from_grid.panel_out.actions.is_empty());
}

#[test]
fn escape_reaches_shell_while_ctrl_backquote_and_floating_shortcuts_isolate_background_grid() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut state = FloatingTerminalState::new();
    let mut cfg = StepConfig::default();
    let _ = run_step(&ctx, &mut state, &cfg, 0.0, Vec::new());

    // 1. `Ctrl+\`` on French AZERTY (logical `Key::Num2` / `²`, physical `Key::Backtick`) opens panel.
    let s_azerty = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.1,
        vec![key_press(
            Key::Num2,
            Some(Key::Backtick),
            Modifiers::CTRL,
            false,
        )],
    );
    assert_eq!(
        s_azerty.shortcut_actions,
        vec![FloatingTerminalAction::Show]
    );
    assert!(state.is_open());

    // Settle initial Area sizing pass while `Key::Num2` stays held down in `egui`'s `keys_down`.
    let _ = run_step(&ctx, &mut state, &cfg, 0.15, Vec::new());

    // Key repeat on held `Ctrl+\`` (`Key::Num2` still in `keys_down`) is consumed without toggling back off.
    let s_repeat = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.2,
        vec![key_press(
            Key::Num2,
            Some(Key::Backtick),
            Modifiers::CTRL,
            true,
        )],
    );
    assert!(s_repeat.shortcut_actions.is_empty());
    assert!(state.is_open());

    let _ = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.25,
        vec![key_release(Key::Num2, Some(Key::Backtick), Modifiers::CTRL)],
    );

    // `Ctrl+Shift+\`` and `Ctrl+Alt+\`` (AltGr) do NOT toggle.
    let mut mod_events = key_tap(
        Key::Backtick,
        Some(Key::Backtick),
        Modifiers::CTRL | Modifiers::SHIFT,
    );
    mod_events.extend(key_tap(
        Key::Backtick,
        Some(Key::Backtick),
        Modifiers::CTRL | Modifiers::ALT,
    ));
    let s_mods = run_step(&ctx, &mut state, &cfg, 0.3, mod_events);
    assert!(s_mods.shortcut_actions.is_empty());
    assert!(state.is_open());

    // 2. `Escape` NEVER hides floating terminal and passes straight through to the shell (`shell_escape_count == 1`).
    let s_esc = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.4,
        key_tap(Key::Escape, Some(Key::Escape), Modifiers::NONE),
    );
    assert!(state.is_open());
    assert_eq!(s_esc.shell_escape_count, 1);

    // 3. `Ctrl+Shift+{E,O,B}` and `Alt+Arrows` are swallowed so background grid never splits/moves/broadcasts,
    //    whereas `Ctrl+Shift+P` still reaches `App`.
    let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
    let mut swallow_events = Vec::new();
    for k in [Key::E, Key::O, Key::B] {
        swallow_events.extend(key_tap(k, Some(k), ctrl_shift));
    }
    for k in [Key::ArrowLeft, Key::ArrowDown] {
        swallow_events.extend(key_tap(k, Some(k), Modifiers::ALT));
    }
    swallow_events.extend(key_tap(Key::P, Some(Key::P), ctrl_shift));
    let s_swallow = run_step(&ctx, &mut state, &cfg, 0.5, swallow_events);
    assert!(s_swallow.shortcut_actions.is_empty());
    assert_eq!(s_swallow.remaining_ctrl_shift_keys, vec![Key::P]);
    assert_eq!(s_swallow.remaining_alt_arrows, 0);

    // 4. `Ctrl+Shift+R` restarts floating terminal (stays open, consumed from grid).
    let s_restart = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.6,
        key_tap(Key::R, Some(Key::R), ctrl_shift),
    );
    assert_eq!(
        s_restart.shortcut_actions,
        vec![FloatingTerminalAction::Restart]
    );
    assert!(s_restart.remaining_ctrl_shift_keys.is_empty());
    assert!(state.is_open());

    // 5. `Ctrl+Shift+W` closes floating terminal (consumed so active grid pane is untouched).
    let s_close = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.7,
        key_tap(Key::W, Some(Key::W), ctrl_shift),
    );
    assert_eq!(
        s_close.shortcut_actions,
        vec![FloatingTerminalAction::Close]
    );
    assert!(s_close.remaining_ctrl_shift_keys.is_empty());
    assert!(!state.is_open());

    // 6. While a dialog is open (`dialog_open: true`), `Ctrl+\`` does NOT toggle floating terminal.
    cfg.dialog_open = true;
    let s_blocked = run_step(
        &ctx,
        &mut state,
        &cfg,
        0.8,
        key_tap(Key::Backtick, Some(Key::Backtick), Modifiers::CTRL),
    );
    assert!(s_blocked.shortcut_actions.is_empty());
    assert!(!state.is_open());
}

#[test]
fn renders_across_all_seven_themes_both_languages_and_narrow_viewports_without_overflow() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut t = 0.0_f64;

    for theme in THEMES {
        for french in [false, true] {
            let mut state = FloatingTerminalState::from_saved(true, 0.45);
            let cfg = StepConfig {
                viewport: Vec2::new(260.0, 200.0),
                workspace_rect: Rect::from_min_size(Pos2::new(0.0, 40.0), Vec2::new(260.0, 140.0)),
                title: "PowerShell\nwith\tcontrol-chars-and-very-long-workspace-branch-suffix-name",
                french,
                theme,
                ..Default::default()
            };
            t += 0.1;
            let step = run_step(&ctx, &mut state, &cfg, t, Vec::new());
            let pr = step.panel_out.panel_rect.expect("panel rect");
            let br = step.panel_out.body_rect.expect("body rect");
            assert!(cfg.workspace_rect.contains_rect(pr));
            assert!(pr.contains_rect(br));
            let expected_hide = if french {
                "Masquer le terminal flottant"
            } else {
                "Hide the floating terminal"
            };
            assert!(
                accesskit_center(&step.full_output, Role::Button, expected_hide).is_some(),
                "missing `{expected_hide}` in theme {:?}",
                theme
            );
        }
    }
}

#[test]
fn yielding_keyboard_keeps_panel_visible_and_does_not_reclaim_external_focus() {
    let ctx = egui::Context::default();
    let mut state = FloatingTerminalState::new();
    let cfg = StepConfig::default();
    let initial = run_step(&ctx, &mut state, &cfg, 0.0, Vec::new());
    state.show();
    let opened = run_open_settled(&ctx, &mut state, &cfg, 0.1, 0.2);
    assert_eq!(ctx.memory(|m| m.focused()), Some(opened.content_id));
    // A programmatic navigation may supersede a queued terminal focus request.
    state.show();
    state.yield_keyboard();
    ctx.memory_mut(|m| m.request_focus(initial.outside_id));
    for time in [0.3, 0.4] {
        let step = run_step(&ctx, &mut state, &cfg, time, Vec::new());
        assert!(state.is_open());
        assert!(!state.has_keyboard());
        assert_eq!(ctx.memory(|m| m.focused()), Some(initial.outside_id));
        assert!(
            !step
                .panel_out
                .actions
                .contains(&FloatingTerminalAction::FocusTerminal)
        );
    }
}
