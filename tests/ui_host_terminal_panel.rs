//! Standalone headless `egui` + `AccessKit` tests for `src/host_terminal_panel.rs`
//! (Gate 7B remote `tessel-host` terminal overlays, close-choice safety, and EN/FR parity).

#[path = "../src/host_terminal_panel.rs"]
mod host_terminal_panel;
#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/theme.rs"]
mod theme;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Event, Id, Key, Modifiers, PointerButton, Pos2, Rect, Shape, Vec2};
use host_terminal_panel::{
    HostHeaderTag, HostTerminalAction, HostTerminalCapabilities, HostTerminalInput,
    HostTerminalPanel, HostTerminalPhase, LostReason, MAX_EPOCH_CHARS, MAX_HOST_LABEL_CHARS,
    MAX_NOTE_CHARS, MAX_RAW_SCAN_CHARS, PersistenceWarningInput, ReadOnlyReason, SkippedOutput,
    SkippedOutputInput, SurvivesLogout, header_tag_width, sanitize_host_text, show_header_tag,
};
use theme_palettes::THEMES;

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

fn has_painted_substring(shapes: &[egui::epaint::ClippedShape], needle: &str) -> bool {
    painted_texts(shapes)
        .iter()
        .any(|(text, _, _)| text.contains(needle))
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
    vec![Event::PointerButton {
        pos,
        button,
        pressed: false,
        modifiers: Modifiers::NONE,
    }]
}

fn key_ev(key: Key, pressed: bool, repeat: bool, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat,
        modifiers,
    }
}

fn find_accesskit_button_center(out: &egui::FullOutput, label: &str) -> Option<Pos2> {
    let update = out.platform_output.accesskit_update.as_ref()?;
    for (_, node) in &update.nodes {
        if node.role() == Role::Button
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

fn require_accesskit_button_center(out: &egui::FullOutput, label: &str) -> Pos2 {
    find_accesskit_button_center(out, label)
        .unwrap_or_else(|| panic!("missing AccessKit Button with exact label `{label}`"))
}

struct FrameMetrics {
    action: Option<HostTerminalAction>,
    terminal_rect: Rect,
    grid_rows_cols: (u16, u16),
    ui_min_rect: Rect,
    ui_cursor: Rect,
    forwarded_escape_count: usize,
    unconsumed_enter_count: usize,
    popup_open: bool,
    output: egui::FullOutput,
}

fn step_pane_frame(
    ctx: &egui::Context,
    panel: &mut HostTerminalPanel,
    input: &HostTerminalInput<'_>,
    viewport: Vec2,
    time: f64,
    events: Vec<Event>,
    with_context_menu: bool,
) -> FrameMetrics {
    let mut action = None;
    let mut terminal_rect = Rect::NOTHING;
    let mut grid_rows_cols = (0_u16, 0_u16);
    let mut ui_min_rect = Rect::NOTHING;
    let mut ui_cursor = Rect::NOTHING;
    let mut forwarded_escape_count = 0_usize;
    let mut unconsumed_enter_count = 0_usize;
    let mut popup_open = false;

    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
            focused: true,
            time: Some(time),
            events,
            ..Default::default()
        },
        |ctx| {
            let intercepted = panel.intercept_key(ctx, input);
            unconsumed_enter_count = ctx.input(|i| {
                i.events
                    .iter()
                    .filter(|ev| {
                        matches!(
                            ev,
                            Event::Key {
                                key: Key::Enter,
                                pressed: true,
                                ..
                            }
                        )
                    })
                    .count()
            });
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    // Simulated 28px pane header reserving `.exit-tag` width + truncating title
                    // (matching `app.rs:2272-2319` so narrow panes never overflow horizontally).
                    let header_w = ui.available_width();
                    ui.allocate_ui_with_layout(
                        Vec2::new(header_w, 28.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_size(Vec2::new(header_w, 28.0));
                            let tag = HostTerminalPanel::header_tag(input.phase);
                            let tag_reserve = tag.map_or(0.0, |t| {
                                header_tag_width(ui, t, input.french) + ui.spacing().item_spacing.x
                            });
                            let title_w = (header_w - tag_reserve).max(0.0);
                            ui.allocate_ui_with_layout(
                                Vec2::new(title_w, 28.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(sanitize_host_text(
                                            input.host_label,
                                            MAX_HOST_LABEL_CHARS,
                                        ))
                                        .truncate(),
                                    );
                                },
                            );
                            if let Some(t) = tag {
                                let _ = show_header_tag(ui, t, input.french);
                            }
                        },
                    );

                    // Simulated `TerminalView::show`: compute grid from available space BEFORE overlays
                    let available = ui.available_size();
                    let cols = ((available.x / 8.0).floor() as u16).max(2);
                    let rows = ((available.y / 16.0).floor() as u16).max(1);
                    grid_rows_cols = (rows, cols);

                    let (rect, term_resp) =
                        ui.allocate_exact_size(available, egui::Sense::click_and_drag());
                    terminal_rect = rect;

                    if !panel.blocks_terminal_focus()
                        && term_resp.clicked()
                        && term_resp
                            .interact_pointer_pos()
                            .is_none_or(|pos| !panel.pointer_over_overlay(pos))
                    {
                        term_resp.request_focus();
                    }
                    if with_context_menu {
                        term_resp.context_menu(|ui| {
                            if ui.button("Copy visible text").clicked() {
                                ui.close();
                            }
                        });
                    }

                    // Simulated terminal keyboard forwarding loop gated by `accepts_input`
                    if panel.accepts_input(input.phase) {
                        forwarded_escape_count = ui.input(|i| {
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

                    let overlay_act = panel.show_overlays(ui, rect, input);
                    ui_min_rect = ui.min_rect();
                    ui_cursor = ui.cursor();
                    action = intercepted.or(overlay_act);
                });
            popup_open = egui::Popup::is_any_open(ctx);
        },
    );

    FrameMetrics {
        action,
        terminal_rect,
        grid_rows_cols,
        ui_min_rect,
        ui_cursor,
        forwarded_escape_count,
        unconsumed_enter_count,
        popup_open,
        output,
    }
}

fn settle_pane(
    ctx: &egui::Context,
    panel: &mut HostTerminalPanel,
    input: &HostTerminalInput<'_>,
    viewport: Vec2,
    time: f64,
) -> FrameMetrics {
    let _ = step_pane_frame(ctx, panel, input, viewport, time, vec![], false);
    step_pane_frame(ctx, panel, input, viewport, time + 0.016, vec![], false)
}

fn click_pane(
    ctx: &egui::Context,
    panel: &mut HostTerminalPanel,
    input: &HostTerminalInput<'_>,
    viewport: Vec2,
    time: f64,
    pos: Pos2,
) -> (Option<HostTerminalAction>, FrameMetrics) {
    let f1 = step_pane_frame(
        ctx,
        panel,
        input,
        viewport,
        time,
        pointer_down(pos, PointerButton::Primary),
        false,
    );
    let f2 = step_pane_frame(
        ctx,
        panel,
        input,
        viewport,
        time + 0.016,
        pointer_up(pos, PointerButton::Primary),
        false,
    );
    (f1.action.or(f2.action), f2)
}

fn full_caps() -> HostTerminalCapabilities {
    HostTerminalCapabilities {
        can_terminate: true,
        can_detach: true,
        can_take_control: true,
        can_reconnect: true,
        can_restart: true,
    }
}

#[test]
fn focus_and_held_escape_isolation_never_leaks_to_live_terminal_and_respects_open_popup() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));

    let mut panel = HostTerminalPanel::new();
    let vp = Vec2::new(800.0, 500.0);
    let live = HostTerminalPhase::Live;
    let input = HostTerminalInput {
        id_salt: Id::new("pane-esc"),
        host_label: "prod-eu-1",
        epoch: Some("epoch-live-a"),
        phase: &live,
        capabilities: full_caps(),
        terminal_focused: true,
        persistence_warning: None,
        skipped_output: None,
        scrollback_lines: 0,
        french: false,
    };

    let base = settle_pane(&ctx, &mut panel, &input, vp, 1.0);
    assert!(panel.accepts_input(&live));
    assert!(panel.accepts_resize(&live));
    assert!(!panel.blocks_terminal_focus());
    assert!(panel.overlay_rects().is_empty());

    // Requesting close in `Live` with `can_detach && can_terminate` opens the confirmation card
    // and moves focus onto the safe `Detach` button.
    assert_eq!(panel.request_close(&live, input.capabilities), None);
    assert!(panel.has_pending_close_choice());
    assert!(panel.blocks_terminal_focus());
    assert!(!panel.accepts_input(&live));

    let opened = settle_pane(&ctx, &mut panel, &input, vp, 1.1);
    assert_eq!(panel.overlay_rects().len(), 1);
    require_accesskit_button_center(&opened.output, "Detach");
    require_accesskit_button_center(&opened.output, "Terminate process");
    require_accesskit_button_center(&opened.output, "Cancel");

    // Pressing Enter immediately activates the initially focused safe `Detach` button
    // (never `Terminate process`).
    let enter_detach = step_pane_frame(
        &ctx,
        &mut panel,
        &input,
        vp,
        1.15,
        vec![
            key_ev(Key::Enter, true, false, Modifiers::NONE),
            key_ev(Key::Enter, false, false, Modifiers::NONE),
        ],
        false,
    );
    assert_eq!(enter_detach.action, Some(HostTerminalAction::Detach));
    assert!(!panel.has_pending_close_choice());

    // Re-open close confirmation and open a real `egui::Popup` context menu on the terminal.
    assert_eq!(panel.request_close(&live, input.capabilities), None);
    let _ = settle_pane(&ctx, &mut panel, &input, vp, 1.18);
    let center = base.terminal_rect.center();
    let _ = step_pane_frame(
        &ctx,
        &mut panel,
        &input,
        vp,
        1.2,
        pointer_down(center, PointerButton::Secondary),
        true,
    );
    let popup_frame = step_pane_frame(
        &ctx,
        &mut panel,
        &input,
        vp,
        1.22,
        pointer_up(center, PointerButton::Secondary),
        true,
    );
    assert!(popup_frame.popup_open, "context menu popup should be open");
    assert!(panel.has_pending_close_choice());

    // 1st Escape press closes the popup FIRST without dismissing `confirm_close_open`,
    // and autorepeats while held neither dismiss `confirm_close_open` nor leak `\x1b`.
    let esc_popup_down = step_pane_frame(
        &ctx,
        &mut panel,
        &input,
        vp,
        1.3,
        vec![key_ev(Key::Escape, true, false, Modifiers::NONE)],
        true,
    );
    assert!(
        panel.has_pending_close_choice(),
        "close confirmation must remain open when Escape closed a popup"
    );
    assert_eq!(esc_popup_down.forwarded_escape_count, 0);

    let esc_popup_repeat = step_pane_frame(
        &ctx,
        &mut panel,
        &input,
        vp,
        1.35,
        vec![key_ev(Key::Escape, true, true, Modifiers::NONE)],
        true,
    );
    assert!(
        !esc_popup_repeat.popup_open,
        "popup must close on first Escape"
    );
    assert!(
        panel.has_pending_close_choice(),
        "held Escape repeat after popup close must not dismiss confirmation"
    );
    assert_eq!(esc_popup_repeat.forwarded_escape_count, 0);

    // Physical release resets `escape_held`.
    let _ = step_pane_frame(
        &ctx,
        &mut panel,
        &input,
        vp,
        1.4,
        vec![key_ev(Key::Escape, false, false, Modifiers::NONE)],
        true,
    );
    assert!(panel.has_pending_close_choice());

    // 2nd fresh Escape press dismisses `confirm_close_open`, and subsequent `repeat: true`
    // frames while Escape stays physically down MUST NOT leak `\x1b` into the `Live` terminal!
    let esc_dismiss = step_pane_frame(
        &ctx,
        &mut panel,
        &input,
        vp,
        1.5,
        vec![key_ev(Key::Escape, true, false, Modifiers::NONE)],
        true,
    );
    assert!(!panel.has_pending_close_choice());
    assert!(
        !panel.accepts_input(&live),
        "input must stay gated while Escape remains physically held"
    );
    assert_eq!(esc_dismiss.forwarded_escape_count, 0);

    let esc_live_repeat = step_pane_frame(
        &ctx,
        &mut panel,
        &input,
        vp,
        1.55,
        vec![key_ev(Key::Escape, true, true, Modifiers::NONE)],
        true,
    );
    assert!(!panel.accepts_input(&live));
    assert_eq!(
        esc_live_repeat.forwarded_escape_count, 0,
        "held Escape autorepeat must never leak into Live PTY"
    );

    // Once Escape is physically released, `Live` accepts input again and forwards fresh Escape.
    let after_release = step_pane_frame(
        &ctx,
        &mut panel,
        &input,
        vp,
        1.6,
        vec![key_ev(Key::Escape, false, false, Modifiers::NONE)],
        true,
    );
    assert!(panel.accepts_input(&live));
    assert_eq!(after_release.forwarded_escape_count, 0);

    let fresh_vim_esc = step_pane_frame(
        &ctx,
        &mut panel,
        &input,
        vp,
        1.7,
        vec![key_ev(Key::Escape, true, false, Modifiers::NONE)],
        true,
    );
    assert_eq!(
        fresh_vim_esc.forwarded_escape_count, 1,
        "fresh Escape in Live after release must reach terminal"
    );
    let _ = step_pane_frame(
        &ctx,
        &mut panel,
        &input,
        vp,
        1.72,
        vec![key_ev(Key::Escape, false, false, Modifiers::NONE)],
        true,
    );

    // When `can_detach == false && can_terminate == true`, initial focus lands on `Cancel`
    // (never on destructive `Terminate process`), so pressing Enter cancels safely!
    let term_only_input = HostTerminalInput {
        capabilities: HostTerminalCapabilities {
            can_detach: false,
            can_terminate: true,
            ..Default::default()
        },
        ..input
    };
    assert_eq!(
        panel.request_close(&live, term_only_input.capabilities),
        None
    );
    let _ = settle_pane(&ctx, &mut panel, &term_only_input, vp, 2.0);
    let enter_on_default = step_pane_frame(
        &ctx,
        &mut panel,
        &term_only_input,
        vp,
        2.1,
        vec![
            key_ev(Key::Enter, true, false, Modifiers::NONE),
            key_ev(Key::Enter, false, false, Modifiers::NONE),
        ],
        false,
    );
    assert_eq!(
        enter_on_default.action, None,
        "default focus when Detach is unavailable must be Cancel, never Terminate"
    );
    assert!(!panel.has_pending_close_choice());
}

#[test]
fn readonly_suppresses_input_and_resize_throttles_flash_and_emits_take_control() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));

    let mut panel = HostTerminalPanel::new();
    let vp = Vec2::new(800.0, 500.0);
    let ro_elsewhere = HostTerminalPhase::ReadOnly {
        reason: ReadOnlyReason::ControlledElsewhere,
    };
    let no_take_input = HostTerminalInput {
        id_salt: Id::new("pane-ro"),
        host_label: "gpu-cluster",
        epoch: Some("ep-ro-1"),
        phase: &ro_elsewhere,
        capabilities: HostTerminalCapabilities {
            can_take_control: false,
            can_detach: true,
            can_terminate: false,
            ..Default::default()
        },
        terminal_focused: true,
        persistence_warning: None,
        skipped_output: None,
        scrollback_lines: 0,
        french: false,
    };

    assert!(!panel.accepts_input(&ro_elsewhere));
    assert!(!panel.accepts_resize(&ro_elsewhere));
    assert_eq!(
        HostTerminalPanel::header_tag(&ro_elsewhere),
        Some(HostHeaderTag::ReadOnly)
    );

    let f0 = settle_pane(&ctx, &mut panel, &no_take_input, vp, 5.0);
    assert!(has_painted_text(
        &f0.output.shapes,
        "Read-only — controlled in another window."
    ));
    assert!(
        find_accesskit_button_center(&f0.output, "Take control").is_none(),
        "Take control must stay hidden when can_take_control is false"
    );

    // `Ctrl+C` copy shortcut must NOT trigger the read-only write-blocked flash.
    let ctrl_c = Modifiers {
        ctrl: true,
        ..Modifiers::NONE
    };
    let _ = step_pane_frame(
        &ctx,
        &mut panel,
        &no_take_input,
        vp,
        6.0,
        vec![key_ev(Key::C, true, false, ctrl_c)],
        false,
    );
    assert!(!panel.is_readonly_flash_active(6.1));

    // Typing printable text at t = 10.0 triggers the 1.6s flash, throttled for 4.0s.
    let f_write = step_pane_frame(
        &ctx,
        &mut panel,
        &no_take_input,
        vp,
        10.0,
        vec![Event::Text("whoami".to_owned())],
        false,
    );
    assert!(panel.is_readonly_flash_active(10.5));
    assert!(has_painted_text(
        &f_write.output.shapes,
        "Read-only — input blocked while another window controls this session."
    ));

    // Flash expires after 1.6s (at t = 12.0), and typing at t = 12.5 (< 14.0s cooldown) stays throttled.
    let _ = step_pane_frame(&ctx, &mut panel, &no_take_input, vp, 12.0, vec![], false);
    assert!(!panel.is_readonly_flash_active(12.0));
    let _ = step_pane_frame(
        &ctx,
        &mut panel,
        &no_take_input,
        vp,
        12.5,
        vec![Event::Text("ls".to_owned())],
        false,
    );
    assert!(
        !panel.is_readonly_flash_active(12.6),
        "read-only flash must respect 4.0s cooldown"
    );

    // Typing at t = 14.2 (>= 4.0s after 10.0) flashes again.
    let _ = step_pane_frame(
        &ctx,
        &mut panel,
        &no_take_input,
        vp,
        14.2,
        vec![key_ev(Key::Enter, true, false, Modifiers::NONE)],
        false,
    );
    assert!(panel.is_readonly_flash_active(14.5));

    // Observer mode with `can_take_control == true` renders `Take control` and emits `TakeControl`.
    let ro_observer = HostTerminalPhase::ReadOnly {
        reason: ReadOnlyReason::ObserverAttach,
    };
    let can_take_input = HostTerminalInput {
        phase: &ro_observer,
        capabilities: HostTerminalCapabilities {
            can_take_control: true,
            ..no_take_input.capabilities
        },
        ..no_take_input
    };
    let f_obs = settle_pane(&ctx, &mut panel, &can_take_input, vp, 20.0);
    assert!(has_painted_text(
        &f_obs.output.shapes,
        "Read-only — attached as observer."
    ));
    let take_pos = require_accesskit_button_center(&f_obs.output, "Take control");
    let (act, _) = click_pane(&ctx, &mut panel, &can_take_input, vp, 20.1, take_pos);
    assert_eq!(act, Some(HostTerminalAction::TakeControl));
}

#[test]
fn overlays_never_alter_terminal_rect_or_grid_size_across_zero_one_and_three_banners() {
    for viewport in [Vec2::new(800.0, 500.0), Vec2::new(280.0, 320.0)] {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));

        let mut panel = HostTerminalPanel::new();
        let live = HostTerminalPhase::Live;
        let ro = HostTerminalPhase::ReadOnly {
            reason: ReadOnlyReason::ControlledElsewhere,
        };
        let lost = HostTerminalPhase::Lost {
            reason: LostReason::EpochChanged,
        };

        // 0 banners
        let in0 = HostTerminalInput {
            id_salt: Id::new(("geom", viewport.x as i32)),
            host_label: "staging-api-long-hostname.example.internal",
            epoch: Some("ep-geom-1"),
            phase: &live,
            capabilities: full_caps(),
            terminal_focused: true,
            persistence_warning: None,
            skipped_output: None,
            scrollback_lines: 0,
            french: false,
        };
        let m0 = settle_pane(&ctx, &mut panel, &in0, viewport, 1.0);
        assert_eq!(panel.overlay_rects().len(), 0);

        // 1 banner
        let in1 = HostTerminalInput {
            persistence_warning: Some(PersistenceWarningInput::new(
                "ep-geom-1",
                SurvivesLogout::No,
            )),
            ..in0
        };
        let m1 = settle_pane(&ctx, &mut panel, &in1, viewport, 2.0);
        assert_eq!(panel.overlay_rects().len(), 1);

        // 3 stacked top banners (`ReadOnly` + `SurvivesLogout::Unknown` + `SkippedOutput::Resynchronized`)
        let in3 = HostTerminalInput {
            phase: &ro,
            persistence_warning: Some(PersistenceWarningInput::new(
                "ep-geom-1",
                SurvivesLogout::Unknown,
            )),
            skipped_output: Some(SkippedOutputInput::resynchronized("ep-geom-1")),
            scrollback_lines: 48,
            ..in0
        };
        let m3 = settle_pane(&ctx, &mut panel, &in3, viewport, 3.0);
        let rects_3 = panel.overlay_rects().to_vec();
        assert_eq!(rects_3.len(), 3);
        assert!(panel.pointer_over_overlay(rects_3[0].center()));
        assert!(!panel.pointer_over_overlay(Pos2::new(
            m3.terminal_rect.center().x,
            m3.terminal_rect.bottom() - 4.0
        )));

        for (idx, r) in rects_3.iter().enumerate() {
            assert!(
                r.left() >= m3.terminal_rect.left() - 0.5
                    && r.right() <= m3.terminal_rect.right() + 0.5,
                "banner {idx} overflowed terminal_rect horizontally at {viewport:?}: {r:?} vs {:?}",
                m3.terminal_rect
            );
            if idx > 0 {
                assert!(
                    r.top() >= rects_3[idx - 1].bottom() + 7.5,
                    "stacked banners {idx} and {} overlapped: {:?} vs {:?}",
                    idx - 1,
                    rects_3[idx - 1],
                    r
                );
            }
        }

        // Bottom `.exit-overlay` in `Lost`
        let in_lost = HostTerminalInput {
            phase: &lost,
            ..in0
        };
        let m_lost = settle_pane(&ctx, &mut panel, &in_lost, viewport, 4.0);
        assert_eq!(panel.overlay_rects().len(), 1);
        let lost_rect = panel.overlay_rects()[0];
        assert!(
            lost_rect.left() >= m_lost.terminal_rect.left() - 0.5
                && lost_rect.right() <= m_lost.terminal_rect.right() + 0.5,
            "bottom overlay overflowed terminal_rect at {viewport:?}: {lost_rect:?} vs {:?}",
            m_lost.terminal_rect
        );

        // `terminal_rect`, `(rows, cols)`, `ui.min_rect()`, and `ui.cursor()` must be 100% invariant!
        for other in [&m1, &m3, &m_lost] {
            assert_eq!(other.terminal_rect, m0.terminal_rect);
            assert_eq!(other.grid_rows_cols, m0.grid_rows_cols);
            assert_eq!(other.ui_min_rect, m0.ui_min_rect);
            assert_eq!(other.ui_cursor, m0.ui_cursor);
        }

        // Direct guard: even if a caller invokes `show_header_tag` inside an unreserved narrow
        // horizontal row after a wide non-truncated label, `show_header_tag` clamps to
        // `ui.available_width()` and never expands `ui.min_rect().right()` past `viewport.x`.
        let mut raw_row_min_rect = Rect::NOTHING;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                focused: true,
                time: Some(5.0),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("staging-api-long-hostname.example.internal");
                            let _ = show_header_tag(ui, HostHeaderTag::ReadOnly, false);
                        });
                        raw_row_min_rect = ui.min_rect();
                    });
            },
        );
        assert!(
            raw_row_min_rect.right() <= viewport.x + 0.5,
            "show_header_tag overflowed narrow row at {viewport:?}: {raw_row_min_rect:?}"
        );
    }
}

#[test]
fn sanitizes_control_c1_and_bidi_characters_in_host_notes_details_and_epochs() {
    assert_eq!(sanitize_host_text("anything", 0), "");
    assert_eq!(sanitize_host_text("", MAX_NOTE_CHARS), "");
    assert_eq!(sanitize_host_text("a", 1), "a");
    assert_eq!(sanitize_host_text("abcdef", 1), "…");
    assert_eq!(sanitize_host_text("abcdef", 4), "abc…");
    assert_eq!(sanitize_host_text("  abcdef   ", 6), "abcdef");

    let hostile = "  prod\x1b[2J\x07\x00\u{009b}31m\u{202e}evil\u{2066}\r\n\tbox\u{061c}  ";
    let clean = sanitize_host_text(hostile, MAX_HOST_LABEL_CHARS);
    assert_eq!(clean, "prod[2J31mevil box");
    assert!(!clean.chars().any(|c| c.is_control()));

    // Giant payload + controls-only payload must stay strictly bounded during collection
    // (never allocating megabytes before truncating).
    let giant_text = "ω".repeat(MAX_RAW_SCAN_CHARS * 64);
    let giant_out = sanitize_host_text(&giant_text, MAX_NOTE_CHARS);
    assert_eq!(giant_out.chars().count(), MAX_NOTE_CHARS);
    assert!(giant_out.ends_with('…'));
    assert!(
        giant_out.capacity() <= 2048,
        "sanitize_host_text allocated excessive capacity ({}) on giant input",
        giant_out.capacity()
    );

    let giant_controls_only = "\x1b\x07\x00\u{009b}\u{202e}\r\n\t ".repeat(MAX_RAW_SCAN_CHARS * 8);
    let empty_controls_out = sanitize_host_text(&giant_controls_only, MAX_NOTE_CHARS);
    assert_eq!(empty_controls_out, "");
    assert_eq!(
        empty_controls_out.capacity(),
        0,
        "controls-only payload must not allocate heap storage"
    );

    let overlong_epoch = "e".repeat(MAX_EPOCH_CHARS + 20);
    assert_eq!(
        sanitize_host_text(&overlong_epoch, MAX_EPOCH_CHARS)
            .chars()
            .count(),
        MAX_EPOCH_CHARS
    );

    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));
    let mut panel = HostTerminalPanel::new();
    let vp = Vec2::new(800.0, 500.0);

    let phase = HostTerminalPhase::Lost {
        reason: LostReason::StartFailed {
            detail: "handshake\x1b]52;c;cGF5bG9hZA==\x07\u{202e}\r\nrefused\u{0085}".to_owned(),
        },
    };
    let input = HostTerminalInput {
        id_salt: Id::new("pane-sanitize"),
        host_label: "node\x1b[31m\u{202d}-01\x7f",
        epoch: Some("ep\x00\u{202e}-clean"),
        phase: &phase,
        capabilities: full_caps(),
        terminal_focused: true,
        persistence_warning: None,
        skipped_output: None,
        scrollback_lines: 0,
        french: false,
    };
    let out = settle_pane(&ctx, &mut panel, &input, vp, 1.0);
    assert_eq!(panel.observed_epoch(), Some("ep-clean"));

    for (txt, _, _) in painted_texts(&out.output.shapes) {
        assert!(
            !txt.chars().any(|c| {
                matches!(c as u32, 0x00..=0x1F | 0x7F | 0x80..=0x9F)
                    || matches!(
                        c,
                        '\u{061C}'
                            | '\u{200E}'
                            | '\u{200F}'
                            | '\u{202A}'..='\u{202E}'
                            | '\u{2066}'..='\u{2069}'
                    )
            }),
            "unsanitized control or bidi char leaked into painted text: {txt:?}"
        );
    }
}

#[test]
fn lost_and_epoch_change_never_resume_old_process_only_explicit_restart_new() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));
    let mut panel = HostTerminalPanel::new();
    let vp = Vec2::new(800.0, 500.0);

    // 1. `LostReason::Unavailable` (`TerminalEvent::Lost` without `Restarted` proof) must NEVER
    //    claim the remote daemon/service restarted!
    let lost_unavail = HostTerminalPhase::Lost {
        reason: LostReason::Unavailable,
    };
    let in_unavail_en = HostTerminalInput {
        id_salt: Id::new("pane-lost"),
        host_label: "build-box",
        epoch: Some("opaque-epoch-uuid-alpha"),
        phase: &lost_unavail,
        capabilities: full_caps(),
        terminal_focused: true,
        persistence_warning: None,
        skipped_output: None,
        scrollback_lines: 0,
        french: false,
    };
    let f_unavail_en = settle_pane(&ctx, &mut panel, &in_unavail_en, vp, 1.0);
    assert!(!panel.accepts_input(&lost_unavail));
    assert!(!panel.accepts_resize(&lost_unavail));
    assert!(has_painted_text(
        &f_unavail_en.output.shapes,
        "Remote session on build-box is no longer available — start a new session to continue."
    ));
    assert!(
        !has_painted_substring(&f_unavail_en.output.shapes, "restarted"),
        "Unavailable must never claim host service restarted without proof"
    );
    assert!(find_accesskit_button_center(&f_unavail_en.output, "Retry now").is_none());
    assert!(find_accesskit_button_center(&f_unavail_en.output, "Take control").is_none());
    assert!(find_accesskit_button_center(&f_unavail_en.output, "Terminate process").is_none());

    let in_unavail_fr = HostTerminalInput {
        french: true,
        ..in_unavail_en
    };
    let f_unavail_fr = settle_pane(&ctx, &mut panel, &in_unavail_fr, vp, 1.2);
    assert!(has_painted_text(
        &f_unavail_fr.output.shapes,
        "La session distante sur build-box n'est plus disponible — démarrez une nouvelle session pour continuer."
    ));
    assert!(
        !has_painted_substring(&f_unavail_fr.output.shapes, "redémarré"),
        "French Unavailable must never claim service redémarré without proof"
    );

    // Pressing Enter blindly in `Lost` NEVER auto-restarts or resumes!
    let enter_in_lost = step_pane_frame(
        &ctx,
        &mut panel,
        &in_unavail_en,
        vp,
        1.5,
        vec![key_ev(Key::Enter, true, false, Modifiers::NONE)],
        false,
    );
    assert_eq!(enter_in_lost.action, None);

    // Explicit click on `Start new session` emits `RestartNew`; `Close pane` emits `ClosePane`.
    let restart_pos = require_accesskit_button_center(&f_unavail_en.output, "Start new session");
    let (restart_act, _) = click_pane(&ctx, &mut panel, &in_unavail_en, vp, 1.6, restart_pos);
    assert_eq!(restart_act, Some(HostTerminalAction::RestartNew));

    let close_pos = require_accesskit_button_center(&f_unavail_en.output, "Close pane");
    let (close_act, _) = click_pane(&ctx, &mut panel, &in_unavail_en, vp, 1.7, close_pos);
    assert_eq!(close_act, Some(HostTerminalAction::ClosePane));

    // When `can_restart == false`, `Start new session` is absent in `Lost`.
    let no_restart_lost = HostTerminalInput {
        capabilities: HostTerminalCapabilities {
            can_restart: false,
            ..full_caps()
        },
        ..in_unavail_en
    };
    let f_no_restart = settle_pane(&ctx, &mut panel, &no_restart_lost, vp, 1.8);
    assert!(find_accesskit_button_center(&f_no_restart.output, "Start new session").is_none());

    // 2. Proven `EpochChanged`, `SessionGone`, and `ConnectionLost` render distinct messages.
    for (idx, (reason, expected_en, expected_fr)) in [
        (
            LostReason::EpochChanged,
            "Remote host service restarted on build-box — previous session is gone.",
            "Le service hôte distant a redémarré sur build-box — la session précédente n'existe plus.",
        ),
        (
            LostReason::SessionGone,
            "Remote session on build-box was closed.",
            "La session distante sur build-box a été fermée.",
        ),
        (
            LostReason::ConnectionLost {
                detail: Some("SSH keepalive timeout".to_owned()),
            },
            "Connection to build-box lost — SSH keepalive timeout",
            "Connexion à build-box perdue — SSH keepalive timeout",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let t_en = 2.0 + (idx as f64) * 0.1;
        let t_fr = t_en + 0.05;
        let ph = HostTerminalPhase::Lost { reason };
        let en_out = settle_pane(
            &ctx,
            &mut panel,
            &HostTerminalInput {
                phase: &ph,
                french: false,
                ..in_unavail_en
            },
            vp,
            t_en,
        );
        assert!(has_painted_text(&en_out.output.shapes, expected_en));
        let fr_out = settle_pane(
            &ctx,
            &mut panel,
            &HostTerminalInput {
                phase: &ph,
                french: true,
                ..in_unavail_en
            },
            vp,
            t_fr,
        );
        assert!(has_painted_text(&fr_out.output.shapes, expected_fr));
    }

    // 3. Contrast: in `Exited`, pressing `Enter` when `can_restart == true` emits `RestartNew`
    //    ONLY when `terminal_focused == true`. When `terminal_focused == false` (even with the OS
    //    window focused, e.g. typing in another split pane or text field), `Enter` produces NO
    //    action and is NOT consumed from `ctx.input`!
    let exited = HostTerminalPhase::Exited {
        exit_code: Some(0),
        signal: None,
    };
    let in_exited_unfocused = HostTerminalInput {
        phase: &exited,
        terminal_focused: false,
        ..in_unavail_en
    };
    let _ = settle_pane(&ctx, &mut panel, &in_exited_unfocused, vp, 2.4);
    let enter_unfocused = step_pane_frame(
        &ctx,
        &mut panel,
        &in_exited_unfocused,
        vp,
        2.45,
        vec![key_ev(Key::Enter, true, false, Modifiers::NONE)],
        false,
    );
    assert_eq!(
        enter_unfocused.action, None,
        "unfocused Exited pane must never restart when Enter is pressed elsewhere"
    );
    assert_eq!(
        enter_unfocused.unconsumed_enter_count, 1,
        "unfocused Exited pane must not consume Key::Enter from the active widget/pane"
    );

    // Likewise, typing in another pane (`terminal_focused == false`) does not flash a ReadOnly pane,
    // while an open close-choice prompt on a pane STILL owns `Escape`.
    let ro_bg = HostTerminalPhase::ReadOnly {
        reason: ReadOnlyReason::ControlledElsewhere,
    };
    let in_ro_unfocused = HostTerminalInput {
        phase: &ro_bg,
        terminal_focused: false,
        ..in_unavail_en
    };
    let _ = step_pane_frame(
        &ctx,
        &mut panel,
        &in_ro_unfocused,
        vp,
        2.47,
        vec![
            key_ev(Key::Enter, false, false, Modifiers::NONE),
            Event::Text("other-pane".to_owned()),
        ],
        false,
    );
    assert!(!panel.is_readonly_flash_active(2.48));
    assert_eq!(
        panel.request_close(&ro_bg, in_ro_unfocused.capabilities),
        None
    );
    assert!(panel.has_pending_close_choice());
    let _ = step_pane_frame(
        &ctx,
        &mut panel,
        &in_ro_unfocused,
        vp,
        2.49,
        vec![
            key_ev(Key::Escape, true, false, Modifiers::NONE),
            key_ev(Key::Escape, false, false, Modifiers::NONE),
        ],
        false,
    );
    assert!(
        !panel.has_pending_close_choice(),
        "open close-choice confirmation must still own Escape even when terminal_focused is false"
    );

    let in_exited = HostTerminalInput {
        phase: &exited,
        terminal_focused: true,
        ..in_unavail_en
    };
    let _ = settle_pane(&ctx, &mut panel, &in_exited, vp, 2.5);
    let enter_exited = step_pane_frame(
        &ctx,
        &mut panel,
        &in_exited,
        vp,
        2.6,
        vec![key_ev(Key::Enter, true, false, Modifiers::NONE)],
        false,
    );
    assert_eq!(enter_exited.action, Some(HostTerminalAction::RestartNew));
    assert_eq!(enter_exited.unconsumed_enter_count, 0);

    // 4. Opaque string `epoch` change invalidates `confirm_close_open` and resets per-epoch dismissals.
    let live = HostTerminalPhase::Live;
    let in_ep_a = HostTerminalInput {
        epoch: Some("daemon-boot-2026-11-04T01:00Z#alpha"),
        phase: &live,
        persistence_warning: Some(PersistenceWarningInput::new(
            "daemon-boot-2026-11-04T01:00Z#alpha",
            SurvivesLogout::No,
        )),
        skipped_output: Some(SkippedOutputInput::resynchronized(
            "daemon-boot-2026-11-04T01:00Z#alpha",
        )),
        ..in_unavail_en
    };
    let f_ep_a = settle_pane(&ctx, &mut panel, &in_ep_a, vp, 3.0);
    assert_eq!(panel.overlay_rects().len(), 2);

    // Dismiss both notices under epoch alpha, and open `confirm_close_open`.
    let d1 = require_accesskit_button_center(&f_ep_a.output, "Dismiss persistence notice");
    let _ = click_pane(&ctx, &mut panel, &in_ep_a, vp, 3.1, d1);
    let f_ep_a2 = settle_pane(&ctx, &mut panel, &in_ep_a, vp, 3.2);
    let d2 = require_accesskit_button_center(&f_ep_a2.output, "Dismiss output notice");
    let _ = click_pane(&ctx, &mut panel, &in_ep_a, vp, 3.3, d2);
    assert_eq!(panel.request_close(&live, in_ep_a.capabilities), None);
    assert!(panel.has_pending_close_choice());

    // Changing opaque `epoch` string to beta immediately closes stale `confirm_close_open`
    // and re-displays persistence & resynchronized notices for the new epoch!
    let in_ep_b = HostTerminalInput {
        epoch: Some("daemon-boot-2026-11-04T02:30Z#beta"),
        persistence_warning: Some(PersistenceWarningInput::new(
            "daemon-boot-2026-11-04T02:30Z#beta",
            SurvivesLogout::No,
        )),
        skipped_output: Some(SkippedOutputInput::resynchronized(
            "daemon-boot-2026-11-04T02:30Z#beta",
        )),
        ..in_ep_a
    };
    let f_ep_b = settle_pane(&ctx, &mut panel, &in_ep_b, vp, 4.0);
    assert!(!panel.has_pending_close_choice());
    assert_eq!(
        panel.observed_epoch(),
        Some("daemon-boot-2026-11-04T02:30Z#beta")
    );
    assert_eq!(
        panel.overlay_rects().len(),
        2,
        "new opaque epoch must surface fresh notices: {:?}",
        painted_texts(&f_ep_b.output.shapes)
    );
}

#[test]
fn terminate_requires_capability_and_live_or_readonly_and_opening_cancel_is_honest() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));
    let mut panel = HostTerminalPanel::new();
    let vp = Vec2::new(800.0, 500.0);

    // 1. `Opening` EVEN with `can_terminate: true` must NEVER offer `Terminate`, must NEVER open
    //    `confirm_close_open`, and must NEVER claim a remote process was created.
    let opening = HostTerminalPhase::Opening {
        note: Some("establishing SSH tunnel".to_owned()),
    };
    let in_opening = HostTerminalInput {
        id_salt: Id::new("pane-open"),
        host_label: "bastion",
        epoch: None,
        phase: &opening,
        capabilities: full_caps(),
        terminal_focused: true,
        persistence_warning: None,
        skipped_output: None,
        scrollback_lines: 0,
        french: false,
    };
    assert!(!panel.accepts_input(&opening));
    assert!(!panel.accepts_resize(&opening));
    assert_eq!(
        panel.request_close(&opening, in_opening.capabilities),
        Some(HostTerminalAction::Detach)
    );
    assert!(!panel.has_pending_close_choice());
    assert_eq!(
        panel.request_close(
            &opening,
            HostTerminalCapabilities {
                can_detach: false,
                can_terminate: true,
                ..Default::default()
            }
        ),
        Some(HostTerminalAction::ClosePane)
    );

    let f_open = settle_pane(&ctx, &mut panel, &in_opening, vp, 1.0);
    assert!(has_painted_text(
        &f_open.output.shapes,
        "Connecting to bastion — establishing SSH tunnel"
    ));
    assert!(find_accesskit_button_center(&f_open.output, "Terminate process").is_none());
    let cancel_open_pos = require_accesskit_button_center(&f_open.output, "Cancel");
    let (cancel_open_act, _) = click_pane(&ctx, &mut panel, &in_opening, vp, 1.1, cancel_open_pos);
    assert_eq!(cancel_open_act, Some(HostTerminalAction::Detach));

    // 2. `Reconnecting` with `can_terminate: true` offers `Retry now` + `Detach` (never `Terminate`).
    let reconn = HostTerminalPhase::Reconnecting {
        attempt: 2,
        max_attempts: Some(5),
    };
    let in_reconn = HostTerminalInput {
        phase: &reconn,
        ..in_opening
    };
    assert_eq!(
        panel.request_close(&reconn, in_reconn.capabilities),
        Some(HostTerminalAction::Detach)
    );
    let f_reconn = settle_pane(&ctx, &mut panel, &in_reconn, vp, 2.0);
    assert!(has_painted_text(
        &f_reconn.output.shapes,
        "Connection to bastion interrupted — reconnecting (attempt 2 of 5)…"
    ));
    assert!(find_accesskit_button_center(&f_reconn.output, "Terminate process").is_none());
    let retry_pos = require_accesskit_button_center(&f_reconn.output, "Retry now");
    let (retry_act, _) = click_pane(&ctx, &mut panel, &in_reconn, vp, 2.1, retry_pos);
    assert_eq!(retry_act, Some(HostTerminalAction::Reconnect));

    // 3. `Live` without `can_terminate` detaches directly without opening a Terminate prompt.
    let live = HostTerminalPhase::Live;
    assert_eq!(
        panel.request_close(
            &live,
            HostTerminalCapabilities {
                can_detach: true,
                can_terminate: false,
                ..Default::default()
            }
        ),
        Some(HostTerminalAction::Detach)
    );
    assert!(!panel.has_pending_close_choice());

    // 4. `Live` with `can_detach && can_terminate`: test clicking `Detach`, `Terminate process`, and `cancel_close_choice`.
    let in_live = HostTerminalInput {
        phase: &live,
        epoch: Some("ep-live-term"),
        ..in_opening
    };
    assert_eq!(panel.request_close(&live, in_live.capabilities), None);
    let f_choice = settle_pane(&ctx, &mut panel, &in_live, vp, 3.0);
    let detach_pos = require_accesskit_button_center(&f_choice.output, "Detach");
    let (detach_act, _) = click_pane(&ctx, &mut panel, &in_live, vp, 3.1, detach_pos);
    assert_eq!(detach_act, Some(HostTerminalAction::Detach));
    assert!(!panel.has_pending_close_choice());

    assert_eq!(panel.request_close(&live, in_live.capabilities), None);
    let f_choice2 = settle_pane(&ctx, &mut panel, &in_live, vp, 3.5);
    let term_pos = require_accesskit_button_center(&f_choice2.output, "Terminate process");
    let (term_act, _) = click_pane(&ctx, &mut panel, &in_live, vp, 3.6, term_pos);
    assert_eq!(term_act, Some(HostTerminalAction::Terminate));
    assert!(!panel.has_pending_close_choice());

    assert_eq!(panel.request_close(&live, in_live.capabilities), None);
    assert!(panel.has_pending_close_choice());
    panel.cancel_close_choice();
    assert!(!panel.has_pending_close_choice());
}

#[test]
fn bilingual_en_fr_parity_honest_persistence_resync_without_fake_bytes_and_all_themes() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut panel = HostTerminalPanel::new();
    let vp = Vec2::new(840.0, 520.0);
    let live = HostTerminalPhase::Live;
    let mut clock = 1.0_f64;
    let mut next_time = || {
        let current = clock;
        clock += 0.1;
        current
    };

    // 1. Header tag EN/FR labels & `Live -> None`
    assert_eq!(HostTerminalPanel::header_tag(&live), None);
    for (tag, en, fr) in [
        (HostHeaderTag::Opening, "Connecting…", "Connexion…"),
        (HostHeaderTag::ReadOnly, "Read-only", "Lecture seule"),
        (HostHeaderTag::Reconnecting, "Reconnecting…", "Reconnexion…"),
        (HostHeaderTag::Lost, "Disconnected", "Déconnecté"),
        (HostHeaderTag::Exited, "Exited", "Terminé"),
    ] {
        assert_eq!(tag.label(false), en);
        assert_eq!(tag.label(true), fr);
    }

    // 2. `SurvivesLogout::{No, Unknown}` honest EN/FR messages (zero fake tmux/screen fallback)
    for (survives, en_expected, fr_expected) in [
        (
            SurvivesLogout::No,
            "Sessions on prod-db end on logout (user lingering is disabled on this host).",
            "Les sessions sur prod-db se ferment à la déconnexion (la persistance utilisateur est désactivée sur cet hôte).",
        ),
        (
            SurvivesLogout::Unknown,
            "Logout persistence could not be verified on prod-db — this session may end when you log out.",
            "La persistance après déconnexion n'a pas pu être vérifiée sur prod-db — cette session peut s'arrêter à la déconnexion.",
        ),
    ] {
        for (fr, expected) in [(false, en_expected), (true, fr_expected)] {
            let inp = HostTerminalInput {
                id_salt: Id::new("pane-i18n"),
                host_label: "prod-db",
                epoch: Some("ep-i18n"),
                phase: &live,
                capabilities: full_caps(),
                terminal_focused: true,
                persistence_warning: Some(PersistenceWarningInput::new("ep-i18n", survives)),
                skipped_output: None,
                scrollback_lines: 0,
                french: fr,
            };
            let out = settle_pane(&ctx, &mut panel, &inp, vp, next_time());
            assert!(has_painted_text(&out.output.shapes, expected));
            assert!(!has_painted_substring(&out.output.shapes, "tmux"));
            assert!(!has_painted_substring(&out.output.shapes, "screen"));
        }
    }

    // 3. `SkippedOutput`:
    //    - `Bytes(0)` paints NO banner.
    //    - `Resynchronized` (`Snapshot.resync == true`) contains ZERO digits in EN and FR!
    //    - `Bytes(n)` formats exact B/KB/MB in EN and o/Ko/Mo in FR.
    //    - `scrollback_lines > 0` renders `Back to live output` / `Retour au direct` -> `BackToLiveOutput`.
    let zero_skip = HostTerminalInput {
        id_salt: Id::new("pane-skip"),
        host_label: "prod-db",
        epoch: Some("ep-skip"),
        phase: &live,
        capabilities: full_caps(),
        terminal_focused: true,
        persistence_warning: None,
        skipped_output: Some(SkippedOutputInput::bytes("ep-skip", 0)),
        scrollback_lines: 0,
        french: false,
    };
    let _ = settle_pane(&ctx, &mut panel, &zero_skip, vp, next_time());
    assert_eq!(
        panel.overlay_rects().len(),
        0,
        "Bytes(0) must never render a skipped-output banner"
    );

    for (fr, expected_text, expected_btn) in [
        (
            false,
            "Output resynchronized — older output while disconnected was not buffered.",
            "Back to live output",
        ),
        (
            true,
            "Sortie resynchronisée — la sortie plus ancienne produite hors ligne n'a pas été conservée.",
            "Retour au direct",
        ),
    ] {
        let resync_in = HostTerminalInput {
            skipped_output: Some(SkippedOutputInput::resynchronized("ep-skip")),
            scrollback_lines: 12,
            french: fr,
            ..zero_skip
        };
        let out = settle_pane(&ctx, &mut panel, &resync_in, vp, next_time());
        assert!(has_painted_text(&out.output.shapes, expected_text));
        assert!(
            painted_texts(&out.output.shapes)
                .iter()
                .any(|(txt, _, _)| txt == expected_text && !txt.chars().any(|c| c.is_ascii_digit())),
            "Resynchronized text must never invent byte counts: {expected_text}"
        );
        let live_btn = require_accesskit_button_center(&out.output, expected_btn);
        let (act, _) = click_pane(&ctx, &mut panel, &resync_in, vp, next_time(), live_btn);
        assert_eq!(act, Some(HostTerminalAction::BackToLiveOutput));
    }

    for (bytes, expected_en, expected_fr) in [
        (
            512_u64,
            "Skipped 512 B of earlier output while reconnecting.",
            "512 o de sortie antérieure ignorés lors de la reconnexion.",
        ),
        (
            8192_u64,
            "Skipped 8.0 KB of earlier output while reconnecting.",
            "8,0 Ko de sortie antérieure ignorés lors de la reconnexion.",
        ),
        (
            2_097_152_u64,
            "Skipped 2.0 MB of earlier output while reconnecting.",
            "2,0 Mo de sortie antérieure ignorés lors de la reconnexion.",
        ),
    ] {
        assert_eq!(
            SkippedOutputInput::bytes("ep-b", bytes).skipped,
            SkippedOutput::Bytes(bytes)
        );
        let en_out = settle_pane(
            &ctx,
            &mut panel,
            &HostTerminalInput {
                skipped_output: Some(SkippedOutputInput::bytes("ep-b", bytes)),
                french: false,
                ..zero_skip
            },
            vp,
            next_time(),
        );
        assert!(has_painted_text(&en_out.output.shapes, expected_en));
        let fr_out = settle_pane(
            &ctx,
            &mut panel,
            &HostTerminalInput {
                skipped_output: Some(SkippedOutputInput::bytes("ep-b", bytes)),
                french: true,
                ..zero_skip
            },
            vp,
            next_time(),
        );
        assert!(has_painted_text(&fr_out.output.shapes, expected_fr));
    }

    // 4. Render across all 7 named dark themes + light mode without panic or missing geometry.
    let sig_exit = HostTerminalPhase::Exited {
        exit_code: None,
        signal: Some(15),
    };
    for theme_id in THEMES {
        theme::apply(&ctx, true, Some(theme_id));
        let out = settle_pane(
            &ctx,
            &mut panel,
            &HostTerminalInput {
                phase: &sig_exit,
                french: true,
                ..zero_skip
            },
            vp,
            next_time(),
        );
        assert!(has_painted_text(
            &out.output.shapes,
            "Session terminée (signal 15) — appuyez sur Entrée ou démarrez une nouvelle session"
        ));
    }
    theme::apply(&ctx, false, None);
    let light_out = settle_pane(
        &ctx,
        &mut panel,
        &HostTerminalInput {
            phase: &sig_exit,
            french: false,
            ..zero_skip
        },
        vp,
        next_time(),
    );
    assert!(has_painted_text(
        &light_out.output.shapes,
        "Session ended (signal 15) — press Enter or start a new session"
    ));
}
