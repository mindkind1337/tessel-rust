#[allow(dead_code)]
#[path = "../src/agent_env.rs"]
mod agent_env;
#[allow(dead_code)]
#[path = "../src/git/process.rs"]
mod git;
#[allow(dead_code)]
#[path = "../src/gpui_pixels.rs"]
mod gpui_pixels;
#[allow(dead_code)]
#[path = "../src/guest_file_editor.rs"]
mod guest_file_editor;
#[allow(dead_code)]
#[path = "../src/guest_file_listing.rs"]
mod guest_file_listing;
#[allow(dead_code)]
#[path = "../src/guest_git.rs"]
mod guest_git;
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
#[path = "../src/native_recycle.rs"]
mod native_recycle;
#[allow(dead_code)]
#[path = "../src/pane_grid.rs"]
mod pane_grid;
#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/process_job.rs"]
mod process_job;
#[allow(dead_code)]
#[path = "../src/project_panel.rs"]
mod project_panel;
#[path = "../src/raster_preview.rs"]
mod raster_preview;
#[path = "../src/saved_version_diff.rs"]
mod saved_version_diff;
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
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
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
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
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
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
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
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
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
        panel.poll_guest_requests();
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

fn guest_file_fixture() -> (tempfile::TempDir, Host, SharePanel) {
    let dir = tempfile::Builder::new()
        .prefix("tessel-files-fixture-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    // The parent checkout ignores Cargo's target directory. Give the fixture
    // its own Git boundary so guest reads exercise only its own ignore rules.
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet", "--template="])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success()
    );
    let projects: Vec<_> = (1..=2)
        .map(|id| {
            let path = dir.path().join(format!("p{id}"));
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("a.txt"), format!("A in {id}")).unwrap();
            std::fs::write(path.join("b.txt"), format!("B in {id}")).unwrap();
            std::fs::write(path.join("AGENTS.md"), "Instructions").unwrap();
            SharedProject {
                id,
                name: format!("Project {id}"),
                path,
            }
        })
        .collect();
    let host = Host::start_multi(
        projects,
        "Fixture".into(),
        InviteSpec::default(),
        ShareSettings::default(),
        vec![],
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1".parse().unwrap(),
    )
    .unwrap();
    host.publish_tasks(vec![]);
    let guest = GuestSession::connect(host.invitation.clone(), "Fixture".into(), "b8".repeat(32));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(LiveShareAsk::Join { key, .. }) = host.snapshot().asks.first() {
            host.answer_ask(share::AskAnswer::Join {
                key: key.clone(),
                allow: true,
                projects: vec![1, 2],
                rights: ShareRights::collaborate(),
                trust: false,
            });
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut panel = SharePanel::default();
    panel.set_guest_session(guest);
    panel.guest_sub_view = share_panel::GuestSubView::Files;
    loop {
        panel.poll_guest_requests();
        if panel.guest_task_snapshot().scope.is_some() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    (dir, host, panel)
}
fn await_guest_document(
    panel: &mut SharePanel,
    key: &guest_file_editor::Key,
) -> guest_file_editor::Document {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        panel.poll_guest_requests();
        if let Some(doc) = panel.guest_file_document().filter(|d| &d.key == key) {
            return doc;
        }
        assert!(
            Instant::now() < deadline,
            "file read failed: {:?}",
            panel.guest_file_notice()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn draw_guest_files(
    ctx: &egui::Context,
    panel: &mut SharePanel,
    events: Vec<Event>,
) -> egui::FullOutput {
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
}
#[test]
fn compatibility_guest_image_preview_downloads_png_and_preserves_dirty_text() {
    verify_guest_image("poster.png");
}

#[test]
fn compatibility_guest_image_preview_downloads_original_icon() {
    verify_guest_image("poster.ICO");
}

#[test]
fn compatibility_guest_image_preview_downloads_original_svg() {
    verify_guest_image("poster.SVG");
}

#[test]
fn compatibility_guest_image_preview_downloads_original_avif() {
    verify_guest_image("poster.AVIF");
}

fn verify_guest_image(filename: &str) {
    let (dir, _host, mut panel) = guest_file_fixture();
    let path = dir.path().join("p1").join(filename);
    if filename.ends_with(".AVIF") {
        std::fs::write(&path, include_bytes!("fixtures/alpha-bars.avif")).unwrap();
    } else if filename.ends_with(".SVG") {
        std::fs::write(&path, br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 48"><circle cx="32" cy="24" r="20" fill="red"/></svg>"#).unwrap();
    } else {
        image::RgbaImage::from_pixel(64, 48, image::Rgba([255u8, 50, 100, 255]))
            .save(&path)
            .unwrap();
    }
    let scope = panel.guest_task_snapshot().scope.unwrap();
    let key = guest_file_editor::Key {
        scope,
        path: filename.into(),
    };
    panel.open_guest_file(key.clone());
    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(20);
    let pos = loop {
        panel.poll_guest_requests();
        let out = draw_guest_files(&ctx, &mut panel, vec![]);
        let meshes = ctx.tessellate(out.shapes, out.pixels_per_point);
        if let Some(pos) = meshes.iter().find_map(|c| match &c.primitive {
            egui::epaint::Primitive::Mesh(m) if m.texture_id != egui::TextureId::default() => {
                Some(m.calc_bounds().intersect(c.clip_rect).center())
            }
            _ => None,
        }) {
            break pos;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    for pressed in [true, false] {
        draw_guest_files(
            &ctx,
            &mut panel,
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Secondary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    }
    assert!(egui::Popup::is_any_open(&ctx));
    let dest = dir.path().join(filename);
    panel.download_guest_file(&key, |name| {
        assert_eq!(name, filename);
        Some(dest.clone())
    });
    while panel.guest_requests_busy() {
        panel.poll_guest_requests();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(std::fs::read(&dest).unwrap(), std::fs::read(path).unwrap());
    let text = guest_file_editor::Key {
        scope,
        path: "a.txt".into(),
    };
    panel.open_guest_file(text.clone());
    await_guest_document(&mut panel, &text);
    panel.edit_guest_file(&text, "draft".into());
    panel.open_guest_file(key);
    assert_eq!(panel.guest_file_document().unwrap().text.as_str(), "draft");
}
#[test]
fn guest_download_context_and_binary_transfer_preserve_original_bytes() {
    let (dir, _host, mut panel) = guest_file_fixture();
    let bytes = [0, 0xff, 0x80, 0, 1, 3].repeat(512 * 1024);
    std::fs::write(dir.path().join("p1/a.txt"), &bytes).unwrap();
    let dest = dir.path().join("chosen.bin");
    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        panel.poll_guest_requests();
        let out = draw_guest_files(&ctx, &mut panel, vec![]);
        if find_text_center(&out.shapes, "· a.txt").is_some() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..3 {
        draw_guest_files(&ctx, &mut panel, vec![]);
    }
    let out = draw_guest_files(&ctx, &mut panel, vec![]);
    let pos = find_text_center(&out.shapes, "· a.txt").unwrap();
    draw_guest_files(&ctx, &mut panel, vec![Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        draw_guest_files(
            &ctx,
            &mut panel,
            vec![Event::PointerButton {
                pos,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            }],
        );
    }
    let mut out = draw_guest_files(&ctx, &mut panel, vec![]);
    for _ in 0..8 {
        if has_text_containing(&out.shapes, "Download") {
            break;
        }
        out = draw_guest_files(&ctx, &mut panel, vec![]);
    }
    assert!(
        has_text_containing(&out.shapes, "Download"),
        "{:?}",
        painted_texts(&out.shapes)
    );
    let key = guest_file_editor::Key {
        scope: panel.guest_task_snapshot().scope.unwrap(),
        path: "a.txt".into(),
    };
    for expected in [bytes.as_slice(), b"existing file".as_slice()] {
        if expected != bytes {
            std::fs::write(&dest, expected).unwrap();
        }
        panel.download_guest_file(&key, |name| {
            assert_eq!(name, "a.txt");
            Some(dest.clone())
        });
        while panel.guest_requests_busy() {
            panel.poll_guest_requests();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(std::fs::read(&dest).unwrap(), expected);
    }
    assert!(panel.guest_file_document().is_none());
}
#[test]
fn compatibility_guest_timeline_opens_history_for_the_selected_file() {
    let (dir, _host, mut panel) = guest_file_fixture();
    let project = dir.path().join("p1");
    for args in [
        vec!["init", "-q"],
        vec!["add", "a.txt"],
        vec!["commit", "-qm", "first guest timeline entry"],
    ] {
        let out = std::process::Command::new("git")
            .current_dir(&project)
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let key = guest_file_editor::Key {
        scope: panel.guest_task_snapshot().scope.unwrap(),
        path: "a.txt".into(),
    };
    assert!(panel.open_guest_file(key.clone()));
    await_guest_document(&mut panel, &key);
    let ctx = egui::Context::default();
    let _ = draw_guest_files(&ctx, &mut panel, vec![]);
    let deadline = Instant::now() + Duration::from_secs(8);
    let pos = loop {
        panel.poll_guest_requests();
        let out = draw_guest_files(&ctx, &mut panel, vec![]);
        if has_text_containing(&out.shapes, "· b.txt") {
            break find_text_center(&out.shapes, "Timeline").expect("Timeline control");
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    draw_guest_files(&ctx, &mut panel, click_events(pos));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        panel.poll_guest_requests();
        let out = draw_guest_files(&ctx, &mut panel, vec![]);
        if has_text_containing(&out.shapes, "first guest timeline entry") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{:?}",
            painted_texts(&out.shapes)
        );
        std::thread::sleep(Duration::from_millis(15));
    }
    let other = guest_file_editor::Key {
        path: "b.txt".into(),
        ..key
    };
    assert!(panel.open_guest_file(other.clone()));
    await_guest_document(&mut panel, &other);
    let out = draw_guest_files(&ctx, &mut panel, vec![]);
    assert!(!has_text_containing(
        &out.shapes,
        "first guest timeline entry"
    ));
}

#[test]
fn compatibility_guest_file_reads_and_saves_exact_project_and_path() {
    let (dir, _host, mut panel) = guest_file_fixture();
    let scope = panel.guest_task_snapshot().scope.unwrap();
    let a = guest_file_editor::Key {
        scope,
        path: "a.txt".into(),
    };
    let b = guest_file_editor::Key {
        scope,
        path: "b.txt".into(),
    };
    assert!(panel.open_guest_file(a.clone()));
    assert_eq!(await_guest_document(&mut panel, &a).text.as_str(), "A in 1");
    assert!(panel.open_guest_file(b.clone()));
    assert_eq!(await_guest_document(&mut panel, &b).text.as_str(), "B in 1");
    assert!(panel.edit_guest_file(&b, "Edited B only".into()));
    let ctx = egui::Context::default();
    let _ = draw_guest_files(&ctx, &mut panel, vec![]);
    let output = draw_guest_files(&ctx, &mut panel, vec![]);
    let save = find_text_center(&output.shapes, "Save").unwrap_or_else(|| {
        panic!(
            "missing Save; document={:?}; notice={:?}; text={:?}",
            panel.guest_file_document(),
            panel.guest_file_notice(),
            painted_texts(&output.shapes)
                .into_iter()
                .map(|(text, _)| text)
                .collect::<Vec<_>>()
        )
    });
    draw_guest_files(&ctx, &mut panel, click_events(save));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        panel.poll_guest_requests();
        if panel.guest_file_notice() == Some(guest_file_editor::Notice::Saved) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "save failed: {:?}",
            panel.guest_file_notice()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("p1/b.txt")).unwrap(),
        "Edited B only"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("p1/a.txt")).unwrap(),
        "A in 1"
    );
    let protected = guest_file_editor::Key {
        scope,
        path: "AGENTS.md".into(),
    };
    panel.open_guest_file(protected.clone());
    await_guest_document(&mut panel, &protected);
    assert!(!panel.edit_guest_file(&protected, "forbidden".into()));
    panel.apply_gpui_share_action(
        share_panel::ShareGpuiAction::GuestProject {
            session: scope.session,
            project: 2,
        },
        &[],
    );
    let other = guest_file_editor::Key {
        scope: guest_task_board::Scope {
            session: scope.session,
            project: 2,
        },
        path: "a.txt".into(),
    };
    panel.open_guest_file(other.clone());
    assert_eq!(
        await_guest_document(&mut panel, &other).text.as_str(),
        "A in 2"
    );
}
#[test]
fn compatibility_guest_file_disconnect_keeps_draft_and_requires_confirmed_discard() {
    let (_dir, _host, mut panel) = guest_file_fixture();
    let scope = panel.guest_task_snapshot().scope.unwrap();
    let key = guest_file_editor::Key {
        scope,
        path: "a.txt".into(),
    };
    panel.open_guest_file(key.clone());
    await_guest_document(&mut panel, &key);
    panel.edit_guest_file(&key, "Keep this draft".into());
    panel.apply_gpui_share_action(share_panel::ShareGpuiAction::DisconnectGuest, &[]);
    panel.poll_guest_requests();
    let ctx = egui::Context::default();
    let _ = draw_guest_files(&ctx, &mut panel, vec![]);
    let output = draw_guest_files(&ctx, &mut panel, vec![]);
    assert!(has_text_containing(&output.shapes, "Keep this draft"));
    let close = find_text_center(&output.shapes, "Close file").unwrap();
    draw_guest_files(&ctx, &mut panel, click_events(close));
    let output = draw_guest_files(&ctx, &mut panel, vec![]);
    assert!(has_text_containing(
        &output.shapes,
        "Discard this unsaved draft?"
    ));
    let keep = find_text_center(&output.shapes, "Keep draft").unwrap();
    draw_guest_files(&ctx, &mut panel, click_events(keep));
    assert_eq!(
        panel.guest_file_document().unwrap().text.as_str(),
        "Keep this draft"
    );
    let output = draw_guest_files(&ctx, &mut panel, vec![]);
    let close = find_text_center(&output.shapes, "Close file").unwrap();
    draw_guest_files(&ctx, &mut panel, click_events(close));
    let output = draw_guest_files(&ctx, &mut panel, vec![]);
    let discard = find_text_center(&output.shapes, "Discard draft").unwrap();
    draw_guest_files(&ctx, &mut panel, click_events(discard));
    assert!(panel.guest_file_document().is_none());
}

#[test]
fn compatibility_guest_file_paste_is_never_silently_truncated() {
    let (dir, _host, mut panel) = guest_file_fixture();
    let key = guest_file_editor::Key {
        scope: panel.guest_task_snapshot().scope.unwrap(),
        path: "a.txt".into(),
    };
    panel.open_guest_file(key.clone());
    await_guest_document(&mut panel, &key);
    let ctx = egui::Context::default();
    let _ = draw_guest_files(&ctx, &mut panel, vec![]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while panel.guest_requests_busy() {
        panel.poll_guest_requests();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..3 {
        draw_guest_files(&ctx, &mut panel, vec![]);
    }
    let out = draw_guest_files(&ctx, &mut panel, vec![]);
    let text = find_text_center(&out.shapes, "A in 1").unwrap();
    draw_guest_files(&ctx, &mut panel, click_events(text));
    assert!(
        ctx.memory(|m| m.focused()).is_some(),
        "text click at {text:?} did not focus editor"
    );
    let pasted = format!("{}END_OF_PASTE", "x".repeat(200 * 1024));
    draw_guest_files(&ctx, &mut panel, vec![Event::Paste(pasted.clone())]);
    let doc = panel.guest_file_document().unwrap();
    assert_eq!(doc.text.len(), pasted.len() + "A in 1".len());
    assert!(doc.text.contains(&pasted));
    assert_eq!(
        panel.guest_file_notice(),
        Some(guest_file_editor::Notice::TooLarge)
    );
    panel.save_guest_file(&key);
    assert_eq!(
        panel.guest_file_notice(),
        Some(guest_file_editor::Notice::TooLarge)
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("p1/a.txt")).unwrap(),
        "A in 1"
    );
}

#[test]
fn compatibility_guest_git_draft_cannot_cross_projects_and_survives_leave() {
    let (_dir, _host, mut panel) = guest_file_fixture();
    panel.guest_sub_view = share_panel::GuestSubView::Git;
    let scope = panel.guest_git_snapshot().scope.unwrap();
    assert!(panel.edit_guest_commit(scope, "Keep my commit message".into()));
    panel.apply_gpui_share_action(
        share_panel::ShareGpuiAction::GuestProject {
            session: scope.session,
            project: 2,
        },
        &[],
    );
    let other = panel.guest_git_snapshot().scope.unwrap();
    assert_ne!(scope, other);
    assert!(!panel.request_guest_git(other, guest_git::Action::Commit));
    assert_eq!(panel.guest_git_snapshot().draft, "Keep my commit message");
    panel.apply_gpui_share_action(share_panel::ShareGpuiAction::DisconnectGuest, &[]);
    panel.poll_guest_requests();
    let ctx = egui::Context::default();
    for _ in 0..3 {
        draw_guest_files(&ctx, &mut panel, vec![]);
    }
    let out = draw_guest_files(&ctx, &mut panel, vec![]);
    assert!(has_text_containing(&out.shapes, "Keep my commit message"));
    let discard = find_text_center(&out.shapes, "Discard message").unwrap();
    draw_guest_files(&ctx, &mut panel, click_events(discard));
    let out = draw_guest_files(&ctx, &mut panel, vec![]);
    assert!(!panel.guest_git_snapshot().draft.is_empty());
    let confirm = find_text_center(&out.shapes, "Confirm discard").unwrap();
    draw_guest_files(&ctx, &mut panel, click_events(confirm));
    assert!(panel.guest_git_snapshot().draft.is_empty());
}
