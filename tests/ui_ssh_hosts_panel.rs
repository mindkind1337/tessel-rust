#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/settings_surface.rs"]
mod settings_surface;
#[path = "../src/ssh_hosts_panel.rs"]
mod ssh_hosts_panel;
#[allow(dead_code)]
#[path = "../src/theme.rs"]
mod theme;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

use eframe::egui::{self, Color32, Key, Modifiers, Pos2, Rect, Shape, Vec2};
use ssh_hosts_panel::{
    MAX_REMOTE_FOLDER_BYTES, MAX_SSH_ALIAS_BYTES, MAX_SSH_HOSTS, SshHostRow, SshHostSpec,
    SshHostStatus, SshHostValidationError, SshHostsAction, SshHostsCapabilities, SshHostsInput,
    SshHostsNotice, SshHostsNoticeTone, SshHostsPanel, clean_remote_folder, clean_ssh_alias,
    validate_remote_folder, validate_ssh_alias, validate_ssh_host_draft,
};
use theme_palettes::THEMES;

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

fn has_painted_substring(shapes: &[egui::epaint::ClippedShape], needle: &str) -> bool {
    painted_texts(shapes)
        .iter()
        .any(|(text, _, _, _)| text.contains(needle))
}

fn find_painted_text_rect(shapes: &[egui::epaint::ClippedShape], exact: &str) -> Option<Rect> {
    painted_texts(shapes)
        .into_iter()
        .find(|(text, _, _, _)| text == exact)
        .map(|(_, rect, _, _)| rect)
}

fn count_status_dots(shapes: &[egui::epaint::ClippedShape]) -> usize {
    fn walk(shape: &Shape) -> usize {
        match shape {
            Shape::Circle(c) => usize::from(c.radius >= 3.5 && c.radius <= 4.5),
            Shape::Vec(list) => list.iter().map(walk).sum(),
            _ => 0,
        }
    }
    shapes.iter().map(|cs| walk(&cs.shape)).sum()
}

fn has_status_dot_with_fill(shapes: &[egui::epaint::ClippedShape], target: Color32) -> bool {
    fn walk(shape: &Shape, target: Color32) -> bool {
        match shape {
            Shape::Circle(c) => c.fill == target && c.radius >= 3.5 && c.radius <= 4.5,
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

fn key_press_release(key: Key, modifiers: Modifiers) -> Vec<egui::Event> {
    vec![
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        },
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        },
    ]
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

fn run_panel_frame(
    ctx: &egui::Context,
    size: Vec2,
    events: Vec<egui::Event>,
    panel: &mut SshHostsPanel,
    input: SshHostsInput<'_>,
) -> (Option<SshHostsAction>, Vec<egui::epaint::ClippedShape>) {
    let mut action = None;
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                action = panel.show(ui, input.clone());
            });
        },
    );
    (action, out.shapes)
}

#[test]
fn pure_validation_matches_workspace_launch_and_sftp_invariants() {
    // Invisible copy-paste Unicode (`Cf` + outer NBSP) is stripped cleanly.
    assert_eq!(
        clean_ssh_alias(" \u{FEFF}\u{200B}deploy@prod-01.example.com\u{200D}\u{00A0} "),
        "deploy@prod-01.example.com"
    );
    assert_eq!(
        clean_remote_folder(" \u{200B}/srv/my project/..notes\u{FEFF} "),
        "/srv/my project/..notes"
    );

    // Empty & option-injection aliases are rejected.
    assert_eq!(
        validate_ssh_alias(&clean_ssh_alias(" \u{200B}\u{FEFF}\u{00A0} ")),
        Err(SshHostValidationError::AliasEmpty)
    );
    for dash_opt in ["-oProxyCommand=evil", "-J", "-v", "--"] {
        assert_eq!(
            validate_ssh_alias(dash_opt),
            Err(SshHostValidationError::AliasStartsWithDash),
            "must reject {dash_opt}"
        );
    }

    // Internal spaces (including NBSP) and shell/URI metacharacters are rejected, never joined.
    assert_eq!(
        validate_ssh_alias(&clean_ssh_alias("my server")),
        Err(SshHostValidationError::AliasBadChar(' '))
    );
    assert_eq!(
        validate_ssh_alias(&clean_ssh_alias("user@host\u{00A0}-oProxyCommand=evil")),
        Err(SshHostValidationError::AliasBadChar('\u{00A0}'))
    );
    for bad in ["host;id", "host/path", "ssh://host", "host$1", "host`id`"] {
        assert!(
            matches!(
                validate_ssh_alias(bad),
                Err(SshHostValidationError::AliasBadChar(_))
            ),
            "must reject {bad}"
        );
    }

    // 253-byte boundary (`workspace::validate_host`).
    assert!(validate_ssh_alias(&"a".repeat(MAX_SSH_ALIAS_BYTES)).is_ok());
    assert_eq!(
        validate_ssh_alias(&"a".repeat(MAX_SSH_ALIAS_BYTES + 1)),
        Err(SshHostValidationError::AliasTooLong)
    );
    for ok_alias in ["prod-box", "deploy@203.0.113.5", "::1", "host_1.internal"] {
        assert!(
            validate_ssh_alias(ok_alias).is_ok(),
            "must accept {ok_alias}"
        );
    }

    // Folder rules (`remote::clean` + `launch::command` + `host_actions::validate_folder`).
    assert!(validate_remote_folder("").is_ok());
    assert!(validate_remote_folder("/srv/app").is_ok());
    assert!(validate_remote_folder("~/workspace/my project").is_ok());
    // Valid folder names beginning with dots (`..notes`, `...config`) are allowed!
    assert!(validate_remote_folder("/srv/..notes").is_ok());
    assert!(validate_remote_folder("/srv/...config").is_ok());

    for traversal in [
        "..",
        "../secret",
        "/srv/../etc/passwd",
        "C:\\work\\..\\secret",
    ] {
        assert_eq!(
            validate_remote_folder(traversal),
            Err(SshHostValidationError::FolderParentTraversal),
            "must reject traversal {traversal}"
        );
    }
    for ctrl in ["/srv/a\nb", "/srv/a\rb", "/srv/a\0b"] {
        assert_eq!(
            validate_remote_folder(ctrl),
            Err(SshHostValidationError::FolderControlChar)
        );
    }
    assert!(validate_remote_folder(&"x".repeat(MAX_REMOTE_FOLDER_BYTES)).is_ok());
    assert_eq!(
        validate_remote_folder(&"x".repeat(MAX_REMOTE_FOLDER_BYTES + 1)),
        Err(SshHostValidationError::FolderTooLong)
    );

    // Duplicate & 1000-host capacity rules.
    let existing = vec![
        SshHostSpec::new("prod", "/srv/a"),
        SshHostSpec::new("staging", ""),
    ];
    assert_eq!(
        validate_ssh_host_draft("prod", "/srv/a", &existing, None),
        Err(SshHostValidationError::DuplicateTarget)
    );
    // Same alias with different remote folder is allowed.
    assert_eq!(
        validate_ssh_host_draft("prod", "/srv/b", &existing, None),
        Ok(SshHostSpec::new("prod", "/srv/b"))
    );
    // Editing index 0 and saving unchanged `(alias, folder)` is allowed.
    assert_eq!(
        validate_ssh_host_draft("prod", "/srv/a", &existing, Some(0)),
        Ok(SshHostSpec::new("prod", "/srv/a"))
    );
    // Editing index 1 into index 0's `(alias, folder)` is rejected as duplicate.
    assert_eq!(
        validate_ssh_host_draft("prod", "/srv/a", &existing, Some(1)),
        Err(SshHostValidationError::DuplicateTarget)
    );

    let full: Vec<SshHostSpec> = (0..MAX_SSH_HOSTS)
        .map(|i| SshHostSpec::new(format!("host-{i}"), ""))
        .collect();
    assert_eq!(
        validate_ssh_host_draft("extra-host", "", &full, None),
        Err(SshHostValidationError::TooManyTargets)
    );
    assert_eq!(
        validate_ssh_host_draft("host-0-renamed", "/srv", &full, Some(0)),
        Ok(SshHostSpec::new("host-0-renamed", "/srv"))
    );
}

#[test]
fn plain_render_across_all_seven_themes_and_legacy_light_is_idempotent_in_en_and_fr() {
    let mut theme_cases: Vec<(bool, Option<theme_palettes::ThemeId>)> =
        THEMES.iter().map(|&id| (true, Some(id))).collect();
    theme_cases.push((false, None)); // Legacy light

    let rows = [
        SshHostRow {
            host: SshHostSpec::new("deploy@prod.example.com", "/srv/afterlife"),
            status: None,
            error: None,
            busy: false,
        },
        SshHostRow {
            host: SshHostSpec::new("dev-box", ""),
            status: None,
            error: Some("Host key verification failed"),
            busy: false,
        },
    ];

    for (dark, theme_id) in theme_cases {
        for french in [false, true] {
            let ctx = egui::Context::default();
            theme::apply(&ctx, dark, theme_id);
            let mut panel = SshHostsPanel::new();

            // 1. Loading state never falsely says "No SSH targets configured."
            let (act_load, shapes_load) = run_panel_frame(
                &ctx,
                Vec2::new(820.0, 620.0),
                vec![],
                &mut panel,
                SshHostsInput {
                    hosts: &[],
                    loaded: false,
                    loading: true,
                    load_error: None,
                    notice: None,
                    french,
                    capabilities: SshHostsCapabilities::native_openssh(),
                },
            );
            assert_eq!(act_load, None);
            assert!(has_painted_text(
                &shapes_load,
                if french {
                    "Chargement…"
                } else {
                    "Loading…"
                }
            ));
            assert!(!has_painted_text(
                &shapes_load,
                if french {
                    "Aucune cible SSH configurée."
                } else {
                    "No SSH targets configured."
                }
            ));

            // 2. Empty state once loaded.
            let (act_empty, shapes_empty) = run_panel_frame(
                &ctx,
                Vec2::new(820.0, 620.0),
                vec![],
                &mut panel,
                SshHostsInput {
                    hosts: &[],
                    loaded: true,
                    loading: false,
                    load_error: None,
                    notice: None,
                    french,
                    capabilities: SshHostsCapabilities::native_openssh(),
                },
            );
            assert_eq!(act_empty, None);
            assert!(has_painted_text(
                &shapes_empty,
                if french {
                    "Aucune cible SSH configurée."
                } else {
                    "No SSH targets configured."
                }
            ));
            assert!(count_vector_lines_and_paths(&shapes_empty) >= 6);

            // 3. Populated state with notice & error text, plain render emits None.
            let (act_pop, shapes_pop) = run_panel_frame(
                &ctx,
                Vec2::new(820.0, 620.0),
                vec![],
                &mut panel,
                SshHostsInput {
                    hosts: &rows,
                    loaded: true,
                    loading: false,
                    load_error: None,
                    notice: Some(SshHostsNotice {
                        text: if french {
                            "Cible enregistrée"
                        } else {
                            "Target saved"
                        },
                        tone: SshHostsNoticeTone::Ok,
                    }),
                    french,
                    capabilities: SshHostsCapabilities::native_openssh(),
                },
            );
            assert_eq!(act_pop, None);
            assert!(has_painted_text(&shapes_pop, "deploy@prod.example.com"));
            assert!(has_painted_text(&shapes_pop, "/srv/afterlife"));
            assert!(has_painted_text(
                &shapes_pop,
                "Host key verification failed"
            ));
            assert!(has_painted_text(
                &shapes_pop,
                if french {
                    "Cible enregistrée"
                } else {
                    "Target saved"
                }
            ));
        }
    }

    // 4. Error state with Retry button emits RetryLoad on click.
    let ctx = egui::Context::default();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));
    let mut panel = SshHostsPanel::new();
    let (_, shapes_err) = run_panel_frame(
        &ctx,
        Vec2::new(820.0, 620.0),
        vec![],
        &mut panel,
        SshHostsInput {
            hosts: &[],
            loaded: true,
            loading: false,
            load_error: Some("Failed to load SSH targets"),
            notice: None,
            french: false,
            capabilities: SshHostsCapabilities::native_openssh(),
        },
    );
    assert!(has_painted_text(&shapes_err, "Failed to load SSH targets"));
    let retry_rect = find_painted_text_rect(&shapes_err, "Retry").expect("Retry button");
    let (act_retry, _) = run_panel_frame(
        &ctx,
        Vec2::new(820.0, 620.0),
        click_events(retry_rect.center()),
        &mut panel,
        SshHostsInput {
            hosts: &[],
            loaded: true,
            loading: false,
            load_error: Some("Failed to load SSH targets"),
            notice: None,
            french: false,
            capabilities: SshHostsCapabilities::native_openssh(),
        },
    );
    assert_eq!(act_retry, Some(SshHostsAction::RetryLoad));
}

#[test]
fn strict_capability_gating_and_unobserved_status_honesty() {
    let ctx = egui::Context::default();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));
    let mut panel = SshHostsPanel::new();

    let unobserved_rows = [SshHostRow::new(SshHostSpec::new("prod-box", "/srv/app"))];

    // 1. Default capabilities (`all false`) + `status: None` -> zero buttons & zero status dots.
    let (act_none, shapes_none) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 560.0),
        vec![],
        &mut panel,
        SshHostsInput {
            hosts: &unobserved_rows,
            loaded: true,
            loading: false,
            load_error: None,
            notice: None,
            french: false,
            capabilities: SshHostsCapabilities::default(),
        },
    );
    assert_eq!(act_none, None);
    assert_eq!(count_status_dots(&shapes_none), 0);
    for absent in [
        "Import",
        "Add target",
        "Connect",
        "Test",
        "Disconnect",
        "Connected",
        "Disconnected",
        "Connecting…",
    ] {
        assert!(
            !has_painted_text(&shapes_none, absent),
            "unexpected control/status {absent:?} with default capabilities and status: None"
        );
    }

    // 2. `native_openssh()` renders Add/Connect/Edit/Remove, never Import/Test/Disconnect, and
    //    still paints zero status dots while `status` is `None`.
    let (_, shapes_native) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 560.0),
        vec![],
        &mut panel,
        SshHostsInput {
            hosts: &unobserved_rows,
            loaded: true,
            loading: false,
            load_error: None,
            notice: None,
            french: false,
            capabilities: SshHostsCapabilities::native_openssh(),
        },
    );
    assert_eq!(count_status_dots(&shapes_native), 0);
    assert!(has_painted_text(&shapes_native, "Add target"));
    assert!(has_painted_text(&shapes_native, "Connect"));
    assert!(!has_painted_text(&shapes_native, "Import"));
    assert!(!has_painted_text(&shapes_native, "Test"));
    assert!(!has_painted_text(&shapes_native, "Disconnect"));

    // 3. Full capabilities + observed statuses render exact semantic dots & emit Import/Test/Disconnect.
    let full_caps = SshHostsCapabilities {
        can_add: true,
        can_edit: true,
        can_remove: true,
        can_connect: true,
        can_test: true,
        can_disconnect: true,
        can_import_config: true,
    };
    let observed_rows = [
        SshHostRow {
            host: SshHostSpec::new("connected-host", "/srv/live"),
            status: Some(SshHostStatus::Connected),
            error: None,
            busy: false,
        },
        SshHostRow {
            host: SshHostSpec::new("connecting-host", ""),
            status: Some(SshHostStatus::Connecting),
            error: None,
            busy: false,
        },
        SshHostRow {
            host: SshHostSpec::new("error-host", ""),
            status: Some(SshHostStatus::Error),
            error: Some("Timed out"),
            busy: false,
        },
        SshHostRow {
            host: SshHostSpec::new("sleeping-host", ""),
            status: Some(SshHostStatus::Sleeping),
            error: None,
            busy: false,
        },
        SshHostRow {
            host: SshHostSpec::new("offline-host", ""),
            status: Some(SshHostStatus::Disconnected),
            error: None,
            busy: false,
        },
    ];
    let (_, shapes_full) = run_panel_frame(
        &ctx,
        Vec2::new(860.0, 760.0),
        vec![],
        &mut panel,
        SshHostsInput {
            hosts: &observed_rows,
            loaded: true,
            loading: false,
            load_error: None,
            notice: None,
            french: false,
            capabilities: full_caps,
        },
    );
    assert!(has_status_dot_with_fill(
        &shapes_full,
        Color32::from_rgb(0x10, 0xb9, 0x81)
    ));
    assert!(has_status_dot_with_fill(
        &shapes_full,
        Color32::from_rgb(0xea, 0xb3, 0x08)
    ));
    assert!(has_status_dot_with_fill(
        &shapes_full,
        Color32::from_rgb(0xef, 0x44, 0x44)
    ));
    assert!(has_painted_text(&shapes_full, "Sleeping"));
    assert!(has_painted_text(&shapes_full, "Disconnected"));
    assert!(has_painted_text(&shapes_full, "Import"));
    assert!(has_painted_text(&shapes_full, "Disconnect"));
    assert!(has_painted_text(&shapes_full, "Connecting"));
    assert!(has_painted_text(&shapes_full, "Test"));
    assert!(has_painted_text(&shapes_full, "Retry"));

    let import_rect = find_painted_text_rect(&shapes_full, "Import").expect("Import button");
    let (act_imp, _) = run_panel_frame(
        &ctx,
        Vec2::new(860.0, 640.0),
        click_events(import_rect.center()),
        &mut panel,
        SshHostsInput {
            hosts: &observed_rows,
            loaded: true,
            loading: false,
            load_error: None,
            notice: None,
            french: false,
            capabilities: full_caps,
        },
    );
    assert_eq!(act_imp, Some(SshHostsAction::ImportConfig));

    let disc_rect = find_painted_text_rect(&shapes_full, "Disconnect").expect("Disconnect button");
    let (act_disc, _) = run_panel_frame(
        &ctx,
        Vec2::new(860.0, 640.0),
        click_events(disc_rect.center()),
        &mut panel,
        SshHostsInput {
            hosts: &observed_rows,
            loaded: true,
            loading: false,
            load_error: None,
            notice: None,
            french: false,
            capabilities: full_caps,
        },
    );
    assert_eq!(
        act_disc,
        Some(SshHostsAction::Disconnect {
            index: 0,
            host: SshHostSpec::new("connected-host", "/srv/live"),
        })
    );

    let test_rect = find_painted_text_rect(&shapes_full, "Test").expect("Test button");
    let (act_test, _) = run_panel_frame(
        &ctx,
        Vec2::new(860.0, 640.0),
        click_events(test_rect.center()),
        &mut panel,
        SshHostsInput {
            hosts: &observed_rows,
            loaded: true,
            loading: false,
            load_error: None,
            notice: None,
            french: false,
            capabilities: full_caps,
        },
    );
    assert_eq!(
        act_test,
        Some(SshHostsAction::Test {
            index: 2,
            host: SshHostSpec::new("error-host", ""),
        })
    );
}

#[test]
fn add_and_edit_flows_and_vue_same_error_resubmit_regression_proof() {
    let ctx = egui::Context::default();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));
    let mut panel = SshHostsPanel::new();
    let rows = [
        SshHostRow::new(SshHostSpec::new("alpha", "/srv/alpha")),
        SshHostRow::new(SshHostSpec::new("beta", "/srv/beta")),
    ];
    let mk_input = || SshHostsInput {
        hosts: &rows,
        loaded: true,
        loading: false,
        load_error: None,
        notice: None,
        french: false,
        capabilities: SshHostsCapabilities::native_openssh(),
    };

    // Prime frame & click "Add target".
    let (_, shapes0) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        vec![],
        &mut panel,
        mk_input(),
    );
    let add_btn = find_painted_text_rect(&shapes0, "Add target").expect("Add target");
    let (act1, shapes1) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        click_events(add_btn.center()),
        &mut panel,
        mk_input(),
    );
    assert_eq!(act1, None);
    assert!(panel.is_form_open());
    assert!(has_painted_text(&shapes1, "Add SSH host"));
    assert!(has_painted_text(&shapes1, "Alias / user@host *"));
    assert!(has_painted_text(&shapes1, "Remote folder (optional)"));
    // Strictly zero unpersisted fields advertised in native OpenSSH mode.
    for fake_field in ["Identity File", "Proxy Command", "Jump Host", "Port"] {
        assert!(!has_painted_text(&shapes1, fake_field));
    }

    // --- Replay Vue Bug #1: empty submit -> error -> edit folder -> error clears -> resubmit -> error MUST reappear! ---
    let submit_btn = find_painted_text_rect(&shapes1, "Add Target").expect("Add Target submit");
    let (act_err1, _) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        click_events(submit_btn.center()),
        &mut panel,
        mk_input(),
    );
    assert_eq!(act_err1, None);
    let (_, shapes_err1) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        vec![],
        &mut panel,
        mk_input(),
    );
    assert!(has_painted_text(
        &shapes_err1,
        "Host or SSH config alias is required"
    ));

    // Focus folder input & type "/srv/..notes" while alias stays empty -> error clears!
    let folder_lbl =
        find_painted_text_rect(&shapes_err1, "Remote folder (optional)").expect("folder label");
    let folder_input_pos = Pos2::new(folder_lbl.left() + 40.0, folder_lbl.bottom() + 22.0);
    let _ = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        click_events(folder_input_pos),
        &mut panel,
        mk_input(),
    );
    let (_, shapes_cleared) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        vec![egui::Event::Text("/srv/..notes".into())],
        &mut panel,
        mk_input(),
    );
    assert!(
        !has_painted_text(&shapes_cleared, "Host or SSH config alias is required"),
        "editing another field must clear the stale validation error"
    );

    // Resubmit with alias STILL empty -> the exact same error MUST be shown again!
    let submit_btn2 =
        find_painted_text_rect(&shapes_cleared, "Add Target").expect("Add Target submit 2");
    let (act_err2, _) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        click_events(submit_btn2.center()),
        &mut panel,
        mk_input(),
    );
    assert_eq!(act_err2, None);
    let (_, shapes_err2) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        vec![],
        &mut panel,
        mk_input(),
    );
    assert!(
        has_painted_text(&shapes_err2, "Host or SSH config alias is required"),
        "resubmitting the same invalid state after editing another field must re-display the error"
    );

    // Now focus alias field, paste alias with invisible BOM/ZWSP, and press Enter to save!
    let alias_lbl =
        find_painted_text_rect(&shapes_err2, "Alias / user@host *").expect("alias label");
    let alias_input_pos = Pos2::new(alias_lbl.left() + 40.0, alias_lbl.bottom() + 22.0);
    let _ = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        click_events(alias_input_pos),
        &mut panel,
        mk_input(),
    );
    let _ = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        vec![egui::Event::Text("\u{200B}deploy@gamma\u{FEFF}".into())],
        &mut panel,
        mk_input(),
    );
    let (act_saved, _) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        key_press_release(Key::Enter, Modifiers::NONE),
        &mut panel,
        mk_input(),
    );
    // `Save` keeps the form & draft open until backend persistence succeeds (`cancel_dialogs()`),
    // so a disk/limit error reported via `set_form_error(String)` preserves the user's draft!
    assert!(panel.is_form_open());
    assert_eq!(
        act_saved,
        Some(SshHostsAction::Save {
            original: None,
            replacement: SshHostSpec::new("deploy@gamma", "/srv/..notes"),
        })
    );

    // Backend persistence refuses once (`String` error): draft AND error are both visible.
    panel.set_form_error(String::from(
        "Workspace exceeds 4 MiB; previous settings retained",
    ));
    assert!(panel.is_form_open());
    let (_, shapes_backend_err1) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        vec![],
        &mut panel,
        mk_input(),
    );
    assert!(has_painted_text(
        &shapes_backend_err1,
        "Workspace exceeds 4 MiB; previous settings retained"
    ));
    assert!(has_painted_substring(&shapes_backend_err1, "deploy@gamma"));
    assert!(has_painted_substring(&shapes_backend_err1, "/srv/..notes"));

    // User edits folder (`-2`), clearing the backend error, submits again, and backend fails with
    // the EXACT SAME error string -> identical error is visible again and draft is preserved!
    let folder_lbl_b = find_painted_text_rect(&shapes_backend_err1, "Remote folder (optional)")
        .expect("folder label after backend error");
    let _ = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        click_events(Pos2::new(
            folder_lbl_b.left() + 40.0,
            folder_lbl_b.bottom() + 22.0,
        )),
        &mut panel,
        mk_input(),
    );
    let (_, shapes_backend_cleared) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        vec![egui::Event::Text("/srv/..notes-2".into())],
        &mut panel,
        mk_input(),
    );
    assert!(!has_painted_text(
        &shapes_backend_cleared,
        "Workspace exceeds 4 MiB; previous settings retained"
    ));
    let submit_retry = find_painted_text_rect(&shapes_backend_cleared, "Add Target")
        .expect("Add Target retry submit");
    let (act_saved_retry, _) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        click_events(submit_retry.center()),
        &mut panel,
        mk_input(),
    );
    assert_eq!(
        act_saved_retry,
        Some(SshHostsAction::Save {
            original: None,
            replacement: SshHostSpec::new("deploy@gamma", "/srv/..notes-2"),
        })
    );
    assert!(panel.is_form_open());
    panel.set_form_error(String::from(
        "Workspace exceeds 4 MiB; previous settings retained",
    ));
    let (_, shapes_backend_err2) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        vec![],
        &mut panel,
        mk_input(),
    );
    assert!(has_painted_text(
        &shapes_backend_err2,
        "Workspace exceeds 4 MiB; previous settings retained"
    ));
    assert!(has_painted_substring(
        &shapes_backend_err2,
        "/srv/..notes-2"
    ));

    // Caller persists successfully and calls `cancel_dialogs()`.
    panel.cancel_dialogs();
    assert!(!panel.is_form_open());

    // --- Edit flow + `set_form_error` clearing + `cancel_dialogs()` ---
    panel.open_edit(1, SshHostSpec::new("beta", "/srv/beta"));
    assert!(panel.is_form_open());
    let (_, shapes_edit) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        vec![],
        &mut panel,
        mk_input(),
    );
    assert!(has_painted_text(&shapes_edit, "Edit SSH host"));
    assert!(has_painted_substring(&shapes_edit, "Editing beta"));

    panel.set_form_error("Disk write failed");
    let (_, shapes_ferr) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        vec![],
        &mut panel,
        mk_input(),
    );
    assert!(has_painted_text(&shapes_ferr, "Disk write failed"));

    // Typing clears `set_form_error`, and submitting emits `Save { original: Some((1, ...)), replacement }`.
    let folder_lbl_e =
        find_painted_text_rect(&shapes_ferr, "Remote folder (optional)").expect("folder label");
    let _ = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        click_events(Pos2::new(
            folder_lbl_e.left() + 40.0,
            folder_lbl_e.bottom() + 22.0,
        )),
        &mut panel,
        mk_input(),
    );
    let (_, shapes_after_type) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        vec![egui::Event::Text("/srv/beta-v2".into())],
        &mut panel,
        mk_input(),
    );
    assert!(!has_painted_text(&shapes_after_type, "Disk write failed"));

    let save_changes_btn =
        find_painted_text_rect(&shapes_after_type, "Save Changes").expect("Save Changes");
    let (act_edited, _) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 720.0),
        click_events(save_changes_btn.center()),
        &mut panel,
        mk_input(),
    );
    assert_eq!(
        act_edited,
        Some(SshHostsAction::Save {
            original: Some((1, SshHostSpec::new("beta", "/srv/beta"))),
            replacement: SshHostSpec::new("beta", "/srv/beta-v2"),
        })
    );
    assert!(panel.is_form_open());
    panel.cancel_dialogs();
    assert!(!panel.is_form_open());

    panel.open_add();
    assert!(panel.is_form_open());
    panel.reset();
    assert!(!panel.is_form_open());
    assert!(!panel.is_confirm_remove_open());
}

#[test]
fn remove_confirmation_requires_explicit_confirm_and_cancel_never_deletes() {
    let ctx = egui::Context::default();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));
    let mut panel = SshHostsPanel::new();
    let rows = [
        SshHostRow::new(SshHostSpec::new("prod-01", "/srv/prod")),
        SshHostRow::new(SshHostSpec::new("stage-01", "/srv/stage")),
    ];
    let mk_input = || SshHostsInput {
        hosts: &rows,
        loaded: true,
        loading: false,
        load_error: None,
        notice: None,
        french: false,
        capabilities: SshHostsCapabilities::native_openssh(),
    };

    let (_, shapes0) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 620.0),
        vec![],
        &mut panel,
        mk_input(),
    );
    // On card 0: `[Pencil (28px)] [ gap 4px ] [Trash2 (28px)] [ gap 4px ] [Connect]`.
    let connect0 = find_painted_text_rect(&shapes0, "Connect").expect("Connect");
    // Connect button has an 8px left pad + 12px icon + 6px gap before "Connect" text; trash sits left of it.
    let trash_pos = Pos2::new(connect0.left() - 26.0 - 4.0 - 14.0, connect0.center().y);

    let (act_open, _) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 620.0),
        click_events(trash_pos),
        &mut panel,
        mk_input(),
    );
    assert_eq!(act_open, None);
    assert!(panel.is_confirm_remove_open());

    let (_, shapes_dlg) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 620.0),
        vec![],
        &mut panel,
        mk_input(),
    );
    assert!(has_painted_text(&shapes_dlg, "Remove SSH target"));
    assert!(has_painted_substring(&shapes_dlg, "prod-01"));

    // Cancel closes dialog without emitting Remove.
    let cancel_rect = find_painted_text_rect(&shapes_dlg, "Cancel").expect("Cancel");
    let (act_cancel, _) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 620.0),
        click_events(cancel_rect.center()),
        &mut panel,
        mk_input(),
    );
    assert_eq!(act_cancel, None);
    assert!(!panel.is_confirm_remove_open());

    // Settle layout after dialog close (`prev_pass` widget rects return to unshifted positions),
    // then re-open & click Remove -> emits Remove { index: 0, expected }.
    let _ = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 620.0),
        vec![],
        &mut panel,
        mk_input(),
    );
    let _ = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 620.0),
        click_events(trash_pos),
        &mut panel,
        mk_input(),
    );
    let (_, shapes_dlg2) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 620.0),
        vec![],
        &mut panel,
        mk_input(),
    );
    let remove_rect = find_painted_text_rect(&shapes_dlg2, "Remove").expect("Remove");
    let (act_rem, _) = run_panel_frame(
        &ctx,
        Vec2::new(840.0, 620.0),
        click_events(remove_rect.center()),
        &mut panel,
        mk_input(),
    );
    assert!(!panel.is_confirm_remove_open());
    assert_eq!(
        act_rem,
        Some(SshHostsAction::Remove {
            index: 0,
            expected: SshHostSpec::new("prod-01", "/srv/prod"),
        })
    );
}

#[test]
fn nested_settings_modal_escape_closes_inner_dialogs_first_with_held_key_anti_repeat() {
    let ctx = egui::Context::default();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));
    let mut panel = SshHostsPanel::new();
    let rows = [SshHostRow::new(SshHostSpec::new("prod-01", "/srv/prod"))];

    let run_in_settings_modal = |events: Vec<egui::Event>, panel: &mut SshHostsPanel| -> bool {
        let mut should_close_settings = false;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1024.0, 720.0))),
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| {
                let resp = settings_surface::show(ctx, |ui, _nav_w| {
                    settings_surface::body(ui, |ui| {
                        let _ = panel.show(
                            ui,
                            SshHostsInput {
                                hosts: &rows,
                                loaded: true,
                                loading: false,
                                load_error: None,
                                notice: None,
                                french: false,
                                capabilities: SshHostsCapabilities::native_openssh(),
                            },
                        );
                    });
                });
                should_close_settings = resp.should_close();
            },
        );
        should_close_settings
    };

    // 1. Add form open inside `settings_surface::show`:
    panel.open_add();
    assert!(!run_in_settings_modal(vec![], &mut panel));
    assert!(panel.is_form_open());

    // Frame A: Escape pressed (and still held down). Closes Add form, keeps Settings modal open!
    let closed_a = run_in_settings_modal(
        vec![egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        &mut panel,
    );
    assert!(!panel.is_form_open());
    assert!(
        !closed_a,
        "first Escape must NOT close outer Settings modal"
    );

    // Frame B: OS auto-repeat while Escape is STILL held down. Settings modal must STILL stay open!
    let closed_b = run_in_settings_modal(
        vec![egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: Modifiers::NONE,
        }],
        &mut panel,
    );
    assert!(
        !closed_b,
        "held Escape repeat after closing inner dialog must NOT close outer Settings modal"
    );

    // Frame C: Escape released.
    let closed_c = run_in_settings_modal(
        vec![egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        &mut panel,
    );
    assert!(!closed_c);

    // Frame D: Fresh Escape press+release with no inner dialog open -> closes Settings modal!
    let closed_d =
        run_in_settings_modal(key_press_release(Key::Escape, Modifiers::NONE), &mut panel);
    assert!(
        closed_d,
        "second fresh Escape with no inner dialog open must close outer Settings modal"
    );
}

#[test]
fn focus_scoped_keyboard_nav_external_focus_isolation_and_narrow_300px_bounds() {
    let ctx = egui::Context::default();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::Classic));
    let mut panel = SshHostsPanel::new();
    let rows = [
        SshHostRow::new(SshHostSpec::new("alpha-host", "/srv/alpha")),
        SshHostRow::new(SshHostSpec::new("beta-host", "/srv/beta")),
    ];

    let external_search_id = egui::Id::new("settings-nav-search");
    let mut search_query = String::new();

    let run_with_external = |ctx: &egui::Context,
                             events: Vec<egui::Event>,
                             focused: bool,
                             enabled: bool,
                             search_query: &mut String,
                             panel: &mut SshHostsPanel|
     -> (Option<SshHostsAction>, Vec<egui::epaint::ClippedShape>) {
        let mut act = None;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(860.0, 640.0))),
                events,
                focused,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = ui.add(egui::TextEdit::singleline(search_query).id(external_search_id));
                    ui.add_enabled_ui(enabled, |ui| {
                        act = panel.show(
                            ui,
                            SshHostsInput {
                                hosts: &rows,
                                loaded: true,
                                loading: false,
                                load_error: None,
                                notice: None,
                                french: false,
                                capabilities: SshHostsCapabilities::native_openssh(),
                            },
                        );
                    });
                });
            },
        );
        (act, out.shapes)
    };

    // 1. Prime frame, focus external search input, prime focus, then send ArrowDown/Enter/Delete/F2:
    let _ = run_with_external(&ctx, vec![], true, true, &mut search_query, &mut panel);
    ctx.memory_mut(|m| m.request_focus(external_search_id));
    let _ = run_with_external(&ctx, vec![], true, true, &mut search_query, &mut panel);

    for k in [Key::ArrowDown, Key::Enter, Key::Delete, Key::F2] {
        let (act, _) = run_with_external(
            &ctx,
            key_press_release(k, Modifiers::NONE),
            true,
            true,
            &mut search_query,
            &mut panel,
        );
        assert_eq!(act, None, "external search focus must isolate {k:?}");
        assert!(!panel.is_form_open());
        assert!(!panel.is_confirm_remove_open());
    }

    // 2. Click card 0 background to focus list navigation (`Sense::focusable_noninteractive`),
    //    then run the requested repaint frame so `begin_pass` promotes `list_focus_id` and
    //    locks `EventFilter { vertical_arrows: true, .. }`.
    let (_, shapes) = run_with_external(&ctx, vec![], true, true, &mut search_query, &mut panel);
    let alpha_rect = find_painted_text_rect(&shapes, "alpha-host").expect("alpha-host");
    let _ = run_with_external(
        &ctx,
        click_events(alpha_rect.center()),
        true,
        true,
        &mut search_query,
        &mut panel,
    );
    let _ = run_with_external(&ctx, vec![], true, true, &mut search_query, &mut panel);

    // ArrowDown selects card 1 (`beta-host`), Enter connects to `beta-host`!
    let _ = run_with_external(
        &ctx,
        key_press_release(Key::ArrowDown, Modifiers::NONE),
        true,
        true,
        &mut search_query,
        &mut panel,
    );
    let (act_enter, _) = run_with_external(
        &ctx,
        key_press_release(Key::Enter, Modifiers::NONE),
        true,
        true,
        &mut search_query,
        &mut panel,
    );
    assert_eq!(
        act_enter,
        Some(SshHostsAction::Connect {
            index: 1,
            host: SshHostSpec::new("beta-host", "/srv/beta"),
        })
    );

    // Disabled UI (`enabled = false`) & unfocused window (`focused = false`) ignore Enter.
    let (act_disabled, _) = run_with_external(
        &ctx,
        key_press_release(Key::Enter, Modifiers::NONE),
        true,
        false,
        &mut search_query,
        &mut panel,
    );
    assert_eq!(act_disabled, None);
    let (act_unfocused, _) = run_with_external(
        &ctx,
        key_press_release(Key::Enter, Modifiers::NONE),
        false,
        true,
        &mut search_query,
        &mut panel,
    );
    assert_eq!(act_unfocused, None);

    // Re-focus card list, Delete opens Remove confirmation (never deletes immediately).
    let (_, shapes2) = run_with_external(&ctx, vec![], true, true, &mut search_query, &mut panel);
    let beta_rect = find_painted_text_rect(&shapes2, "beta-host").expect("beta-host");
    let _ = run_with_external(
        &ctx,
        click_events(beta_rect.center()),
        true,
        true,
        &mut search_query,
        &mut panel,
    );
    let (act_del, _) = run_with_external(
        &ctx,
        key_press_release(Key::Delete, Modifiers::NONE),
        true,
        true,
        &mut search_query,
        &mut panel,
    );
    assert_eq!(act_del, None);
    assert!(panel.is_confirm_remove_open());
    panel.cancel_dialogs();

    // 3. Narrow 300px viewport truncation & bounds check with very long alias + folder.
    let long_rows = [SshHostRow {
        host: SshHostSpec::new(
            format!(
                "deploy@{}.internal.example.com",
                "very-long-node-".repeat(8)
            ),
            format!("/srv/{}/current/release", "deeply-nested-dir-".repeat(8)),
        ),
        status: Some(SshHostStatus::Connected),
        error: None,
        busy: false,
    }];
    let narrow_ctx = egui::Context::default();
    theme::apply(&narrow_ctx, true, Some(theme_palettes::ThemeId::Classic));
    let mut narrow_panel = SshHostsPanel::new();
    let (_, narrow_shapes) = run_panel_frame(
        &narrow_ctx,
        Vec2::new(300.0, 640.0),
        vec![],
        &mut narrow_panel,
        SshHostsInput {
            hosts: &long_rows,
            loaded: true,
            loading: false,
            load_error: None,
            notice: None,
            french: true,
            capabilities: SshHostsCapabilities {
                can_add: true,
                can_edit: true,
                can_remove: true,
                can_connect: true,
                can_test: true,
                can_disconnect: true,
                can_import_config: true,
            },
        },
    );
    for (txt, rect, clip, _) in painted_texts(&narrow_shapes) {
        let visible_right = rect.right().min(clip.right());
        assert!(
            visible_right <= 301.5,
            "narrow 300px text {txt:?} overflowed right bound: rect={rect:?}, clip={clip:?}"
        );
    }
}

#[test]
fn edit_form_auto_expands_advanced_for_tmux_off_or_daemon_on() {
    let mut panel = SshHostsPanel::new();
    panel.open_edit(0, SshHostSpec::new("plain", "/srv"));
    assert!(panel.is_form_open());
    assert!(!panel.is_advanced_open());

    let mut tmux_off = SshHostSpec::new("no-tmux", "/srv");
    tmux_off.persistent_sessions = Some(false);
    panel.open_edit(0, tmux_off);
    assert!(panel.is_advanced_open());

    let mut daemon = SshHostSpec::new("daemon", "/srv");
    daemon.host_daemon = true;
    panel.open_edit(0, daemon);
    assert!(panel.is_advanced_open());

    panel.open_add();
    assert!(!panel.is_advanced_open());
}
