#[allow(dead_code)]
#[path = "../src/guest_task_board.rs"]
mod guest_task_board;
#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/layout_broadcast_ui.rs"]
mod layout_broadcast_ui;
#[allow(dead_code)]
#[path = "../src/pane_grid.rs"]
mod pane_grid;
#[allow(dead_code)]
#[path = "../src/project_panel.rs"]
mod project_panel;
#[allow(dead_code)]
#[path = "../src/share.rs"]
mod share;
#[allow(dead_code)]
#[path = "../src/share_panel.rs"]
mod share_panel;
#[allow(dead_code)]
#[path = "../src/shortcuts.rs"]
mod shortcuts;
#[allow(dead_code)]
#[path = "../src/sidebar_model.rs"]
mod sidebar_model;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;
#[path = "../src/toolbar_fit.rs"]
mod toolbar_fit;

use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, Rect, Shape, Vec2};
use project_panel::{ProjectEntry, ProjectPanel, ProjectPanelInput};
use share::{
    GuestConnectionState, GuestPhase, GuestSession, Host, InviteSpec, LiveShareAsk,
    PaneGuestPresence, ShareRights, ShareSettings, SharedProject,
};
use share_panel::{HostSubTab, SharePanel};
use std::time::{Duration, Instant};

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

fn has_text_containing(shapes: &[egui::epaint::ClippedShape], needle: &str) -> bool {
    painted_texts(shapes)
        .iter()
        .any(|(text, _)| text.contains(needle))
}

fn find_text_center(shapes: &[egui::epaint::ClippedShape], exact: &str) -> Option<Pos2> {
    painted_texts(shapes)
        .into_iter()
        .find(|(text, _)| text == exact)
        .map(|(_, rect)| rect.center())
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

#[test]
fn live_share_dialog_subtabs_rights_grid_and_stop_confirmation_en_and_fr() {
    let dir = tempfile::Builder::new()
        .prefix("tessel-ui-ls-dialog-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    std::fs::write(dir.path().join("README.md"), "demo").unwrap();

    let proj = SharedProject {
        id: 7,
        name: "Alpha".into(),
        path: dir.path().to_path_buf(),
    };
    let projects = vec![proj.clone()];

    let ctx = egui::Context::default();
    let viewport = Vec2::new(1024.0_f32, 820.0_f32);
    let mut panel = SharePanel::default();
    panel.pre_share_invite.open = true;
    panel.pre_share_invite.rights = ShareRights::full_control();

    // 1. Pre-share dialog in English shows 8-tile RightsGrid, presets, danger Type tag, and warning
    let run_dialog = |panel: &mut SharePanel, french: bool, events: Vec<Event>| {
        let mut closed = false;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                closed = panel.show_dialog_modal(ctx, &projects, french);
            },
        );
        (closed, out)
    };

    let _ = run_dialog(&mut panel, false, vec![]);
    let (_, out_en) = run_dialog(&mut panel, false, vec![]);
    assert!(has_text_containing(&out_en.shapes, "Live Share"));
    assert!(has_text_containing(&out_en.shapes, "First invitation"));
    assert!(has_text_containing(&out_en.shapes, "Full control"));
    assert!(has_text_containing(&out_en.shapes, "Runs commands as you"));
    assert!(has_text_containing(
        &out_en.shapes,
        "Warning: Type permission"
    ));

    // 2. Start active Host and verify status banner + 4 host sub-tabs + inline Stop confirmation in French
    let host = Host::start_multi(
        vec![proj],
        "Jean".into(),
        InviteSpec::default(),
        ShareSettings::default(),
        Vec::new(),
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1".parse().unwrap(),
    )
    .unwrap();
    panel.set_host(host);

    let _ = run_dialog(&mut panel, true, vec![]);
    let (_, out_fr) = run_dialog(&mut panel, true, vec![]);
    assert!(has_text_containing(
        &out_fr.shapes,
        "Partage de 1 projet(s)"
    ));
    assert!(has_text_containing(&out_fr.shapes, "Aperçu"));
    assert!(has_text_containing(&out_fr.shapes, "Personnes (0)"));
    assert!(has_text_containing(&out_fr.shapes, "Invitations (1)"));
    assert!(has_text_containing(&out_fr.shapes, "Réglages"));

    // Click "Arrêter le partage" -> inline confirmation "Arrêter Live Share ?" appears without stopping yet!
    let stop_pos = find_text_center(&out_fr.shapes, "Arrêter le partage").unwrap();
    let _ = run_dialog(&mut panel, true, click_events(stop_pos));
    let (_, out_confirm) = run_dialog(&mut panel, true, vec![]);
    assert!(
        panel.hosting(),
        "inline confirmation must not stop immediately"
    );
    assert!(has_text_containing(
        &out_confirm.shapes,
        "Arrêter Live Share ?"
    ));
    assert!(has_text_containing(
        &out_confirm.shapes,
        "Continuer le partage"
    ));

    // Switch through People, Invitations, Settings sub-tabs
    panel.confirm_stop_sharing = false;
    panel.host_sub_tab = HostSubTab::Invitations;
    let (_, out_inv) = run_dialog(&mut panel, false, vec![]);
    assert!(has_text_containing(&out_inv.shapes, "New invitation"));
    assert!(has_text_containing(&out_inv.shapes, "1/20"));

    panel.host_sub_tab = HostSubTab::Settings;
    let (_, out_set) = run_dialog(&mut panel, false, vec![]);
    assert!(has_text_containing(
        &out_set.shapes,
        "Hide ignored and secret files from guests"
    ));
    assert!(has_text_containing(&out_set.shapes, "Always"));
}

#[test]
fn live_share_ask_enforces_900ms_arm_delay_blocks_escape_and_gates_trust_on_sas() {
    let dir = tempfile::Builder::new()
        .prefix("tessel-ui-ls-ask-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

    let proj = SharedProject {
        id: 1,
        name: "Core".into(),
        path: dir.path().to_path_buf(),
    };
    let projects = vec![proj.clone()];
    let host = Host::start_multi(
        vec![proj],
        "HostOwner".into(),
        InviteSpec::default(),
        ShareSettings::default(),
        Vec::new(),
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1".parse().unwrap(),
    )
    .unwrap();
    let invite = host.invitation.clone();

    let mut panel = SharePanel::default();
    panel.set_host(host);

    let mut guest = GuestSession::connect(invite, "Marie".into(), "b2".repeat(32));
    let start = Instant::now();
    let guest_sas = loop {
        if let GuestPhase::WaitingForHost { sas_code, .. } = guest.snapshot().phase {
            break sas_code;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(20));
    };
    while !panel.has_pending_ask() {
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(20));
    }

    let ctx = egui::Context::default();
    let viewport = Vec2::new(1024.0_f32, 768.0_f32);
    let run_ask = |panel: &mut SharePanel, time_s: f64, events: Vec<Event>| {
        let mut dialog_closed = false;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                focused: true,
                time: Some(time_s),
                events,
                ..Default::default()
            },
            |ctx| {
                dialog_closed = panel.show_dialog_modal(ctx, &projects, false);
                let _ = panel.show_ask_modal(ctx, &projects, false);
            },
        );
        (dialog_closed, out)
    };

    // Frame at t = 0.0s (sizing) + t = 0.1s: SAS matches, Escape is blocked, Allow is NOT armed yet!
    let _ = run_ask(&mut panel, 0.0, vec![]);
    let (closed_on_esc, out_early) = run_ask(
        &mut panel,
        0.1,
        vec![Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(
        !closed_on_esc,
        "Escape must be blocked while LiveShareAsk is on screen"
    );
    assert!(has_text_containing(
        &out_early.shapes,
        "Marie wants to join"
    ));
    assert!(has_text_containing(&out_early.shapes, &guest_sas));

    // Clicking "Allow" at t = 0.4s (< 900ms) must NOT admit Marie!
    let allow_pos = find_text_center(&out_early.shapes, "Allow").unwrap();
    let (_, out_after_allow) = run_ask(&mut panel, 0.4, click_events(allow_pos));
    assert!(
        panel.has_pending_ask(),
        "Allow button must stay disabled during the 900ms arming delay"
    );

    // Clicking "Trust next time" before ticking "I checked it with them" nudges the user!
    let trust_pos = find_text_center(
        &out_after_allow.shapes,
        "Trust next time (let them in without knocking)",
    )
    .unwrap();
    let _ = run_ask(&mut panel, 0.5, click_events(trust_pos));
    let (_, out_nudge) = run_ask(&mut panel, 0.55, vec![]);
    assert!(has_text_containing(
        &out_nudge.shapes,
        "Tick “I checked it with them” above first."
    ));

    // Tick "I checked it with them: it is the same", then at t = 1.0s (>= 900ms) click "Allow"!
    let sas_chk_pos =
        find_text_center(&out_nudge.shapes, "I checked it with them: it is the same").unwrap();
    let _ = run_ask(&mut panel, 0.95, click_events(sas_chk_pos));
    let (_, out_armed) = run_ask(&mut panel, 1.0, vec![]);
    let allow_armed_pos = find_text_center(&out_armed.shapes, "Allow").unwrap();
    let _ = run_ask(&mut panel, 1.05, click_events(allow_armed_pos));
    assert!(
        !panel.has_pending_ask(),
        "Allow at >=900ms must admit guest"
    );

    let wait_join = Instant::now();
    while !matches!(guest.snapshot().phase, GuestPhase::Joined { .. }) {
        assert!(wait_join.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(20));
    }
    guest.leave();
}

#[test]
fn live_share_pill_popover_presence_and_sidebar_badge_render_cleanly() {
    let dir = tempfile::Builder::new()
        .prefix("tessel-ui-ls-pill-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    let proj = SharedProject {
        id: 10,
        name: "WebApp".into(),
        path: dir.path().to_path_buf(),
    };
    let projects = vec![proj.clone()];
    let host = Host::start_multi(
        vec![proj],
        "Host".into(),
        InviteSpec::default(),
        ShareSettings::default(),
        Vec::new(),
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1".parse().unwrap(),
    )
    .unwrap();
    let mut panel = SharePanel::default();
    panel.set_host(host);

    let ctx = egui::Context::default();
    let viewport = Vec2::new(900.0_f32, 600.0_f32);
    let presence = vec![PaneGuestPresence {
        guest_id: "g1".into(),
        name: "Claire Martin".into(),
        initials: "CM".into(),
        watching: true,
        typing: true,
    }];
    let mut sidebar = ProjectPanel::new();
    sidebar.set_live_share_projects([(10u64, vec!["CM".to_owned()])].into_iter().collect());
    let sidebar_entries = vec![ProjectEntry {
        id: 10,
        name: "WebApp".into(),
        path: dir.path().display().to_string(),
        sessions: Vec::new(),
    }];

    let run_frame = |panel: &mut SharePanel, sidebar: &mut ProjectPanel, events: Vec<Event>| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::TopBottomPanel::top("tb").show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let _ = panel.show_toolbar_control(ui, &projects, Some(10), false);
                        SharePanel::paint_pane_presence(ui, &presence, false);
                    });
                });
                egui::SidePanel::left("sb")
                    .default_width(260.0_f32)
                    .show(ctx, |ui| {
                        let _ = sidebar.show(
                            ui,
                            ProjectPanelInput {
                                projects: &sidebar_entries,
                                global_sessions: &[],
                                active_project: Some(10),
                                active_session: None,
                                french: false,
                            },
                        );
                    });
            },
        )
    };

    let _ = run_frame(&mut panel, &mut sidebar, vec![]);
    let out = run_frame(&mut panel, &mut sidebar, vec![]);
    // Compact 20 px pill: the number of people only; the words are in the tooltip.
    assert!(find_text_center(&out.shapes, "0").is_some());
    assert!(!has_text_containing(&out.shapes, "Live · 0"));
    assert!(has_text_containing(&out.shapes, "⌨ CM"));
    assert!(has_text_containing(&out.shapes, "Live · CM"));

    // Click toolbar pill -> opens 360px quick-manage popover!
    let pill_pos = find_text_center(&out.shapes, "0").unwrap();
    let _ = run_frame(&mut panel, &mut sidebar, click_events(pill_pos));
    let out_pop = run_frame(&mut panel, &mut sidebar, vec![]);
    assert!(panel.is_pill_popover_open());
    assert!(has_text_containing(&out_pop.shapes, "Copy invite code"));
    assert!(has_text_containing(&out_pop.shapes, "Stop Live Share"));

    let _ = GuestConnectionState::Joined;
    let _ = LiveShareAsk::Close {
        key: "k".into(),
        guest_id: "g".into(),
        name: "n".into(),
        pane_id: 1,
        pane_name: "p".into(),
        detail: "d".into(),
    };
}

#[test]
fn compatibility_guest_board_add_is_pending_without_waiting_for_host_commit() {
    let dir = tempfile::Builder::new()
        .prefix("tessel-guest-fixture-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    let project = SharedProject {
        id: 1,
        name: "Fixture board".into(),
        path: dir.path().into(),
    };
    let host = Host::start_multi(
        vec![project],
        "Fixture".into(),
        InviteSpec::default(),
        ShareSettings::default(),
        vec![],
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1".parse().unwrap(),
    )
    .unwrap();
    host.publish_tasks(vec![share::SharedTask {
        project_id: 1,
        id: "1".into(),
        title: "Visible fixture card".into(),
        column: "review".into(),
        guest_by: Some("Guest fixture".into()),
    }]);
    let guest = GuestSession::connect(
        host.invitation.clone(),
        "Fixture guest".into(),
        "c3".repeat(32),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(LiveShareAsk::Join { key, .. }) = host.snapshot().asks.first() {
            host.answer_ask(share::AskAnswer::Join {
                key: key.clone(),
                allow: true,
                projects: vec![1],
                rights: ShareRights::collaborate(),
                trust: false,
            });
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut panel = SharePanel::default();
    panel.set_guest_session(guest);
    panel.guest_sub_view = share_panel::GuestSubView::Tasks;
    let scope = loop {
        panel.poll_guest_tasks();
        let board = panel.guest_task_snapshot();
        if let Some(scope) = board.scope
            && !board.pending
            && board.cards.len() == 1
        {
            break scope;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    panel.edit_guest_task_draft(scope, "Added from Compatibility".into());
    let ctx = egui::Context::default();
    let draw = |panel: &mut SharePanel, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200., 1000.))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                panel.show_dialog_modal(ctx, &[], false);
            },
        )
    };
    let _ = draw(&mut panel, vec![]);
    let output = draw(&mut panel, vec![]);
    assert!(has_text_containing(&output.shapes, "Visible fixture card"));
    assert!(has_text_containing(&output.shapes, "Guest fixture"));
    let add = find_text_center(&output.shapes, "Add").unwrap();
    let _ = draw(&mut panel, click_events(add));
    assert!(panel.guest_task_snapshot().pending);
    assert_eq!(
        panel.guest_task_snapshot().draft,
        "Added from Compatibility"
    );
    // The host has not applied a mutation; the frame nevertheless returned with a pending state.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(share::HostCommand::TaskMutation(command)) = host.drain_commands().pop() {
            command.reject();
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
}
