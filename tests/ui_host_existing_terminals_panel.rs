//! Standalone headless `egui` + `AccessKit` tests for `src/host_existing_terminals_panel.rs`
//! (Gate 7B modal picker for `terminal.list`: exact raw protocol identity, stale/disconnected
//! epoch gating, exited-row safety, zero implicit `create`, pagination, `Escape` isolation, and
//! EN/FR + multi-theme parity).

#[path = "../src/host_existing_terminals_panel.rs"]
mod host_existing_terminals_panel;
#[allow(dead_code)]
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
use host_existing_terminals_panel::{
    ExistingTerminalEntry, ExistingTerminalExit, ExistingTerminalsAction, ExistingTerminalsError,
    ExistingTerminalsInput, ExistingTerminalsState, HostExistingTerminalsPanel,
    MAX_LISTED_TERMINALS, MAX_TERMINAL_ID_DISPLAY_CHARS, PAGE_SIZE,
};
use host_terminal_panel::sanitize_host_text;
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

fn all_accesskit_button_labels(out: &egui::FullOutput) -> Vec<String> {
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

fn is_forbidden_display_char(ch: char) -> bool {
    let cp = ch as u32;
    ch.is_control()
        || matches!(
            cp,
            0x061C | 0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069
        )
}

struct ModalFrame {
    action: Option<ExistingTerminalsAction>,
    forwarded_escape_count: usize,
    popup_open: bool,
    output: egui::FullOutput,
}

fn step_modal_frame(
    ctx: &egui::Context,
    panel: &mut HostExistingTerminalsPanel,
    input: Option<&ExistingTerminalsInput<'_>>,
    viewport: Vec2,
    time: f64,
    events: Vec<Event>,
    with_context_menu: bool,
) -> ModalFrame {
    let modal_open = input.is_some();
    let mut action = None;
    let mut forwarded_escape_count = 0_usize;
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
            let pre_esc = panel.handle_escape(ctx, modal_open);

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let (_, term_resp) =
                        ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
                    if with_context_menu {
                        term_resp.context_menu(|ui| {
                            if ui.button("Copy visible text").clicked() {
                                ui.close();
                            }
                        });
                    }
                    // Simulated underlying Live terminal: forwards Escape only when modal is closed
                    // and `escape_held` is not latched.
                    if !modal_open && !panel.is_escape_held() {
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
                });

            let modal_act = input.and_then(|inp| panel.show_modal(ctx, inp));
            action = pre_esc.or(modal_act);
            popup_open = egui::Popup::is_any_open(ctx);
        },
    );

    ModalFrame {
        action,
        forwarded_escape_count,
        popup_open,
        output,
    }
}

fn settle_modal(
    ctx: &egui::Context,
    panel: &mut HostExistingTerminalsPanel,
    input: &ExistingTerminalsInput<'_>,
    viewport: Vec2,
    time: f64,
) -> ModalFrame {
    let _ = step_modal_frame(ctx, panel, Some(input), viewport, time, vec![], false);
    let _ = step_modal_frame(
        ctx,
        panel,
        Some(input),
        viewport,
        time + 0.012,
        vec![],
        false,
    );
    step_modal_frame(
        ctx,
        panel,
        Some(input),
        viewport,
        time + 0.024,
        vec![],
        false,
    )
}

fn click_modal(
    ctx: &egui::Context,
    panel: &mut HostExistingTerminalsPanel,
    input: &ExistingTerminalsInput<'_>,
    viewport: Vec2,
    time: f64,
    pos: Pos2,
) -> (Option<ExistingTerminalsAction>, ModalFrame) {
    let f1 = step_modal_frame(
        ctx,
        panel,
        Some(input),
        viewport,
        time,
        pointer_down(pos, PointerButton::Primary),
        false,
    );
    let mut f2 = step_modal_frame(
        ctx,
        panel,
        Some(input),
        viewport,
        time + 0.016,
        pointer_up(pos, PointerButton::Primary),
        false,
    );
    let action = f1.action.or_else(|| f2.action.take());
    (action, f2)
}

fn press_key(
    ctx: &egui::Context,
    panel: &mut HostExistingTerminalsPanel,
    input: &ExistingTerminalsInput<'_>,
    viewport: Vec2,
    time: f64,
    key: Key,
) -> (Option<ExistingTerminalsAction>, ModalFrame) {
    let f1 = step_modal_frame(
        ctx,
        panel,
        Some(input),
        viewport,
        time,
        vec![key_ev(key, true, false, Modifiers::NONE)],
        false,
    );
    let mut f2 = step_modal_frame(
        ctx,
        panel,
        Some(input),
        viewport,
        time + 0.016,
        vec![key_ev(key, false, false, Modifiers::NONE)],
        false,
    );
    let action = f1.action.or_else(|| f2.action.take());
    (action, f2)
}

fn assert_modal_within_viewport(panel: &HostExistingTerminalsPanel, viewport: Vec2) {
    let rect = panel
        .last_modal_rect()
        .expect("modal rect should be recorded when shown");
    assert!(rect.width() > 0.0 && rect.height() > 0.0);
    assert!(
        rect.min.x >= -0.5
            && rect.min.y >= -0.5
            && rect.max.x <= viewport.x + 0.5
            && rect.max.y <= viewport.y + 0.5,
        "modal card {rect:?} overflowed viewport {viewport:?}"
    );
}

#[test]
fn pagination_cap_64_page_16_keyboard_nav_and_viewport_containment() {
    assert_eq!(MAX_LISTED_TERMINALS, 64);
    assert_eq!(PAGE_SIZE, 16);

    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));

    let ids: Vec<String> = (0..80).map(|i| format!("term-{i:03}")).collect();
    let entries: Vec<ExistingTerminalEntry<'_>> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| ExistingTerminalEntry {
            terminal_id: id.as_str(),
            pid: 4000 + i as u32,
            size: (24, 80),
            attached: (i % 3) as u32,
            leased_on_host: i % 2 == 0,
            retained_bytes: match i % 3 {
                0 => 512,
                1 => 32 * 1024,
                _ => 3 * 1024 * 1024,
            },
            exited: None,
            has_saved_lease: i == 0,
            open_locally: false,
        })
        .collect();

    let mut panel = HostExistingTerminalsPanel::new();
    let vp_wide = Vec2::new(800.0, 500.0);
    let input_en = ExistingTerminalsInput {
        id_salt: Id::new("picker-cap"),
        host_label: "prod-eu-1",
        live_epoch: Some("epoch-live-1"),
        state: ExistingTerminalsState::Loaded {
            list_epoch: "epoch-live-1",
            entries: &entries,
        },
        can_attach: true,
        can_open_new: true,
        french: false,
    };

    let f0 = settle_modal(&ctx, &mut panel, &input_en, vp_wide, 1.0);
    assert_modal_within_viewport(&panel, vp_wide);
    assert_eq!(panel.selected_index(), 0);
    assert_eq!(panel.current_page(), 0);
    assert!(has_painted_substring(
        &f0.output.shapes,
        "Showing the first 64 terminals reported by the host."
    ));
    assert!(has_painted_text(&f0.output.shapes, "Page 1 of 4"));
    assert!(has_painted_text(&f0.output.shapes, "#term-000"));
    assert!(!has_painted_text(&f0.output.shapes, "#term-016"));
    assert!(!has_painted_text(&f0.output.shapes, "#term-064"));

    // Clicking `#term-002` selects index 2 without triggering an attach action.
    let row2_pos = find_painted_text_center(&f0.output.shapes, "#term-002")
        .expect("#term-002 should be visible near top of page 1");
    let (click_sel_act, _) = click_modal(&ctx, &mut panel, &input_en, vp_wide, 1.1, row2_pos);
    assert_eq!(click_sel_act, None);
    assert_eq!(panel.selected_index(), 2);

    // Navigate down to index 15 (still page 0), then 16 -> flips to page 1 (`Page 2 of 4`).
    for step in 0..13 {
        let _ = press_key(
            &ctx,
            &mut panel,
            &input_en,
            vp_wide,
            1.2 + f64::from(step) * 0.04,
            Key::ArrowDown,
        );
    }
    assert_eq!(panel.selected_index(), 15);
    assert_eq!(panel.current_page(), 0);

    let (_, f_p2) = press_key(&ctx, &mut panel, &input_en, vp_wide, 2.0, Key::ArrowDown);
    assert_eq!(panel.selected_index(), 16);
    assert_eq!(panel.current_page(), 1);
    assert!(has_painted_text(&f_p2.output.shapes, "Page 2 of 4"));
    assert!(has_painted_text(&f_p2.output.shapes, "#term-016"));

    // ArrowUp crosses back from 16 -> 15 (`Page 1 of 4`).
    let (_, f_back) = press_key(&ctx, &mut panel, &input_en, vp_wide, 2.1, Key::ArrowUp);
    assert_eq!(panel.selected_index(), 15);
    assert_eq!(panel.current_page(), 0);
    assert!(has_painted_text(&f_back.output.shapes, "Page 1 of 4"));

    // PageDown -> 31 (page 1), End -> 63 (page 3, clamped strictly at MAX_LISTED_TERMINALS - 1).
    let _ = press_key(&ctx, &mut panel, &input_en, vp_wide, 2.2, Key::PageDown);
    assert_eq!(panel.selected_index(), 31);
    assert_eq!(panel.current_page(), 1);

    let (_, f_end) = press_key(&ctx, &mut panel, &input_en, vp_wide, 2.3, Key::End);
    assert_eq!(panel.selected_index(), 63);
    assert_eq!(panel.current_page(), 3);
    assert!(has_painted_text(&f_end.output.shapes, "Page 4 of 4"));
    assert!(!has_painted_text(&f_end.output.shapes, "#term-064"));

    // PageUp -> 47 (page 2), Home -> 0 (page 0).
    let _ = press_key(&ctx, &mut panel, &input_en, vp_wide, 2.4, Key::PageUp);
    assert_eq!(panel.selected_index(), 47);
    assert_eq!(panel.current_page(), 2);

    let (_, f_home) = press_key(&ctx, &mut panel, &input_en, vp_wide, 2.5, Key::Home);
    assert_eq!(panel.selected_index(), 0);
    assert_eq!(panel.current_page(), 0);

    // Pagination buttons (`Next` / `Previous`) work via AccessKit.
    let next_pos = require_accesskit_button_center(&f_home.output, "Next");
    let _ = click_modal(&ctx, &mut panel, &input_en, vp_wide, 2.6, next_pos);
    let f_next = settle_modal(&ctx, &mut panel, &input_en, vp_wide, 2.7);
    assert_eq!(panel.current_page(), 1);
    assert_eq!(panel.selected_index(), 16);
    let prev_pos = require_accesskit_button_center(&f_next.output, "Previous");
    let _ = click_modal(&ctx, &mut panel, &input_en, vp_wide, 2.8, prev_pos);
    let _ = settle_modal(&ctx, &mut panel, &input_en, vp_wide, 2.9);
    assert_eq!(panel.current_page(), 0);
    assert_eq!(panel.selected_index(), 0);

    // Move to page 2, then verify `reset_selection()` restores index 0 & page 0.
    let _ = press_key(&ctx, &mut panel, &input_en, vp_wide, 3.0, Key::End);
    assert_eq!(panel.current_page(), 3);
    panel.reset_selection();
    assert_eq!(panel.selected_index(), 0);
    assert_eq!(panel.current_page(), 0);

    // Narrow 280x320 French viewport with BOTH stale-epoch banner AND 64-entry truncation banner:
    // card must stay 100% contained inside 280x320!
    let vp_narrow = Vec2::new(280.0, 320.0);
    let input_fr_narrow = ExistingTerminalsInput {
        live_epoch: Some("epoch-restarted-2"),
        french: true,
        ..input_en
    };
    let f_narrow = settle_modal(&ctx, &mut panel, &input_fr_narrow, vp_narrow, 3.2);
    assert_modal_within_viewport(&panel, vp_narrow);
    assert!(has_painted_text(&f_narrow.output.shapes, "Page 1 sur 4"));
    assert!(
        find_accesskit_button_center(&f_narrow.output, "Actualiser").is_some(),
        "narrow viewport must expose footer Actualiser button"
    );
}

#[test]
fn exact_raw_identity_preserved_while_display_sanitizes_and_observer_vs_resume_honest() {
    assert_eq!(MAX_TERMINAL_ID_DISPLAY_CHARS, 24);

    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));

    // Raw protocol strings deliberately contain leading/trailing spaces, tabs, ANSI CSI, bidi
    // overrides, and > 24 scalar chars so any accidental normalization or truncation in emitted
    // actions would immediately fail equality assertions.
    let raw_epoch = "  ep-raw\t\u{202e}#01  ";
    let raw_id_saved = "  term-saved-\x1b[31m\u{202e}-very-long-raw-identifier-0001  ";
    let raw_id_elsewhere = "term-elsewhere\n\u{2066}#02";
    let raw_id_local = "term-local\r\t#03";

    let entries = [
        ExistingTerminalEntry {
            terminal_id: raw_id_saved,
            pid: 1010,
            size: (30, 120),
            attached: 1,
            leased_on_host: true,
            retained_bytes: 4096,
            exited: None,
            has_saved_lease: true,
            open_locally: false,
        },
        ExistingTerminalEntry {
            terminal_id: raw_id_elsewhere,
            pid: 2020,
            size: (24, 80),
            attached: 2,
            leased_on_host: true,
            retained_bytes: 800,
            exited: None,
            has_saved_lease: false,
            open_locally: false,
        },
        ExistingTerminalEntry {
            terminal_id: raw_id_local,
            pid: 3030,
            size: (40, 160),
            attached: 1,
            leased_on_host: true,
            retained_bytes: 2 * 1024 * 1024,
            exited: None,
            has_saved_lease: true,
            open_locally: true,
        },
    ];

    let mut panel = HostExistingTerminalsPanel::new();
    let vp = Vec2::new(800.0, 520.0);
    let input = ExistingTerminalsInput {
        id_salt: Id::new("picker-raw-id"),
        host_label: "  host-\x1b[2J\u{202e}prod  ",
        live_epoch: Some(raw_epoch),
        state: ExistingTerminalsState::Loaded {
            list_epoch: raw_epoch,
            entries: &entries,
        },
        can_attach: true,
        can_open_new: false,
        french: false,
    };

    let f0 = settle_modal(&ctx, &mut panel, &input, vp, 1.0);
    assert_modal_within_viewport(&panel, vp);

    // 1. Rendered texts and AccessKit labels must NEVER contain control or bidi characters, and
    //    `raw_id_saved` display label must be truncated to `#` + 24 scalars.
    for (text, _, _) in painted_texts(&f0.output.shapes) {
        assert!(
            !text.chars().any(is_forbidden_display_char),
            "unsanitized control/bidi char leaked into painted text: {text:?}"
        );
    }
    let btn_labels = all_accesskit_button_labels(&f0.output);
    for lbl in &btn_labels {
        assert!(
            !lbl.chars().any(is_forbidden_display_char),
            "unsanitized control/bidi char leaked into AccessKit label: {lbl:?}"
        );
    }

    let disp_saved = sanitize_host_text(raw_id_saved, MAX_TERMINAL_ID_DISPLAY_CHARS);
    let disp_elsewhere = sanitize_host_text(raw_id_elsewhere, MAX_TERMINAL_ID_DISPLAY_CHARS);
    let disp_local = sanitize_host_text(raw_id_local, MAX_TERMINAL_ID_DISPLAY_CHARS);
    assert_eq!(disp_saved.chars().count(), MAX_TERMINAL_ID_DISPLAY_CHARS);
    assert!(has_painted_text(
        &f0.output.shapes,
        &format!("#{disp_saved}")
    ));
    assert!(has_painted_text(&f0.output.shapes, "Saved access"));
    assert!(has_painted_text(&f0.output.shapes, "Controlled elsewhere"));
    assert!(has_painted_text(&f0.output.shapes, "Open here"));

    // Row 0 (`has_saved_lease: true`) exposes BOTH `Resume` and `Observe`;
    // Row 1 (`has_saved_lease: false, leased_on_host: true`) exposes ONLY `Observe` (never `Resume`);
    // Row 2 (`open_locally: true`) exposes ONLY `Switch to pane`.
    let resume_0 = require_accesskit_button_center(&f0.output, &format!("Resume #{disp_saved}"));
    let observe_0 = require_accesskit_button_center(&f0.output, &format!("Observe #{disp_saved}"));
    let observe_1 =
        require_accesskit_button_center(&f0.output, &format!("Observe #{disp_elsewhere}"));
    assert!(
        find_accesskit_button_center(&f0.output, &format!("Resume #{disp_elsewhere}")).is_none(),
        "row without local saved lease must never offer Resume"
    );
    let focus_2 =
        require_accesskit_button_center(&f0.output, &format!("Switch to pane #{disp_local}"));

    // 2. Clicking each button emits the EXACT raw `epoch` & `terminal_id` byte-for-byte.
    let (act_resume_click, _) = click_modal(&ctx, &mut panel, &input, vp, 1.1, resume_0);
    assert_eq!(
        act_resume_click,
        Some(ExistingTerminalsAction::ResumeSaved {
            epoch: raw_epoch.to_owned(),
            terminal_id: raw_id_saved.to_owned(),
        })
    );

    let f_after_resume = settle_modal(&ctx, &mut panel, &input, vp, 1.2);
    let obs0_pos =
        require_accesskit_button_center(&f_after_resume.output, &format!("Observe #{disp_saved}"));
    let (act_obs0_click, _) = click_modal(&ctx, &mut panel, &input, vp, 1.3, obs0_pos);
    assert_eq!(
        act_obs0_click,
        Some(ExistingTerminalsAction::AttachObserver {
            epoch: raw_epoch.to_owned(),
            terminal_id: raw_id_saved.to_owned(),
        })
    );

    let (act_obs1_click, _) = click_modal(&ctx, &mut panel, &input, vp, 1.4, observe_1);
    assert_eq!(
        act_obs1_click,
        Some(ExistingTerminalsAction::AttachObserver {
            epoch: raw_epoch.to_owned(),
            terminal_id: raw_id_elsewhere.to_owned(),
        })
    );

    let (act_focus_click, _) = click_modal(&ctx, &mut panel, &input, vp, 1.5, focus_2);
    assert_eq!(
        act_focus_click,
        Some(ExistingTerminalsAction::FocusLocalTerminal {
            epoch: raw_epoch.to_owned(),
            terminal_id: raw_id_local.to_owned(),
        })
    );
    let _ = observe_0;

    // 3. Keyboard `Enter` on Row 0 -> `ResumeSaved`, Row 1 -> `AttachObserver`, Row 2 -> `FocusLocalTerminal`
    //    with exact raw protocol strings.
    panel.reset_selection();
    let _ = settle_modal(&ctx, &mut panel, &input, vp, 1.6);
    let (enter_0, _) = press_key(&ctx, &mut panel, &input, vp, 1.7, Key::Enter);
    assert_eq!(
        enter_0,
        Some(ExistingTerminalsAction::ResumeSaved {
            epoch: raw_epoch.to_owned(),
            terminal_id: raw_id_saved.to_owned(),
        })
    );
    let _ = press_key(&ctx, &mut panel, &input, vp, 1.8, Key::ArrowDown);
    let (enter_1, _) = press_key(&ctx, &mut panel, &input, vp, 1.9, Key::Enter);
    assert_eq!(
        enter_1,
        Some(ExistingTerminalsAction::AttachObserver {
            epoch: raw_epoch.to_owned(),
            terminal_id: raw_id_elsewhere.to_owned(),
        })
    );
    let _ = press_key(&ctx, &mut panel, &input, vp, 2.0, Key::ArrowDown);
    let (enter_2, _) = press_key(&ctx, &mut panel, &input, vp, 2.1, Key::Enter);
    assert_eq!(
        enter_2,
        Some(ExistingTerminalsAction::FocusLocalTerminal {
            epoch: raw_epoch.to_owned(),
            terminal_id: raw_id_local.to_owned(),
        })
    );

    // 4. Local capacity full (`can_attach: false`) with valid `epoch`:
    //    blocks `Resume` and `Observe` on remote rows (with honest EN/FR notice), while still
    //    allowing `FocusLocalTerminal` on already-open local panes!
    let input_no_cap_fr = ExistingTerminalsInput {
        can_attach: false,
        french: true,
        ..input
    };
    panel.reset_selection();
    let f_cap = settle_modal(&ctx, &mut panel, &input_no_cap_fr, vp, 2.3);
    assert!(has_painted_substring(
        &f_cap.output.shapes,
        "Fermez un autre volet de terminal distant localement"
    ));
    assert!(has_painted_text(&f_cap.output.shapes, "Accès enregistré"));
    assert!(has_painted_text(&f_cap.output.shapes, "Contrôlé ailleurs"));
    assert!(has_painted_text(&f_cap.output.shapes, "Ouvert ici"));
    assert!(
        find_accesskit_button_center(&f_cap.output, &format!("Reprendre #{disp_saved}")).is_none()
    );
    assert!(
        find_accesskit_button_center(&f_cap.output, &format!("Observer #{disp_elsewhere}"))
            .is_none()
    );
    let (enter_blocked, _) = press_key(&ctx, &mut panel, &input_no_cap_fr, vp, 2.4, Key::Enter);
    assert_eq!(enter_blocked, None);

    let focus_fr =
        require_accesskit_button_center(&f_cap.output, &format!("Aller au volet #{disp_local}"));
    let (act_focus_fr, _) = click_modal(&ctx, &mut panel, &input_no_cap_fr, vp, 2.5, focus_fr);
    assert_eq!(
        act_focus_fr,
        Some(ExistingTerminalsAction::FocusLocalTerminal {
            epoch: raw_epoch.to_owned(),
            terminal_id: raw_id_local.to_owned(),
        })
    );
}

#[test]
fn stale_or_missing_live_epoch_disables_attach_resume_and_focus_local() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));

    let entries = [
        ExistingTerminalEntry {
            terminal_id: "term-local-1",
            pid: 111,
            size: (24, 80),
            attached: 1,
            leased_on_host: true,
            retained_bytes: 128,
            exited: None,
            has_saved_lease: true,
            open_locally: true,
        },
        ExistingTerminalEntry {
            terminal_id: "term-remote-2",
            pid: 222,
            size: (24, 80),
            attached: 0,
            leased_on_host: false,
            retained_bytes: 256,
            exited: None,
            has_saved_lease: true,
            open_locally: false,
        },
    ];

    let mut panel = HostExistingTerminalsPanel::new();
    let vp = Vec2::new(800.0, 500.0);

    // Case A: daemon restarted (`live_epoch = Some("epoch-b")` != `list_epoch = "epoch-a"`),
    // Case B: whitespace-only mismatch (`live_epoch = Some("epoch-a")` != `list_epoch = " epoch-a "`),
    // Case C: disconnected (`live_epoch = None`).
    let cases: [(&str, Option<&str>, bool, &str); 4] = [
        (
            "epoch-a",
            Some("epoch-b"),
            false,
            "Remote host service restarted on prod-1",
        ),
        (
            " epoch-a ",
            Some("epoch-a"),
            true,
            "Le service hôte distant a redémarré sur prod-1",
        ),
        (
            "epoch-a",
            None,
            false,
            "Connection to prod-1 is interrupted",
        ),
        (
            "epoch-a",
            None,
            true,
            "La connexion à prod-1 est interrompue",
        ),
    ];

    for (idx, (list_ep, live_ep, fr, expected_banner)) in cases.into_iter().enumerate() {
        panel.reset_selection();
        let input = ExistingTerminalsInput {
            id_salt: Id::new(("stale-case", idx)),
            host_label: "prod-1",
            live_epoch: live_ep,
            state: ExistingTerminalsState::Loaded {
                list_epoch: list_ep,
                entries: &entries,
            },
            can_attach: true,
            can_open_new: true,
            french: fr,
        };
        let t = 1.0 + (idx as f64) * 0.5;
        let f = settle_modal(&ctx, &mut panel, &input, vp, t);
        assert!(
            has_painted_substring(&f.output.shapes, expected_banner),
            "missing stale/disconnected notice `{expected_banner}`"
        );

        let btns = all_accesskit_button_labels(&f.output);
        for forbidden in [
            "Switch to pane",
            "Aller au volet",
            "Resume",
            "Reprendre",
            "Observe",
            "Observer",
            "New terminal",
            "Nouveau terminal",
        ] {
            assert!(
                !btns.iter().any(|b| b.contains(forbidden)),
                "stale/disconnected list must disable `{forbidden}`, found buttons: {btns:?}"
            );
        }

        // Keyboard `Enter` on both Row 0 (`open_locally`) and Row 1 (`has_saved_lease`) must return `None`.
        let (e0, _) = press_key(&ctx, &mut panel, &input, vp, t + 0.05, Key::Enter);
        assert_eq!(e0, None);
        let _ = press_key(&ctx, &mut panel, &input, vp, t + 0.10, Key::ArrowDown);
        let (e1, _) = press_key(&ctx, &mut panel, &input, vp, t + 0.15, Key::Enter);
        assert_eq!(e1, None);

        // `Refresh` / `Actualiser` remains available and emits `ExistingTerminalsAction::Refresh`.
        let refresh_lbl = if fr { "Actualiser" } else { "Refresh" };
        let r_pos = require_accesskit_button_center(&f.output, refresh_lbl);
        let (r_act, _) = click_modal(&ctx, &mut panel, &input, vp, t + 0.20, r_pos);
        assert_eq!(r_act, Some(ExistingTerminalsAction::Refresh));
    }
}

#[test]
fn exited_rows_forbid_resume_even_with_saved_lease_and_allow_only_final_output_observer() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));

    let raw_epoch = "ep-linger-77";
    let entries = [
        ExistingTerminalEntry {
            terminal_id: "exited-code-0",
            pid: 501,
            size: (24, 80),
            attached: 0,
            leased_on_host: false,
            retained_bytes: 1024,
            exited: Some(ExistingTerminalExit {
                exit_code: Some(0),
                signal: None,
            }),
            has_saved_lease: true,
            open_locally: false,
        },
        ExistingTerminalEntry {
            terminal_id: "exited-sig-9",
            pid: 502,
            size: (24, 80),
            attached: 0,
            leased_on_host: true,
            retained_bytes: 2048,
            exited: Some(ExistingTerminalExit {
                exit_code: None,
                signal: Some(9),
            }),
            has_saved_lease: true,
            open_locally: false,
        },
        ExistingTerminalEntry {
            terminal_id: "exited-plain",
            pid: 503,
            size: (24, 80),
            attached: 0,
            leased_on_host: false,
            retained_bytes: 64,
            exited: Some(ExistingTerminalExit {
                exit_code: None,
                signal: None,
            }),
            has_saved_lease: true,
            open_locally: false,
        },
    ];

    let mut panel = HostExistingTerminalsPanel::new();
    let vp = Vec2::new(800.0, 500.0);
    let mut clock = 1.0_f64;
    let mut next_t = || {
        let cur = clock;
        clock += 0.1;
        cur
    };

    for (fr, expected_badges, final_prefix) in [
        (
            false,
            ["Exited (code 0)", "Exited (signal 9)", "Exited"],
            "View final output",
        ),
        (
            true,
            ["Terminé (code 0)", "Terminé (signal 9)", "Terminé"],
            "Voir la sortie finale",
        ),
    ] {
        panel.reset_selection();
        let input = ExistingTerminalsInput {
            id_salt: Id::new(("exited-suite", fr)),
            host_label: "staging-2",
            live_epoch: Some(raw_epoch),
            state: ExistingTerminalsState::Loaded {
                list_epoch: raw_epoch,
                entries: &entries,
            },
            can_attach: true,
            can_open_new: false,
            french: fr,
        };
        let f = settle_modal(&ctx, &mut panel, &input, vp, next_t());
        for badge in expected_badges {
            assert!(
                has_painted_text(&f.output.shapes, badge),
                "missing exited badge `{badge}`"
            );
        }
        assert!(!has_painted_text(&f.output.shapes, "Saved access"));
        assert!(!has_painted_text(&f.output.shapes, "Accès enregistré"));

        let btns = all_accesskit_button_labels(&f.output);
        assert!(
            !btns
                .iter()
                .any(|b| b.contains("Resume") || b.contains("Reprendre")),
            "exited rows must never offer Resume/Reprendre even with saved lease: {btns:?}"
        );

        // Bare `Enter` on an exited row must never auto-attach or spawn.
        let (enter_act, _) = press_key(&ctx, &mut panel, &input, vp, next_t(), Key::Enter);
        assert_eq!(enter_act, None);

        // Explicit click on `View final output` attaches read-only (`AttachObserver`).
        let btn_pos =
            require_accesskit_button_center(&f.output, &format!("{final_prefix} #exited-sig-9"));
        let (click_act, _) = click_modal(&ctx, &mut panel, &input, vp, next_t(), btn_pos);
        assert_eq!(
            click_act,
            Some(ExistingTerminalsAction::AttachObserver {
                epoch: raw_epoch.to_owned(),
                terminal_id: "exited-sig-9".to_owned(),
            })
        );
    }
}

#[test]
fn empty_loading_and_fixed_errors_never_auto_create_and_gate_open_new() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));

    let mut panel = HostExistingTerminalsPanel::new();
    let vp = Vec2::new(800.0, 500.0);
    let mut clock = 1.0_f64;
    let mut next_t = || {
        let cur = clock;
        clock += 0.1;
        cur
    };

    // 1. `Loading` in EN & FR: `Enter` returns `None`, `New terminal` never offered, `Cancel` /
    //    header `Close dialog` emit `Close`.
    for (fr, loading_sub, cancel_lbl, hdr_close_lbl) in [
        (
            false,
            "Loading existing terminals on build-box",
            "Cancel",
            "Close dialog",
        ),
        (
            true,
            "Chargement des terminaux existants sur build-box",
            "Annuler",
            "Fermer la fenêtre",
        ),
    ] {
        let input = ExistingTerminalsInput {
            id_salt: Id::new(("loading", fr)),
            host_label: "build-box",
            live_epoch: Some("ep-1"),
            state: ExistingTerminalsState::Loading,
            can_attach: true,
            can_open_new: true,
            french: fr,
        };
        let f = settle_modal(&ctx, &mut panel, &input, vp, next_t());
        assert!(has_painted_substring(&f.output.shapes, loading_sub));
        let btns = all_accesskit_button_labels(&f.output);
        assert!(
            !btns
                .iter()
                .any(|b| b.contains("New terminal") || b.contains("Nouveau terminal"))
        );

        let (enter_act, _) = press_key(&ctx, &mut panel, &input, vp, next_t(), Key::Enter);
        assert_eq!(enter_act, None);

        let f_cancel = settle_modal(&ctx, &mut panel, &input, vp, next_t());
        let cancel_pos = require_accesskit_button_center(&f_cancel.output, cancel_lbl);
        let (c_act, _) = click_modal(&ctx, &mut panel, &input, vp, next_t(), cancel_pos);
        assert_eq!(c_act, Some(ExistingTerminalsAction::Close));

        let f2 = settle_modal(&ctx, &mut panel, &input, vp, next_t());
        let hdr_pos = require_accesskit_button_center(&f2.output, hdr_close_lbl);
        let (h_act, _) = click_modal(&ctx, &mut panel, &input, vp, next_t(), hdr_pos);
        assert_eq!(h_act, Some(ExistingTerminalsAction::Close));
    }

    // 2. All 5 fixed `ExistingTerminalsError` variants in EN & FR: never offer `New terminal` even
    //    when `can_open_new: true`, never trigger on `Enter`, and expose `Refresh` + `Close`.
    let err_cases = [
        (
            ExistingTerminalsError::NotConnected,
            "Could not reach build-box",
            "Impossible de joindre build-box",
        ),
        (
            ExistingTerminalsError::TimedOut,
            "Timed out waiting for build-box",
            "Délai dépassé en attendant la liste",
        ),
        (
            ExistingTerminalsError::EpochChanged,
            "Remote host service restarted on build-box",
            "Le service hôte distant a redémarré sur build-box",
        ),
        (
            ExistingTerminalsError::Unsupported,
            "does not support listing terminals",
            "ne prend pas en charge la liste des terminaux",
        ),
        (
            ExistingTerminalsError::RequestFailed,
            "Could not list existing terminals on build-box",
            "Impossible de lister les terminaux existants sur build-box",
        ),
    ];

    for (idx, (reason, en_sub, fr_sub)) in err_cases.into_iter().enumerate() {
        for (fr, expected_sub, refresh_lbl, close_lbl) in [
            (false, en_sub, "Refresh", "Close"),
            (true, fr_sub, "Actualiser", "Fermer"),
        ] {
            let input = ExistingTerminalsInput {
                id_salt: Id::new(("err", idx, fr)),
                host_label: "build-box",
                live_epoch: Some("ep-1"),
                state: ExistingTerminalsState::Error { reason },
                can_attach: true,
                can_open_new: true,
                french: fr,
            };
            let f = settle_modal(&ctx, &mut panel, &input, vp, next_t());
            assert!(
                has_painted_substring(&f.output.shapes, expected_sub),
                "missing translated error copy `{expected_sub}` for {reason:?}"
            );
            let btns = all_accesskit_button_labels(&f.output);
            assert!(
                !btns
                    .iter()
                    .any(|b| b.contains("New terminal") || b.contains("Nouveau terminal")),
                "Error state must never offer OpenNewTerminal: {btns:?}"
            );
            let (e_act, _) = press_key(&ctx, &mut panel, &input, vp, next_t(), Key::Enter);
            assert_eq!(e_act, None);

            let f_ref = settle_modal(&ctx, &mut panel, &input, vp, next_t());
            let r_pos = require_accesskit_button_center(&f_ref.output, refresh_lbl);
            let (r_act, _) = click_modal(&ctx, &mut panel, &input, vp, next_t(), r_pos);
            assert_eq!(r_act, Some(ExistingTerminalsAction::Refresh));

            let f_close = settle_modal(&ctx, &mut panel, &input, vp, next_t());
            let c_pos = require_accesskit_button_center(&f_close.output, close_lbl);
            let (c_act, _) = click_modal(&ctx, &mut panel, &input, vp, next_t(), c_pos);
            assert_eq!(c_act, Some(ExistingTerminalsAction::Close));
        }
    }

    // 3. Empty `Loaded` (`entries: &[]`): `Enter` NEVER auto-creates; `OpenNewTerminal` is offered
    //    only when `can_open_new && epoch_valid` and only on explicit button click.
    let empty: [ExistingTerminalEntry<'_>; 0] = [];
    let input_empty_disallowed = ExistingTerminalsInput {
        id_salt: Id::new("empty-no-new"),
        host_label: "",
        live_epoch: Some("ep-empty"),
        state: ExistingTerminalsState::Loaded {
            list_epoch: "ep-empty",
            entries: &empty,
        },
        can_attach: true,
        can_open_new: false,
        french: false,
    };
    let f_empty_no = settle_modal(&ctx, &mut panel, &input_empty_disallowed, vp, next_t());
    assert!(has_painted_substring(
        &f_empty_no.output.shapes,
        "No existing terminals are running on remote host."
    ));
    assert!(find_accesskit_button_center(&f_empty_no.output, "New terminal").is_none());
    let (e_empty, _) = press_key(
        &ctx,
        &mut panel,
        &input_empty_disallowed,
        vp,
        next_t(),
        Key::Enter,
    );
    assert_eq!(e_empty, None);

    let input_empty_allowed_fr = ExistingTerminalsInput {
        can_open_new: true,
        french: true,
        ..input_empty_disallowed
    };
    let f_empty_fr = settle_modal(&ctx, &mut panel, &input_empty_allowed_fr, vp, next_t());
    assert!(has_painted_substring(
        &f_empty_fr.output.shapes,
        "Aucun terminal existant n'est actif sur l'hôte distant."
    ));
    let (e_empty_fr, _) = press_key(
        &ctx,
        &mut panel,
        &input_empty_allowed_fr,
        vp,
        next_t(),
        Key::Enter,
    );
    assert_eq!(
        e_empty_fr, None,
        "Enter on empty list must never implicitly spawn a terminal"
    );

    let f_empty_btn = settle_modal(&ctx, &mut panel, &input_empty_allowed_fr, vp, next_t());
    let open_new_pos = require_accesskit_button_center(&f_empty_btn.output, "Nouveau terminal");
    let (new_act, _) = click_modal(
        &ctx,
        &mut panel,
        &input_empty_allowed_fr,
        vp,
        next_t(),
        open_new_pos,
    );
    assert_eq!(new_act, Some(ExistingTerminalsAction::OpenNewTerminal));
}

#[test]
fn escape_popup_priority_held_autorepeat_backdrop_close_and_all_themes() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));

    let entries = [ExistingTerminalEntry {
        terminal_id: "term-esc-1",
        pid: 900,
        size: (24, 80),
        attached: 1,
        leased_on_host: false,
        retained_bytes: 512,
        exited: None,
        has_saved_lease: false,
        open_locally: false,
    }];

    let mut panel = HostExistingTerminalsPanel::new();
    let vp = Vec2::new(800.0, 500.0);
    let input = ExistingTerminalsInput {
        id_salt: Id::new("picker-esc"),
        host_label: "gpu-node",
        live_epoch: Some("ep-esc"),
        state: ExistingTerminalsState::Loaded {
            list_epoch: "ep-esc",
            entries: &entries,
        },
        can_attach: true,
        can_open_new: true,
        french: false,
    };

    // Settle the background pane first so `term_resp` is registered in `egui` hit-testing,
    // then open an `egui::Popup` context menu on it and open the modal while that popup stays open.
    let center = Pos2::new(400.0, 250.0);
    let _ = step_modal_frame(&ctx, &mut panel, None, vp, 0.90, vec![], true);
    let _ = step_modal_frame(&ctx, &mut panel, None, vp, 0.94, vec![], true);
    let _ = step_modal_frame(
        &ctx,
        &mut panel,
        None,
        vp,
        1.0,
        pointer_down(center, PointerButton::Secondary),
        true,
    );
    let popup_opened = step_modal_frame(
        &ctx,
        &mut panel,
        None,
        vp,
        1.02,
        pointer_up(center, PointerButton::Secondary),
        true,
    );
    assert!(popup_opened.popup_open, "context menu popup should be open");

    let modal_with_popup = step_modal_frame(&ctx, &mut panel, Some(&input), vp, 1.05, vec![], true);
    assert!(modal_with_popup.popup_open);
    assert_eq!(modal_with_popup.action, None);

    // 1st Escape press closes the popup FIRST without closing the modal, and latches `escape_held`.
    let esc_1_down = step_modal_frame(
        &ctx,
        &mut panel,
        Some(&input),
        vp,
        1.10,
        vec![key_ev(Key::Escape, true, false, Modifiers::NONE)],
        true,
    );
    assert_eq!(
        esc_1_down.action, None,
        "1st Escape must dismiss open Popup without closing modal"
    );
    assert!(panel.is_escape_held());
    assert_eq!(esc_1_down.forwarded_escape_count, 0);

    let esc_1_repeat = step_modal_frame(
        &ctx,
        &mut panel,
        Some(&input),
        vp,
        1.14,
        vec![key_ev(Key::Escape, true, true, Modifiers::NONE)],
        true,
    );
    assert!(!esc_1_repeat.popup_open, "popup must be closed");
    assert_eq!(
        esc_1_repeat.action, None,
        "held Escape autorepeat after popup close must not close modal"
    );
    assert!(panel.is_escape_held());
    assert_eq!(esc_1_repeat.forwarded_escape_count, 0);

    // Physical release clears `escape_held`.
    let esc_1_up = step_modal_frame(
        &ctx,
        &mut panel,
        Some(&input),
        vp,
        1.18,
        vec![key_ev(Key::Escape, false, false, Modifiers::NONE)],
        true,
    );
    assert!(!panel.is_escape_held());
    assert_eq!(esc_1_up.action, None);

    // 2nd fresh Escape press closes the modal (`ExistingTerminalsAction::Close`) and keeps
    // `escape_held` latched across subsequent autorepeat frames after the modal closes so zero
    // `\x1b` leaks into the underlying `Live` terminal!
    let esc_2_down = step_modal_frame(
        &ctx,
        &mut panel,
        Some(&input),
        vp,
        1.22,
        vec![key_ev(Key::Escape, true, false, Modifiers::NONE)],
        false,
    );
    assert_eq!(esc_2_down.action, Some(ExistingTerminalsAction::Close));
    assert!(panel.is_escape_held());
    assert_eq!(esc_2_down.forwarded_escape_count, 0);

    let esc_2_repeat_after_close = step_modal_frame(
        &ctx,
        &mut panel,
        None,
        vp,
        1.26,
        vec![key_ev(Key::Escape, true, true, Modifiers::NONE)],
        false,
    );
    assert!(panel.is_escape_held());
    assert_eq!(
        esc_2_repeat_after_close.forwarded_escape_count, 0,
        "held Escape autorepeat after modal close must never leak to Live terminal"
    );

    let esc_2_up = step_modal_frame(
        &ctx,
        &mut panel,
        None,
        vp,
        1.30,
        vec![key_ev(Key::Escape, false, false, Modifiers::NONE)],
        false,
    );
    assert!(!panel.is_escape_held());
    assert_eq!(esc_2_up.forwarded_escape_count, 0);

    // Backdrop click outside `last_modal_rect()` emits `Close`.
    let _ = settle_modal(&ctx, &mut panel, &input, vp, 1.40);
    let card_rect = panel.last_modal_rect().expect("modal rect");
    let outside_pos = Pos2::new(4.0, 4.0);
    assert!(!card_rect.contains(outside_pos));
    let (backdrop_act, _) = click_modal(&ctx, &mut panel, &input, vp, 1.45, outside_pos);
    assert_eq!(backdrop_act, Some(ExistingTerminalsAction::Close));

    // Render cleanly across all 7 dark themes + light mode in both EN and FR.
    let mut t = 2.0;
    for spec in &THEMES {
        theme::apply(&ctx, true, Some(*spec));
        for fr in [false, true] {
            let themed_input = ExistingTerminalsInput {
                french: fr,
                ..input
            };
            let frame = settle_modal(&ctx, &mut panel, &themed_input, vp, t);
            assert!(!frame.output.shapes.is_empty());
            assert_modal_within_viewport(&panel, vp);
            t += 0.1;
        }
    }
    theme::apply(&ctx, false, None);
    let light_frame = settle_modal(&ctx, &mut panel, &input, vp, t);
    assert!(!light_frame.output.shapes.is_empty());
    assert_modal_within_viewport(&panel, vp);
}
