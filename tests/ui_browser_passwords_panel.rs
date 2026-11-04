//! Standalone headless `egui` + `AccessKit` integration tests for
//! `src/browser_passwords_panel.rs` (`BrowserPasswords.vue` parity).

#[allow(dead_code)]
#[path = "../src/browser_passwords_model.rs"]
mod browser_passwords_model;
#[path = "../src/browser_passwords_panel.rs"]
mod browser_passwords_panel;
#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/theme.rs"]
mod theme;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

use browser_passwords_model::{BatchProfileResultRow, PAGE_SIZE, VaultEntryMetadata};
use browser_passwords_panel::{
    BatchResultsInput, BrowserPasswordsAction, BrowserPasswordsInput, BrowserPasswordsPanel,
    BrowserProfileSource, ImportProgress, StatusBanner,
};
use eframe::egui::accesskit::{Role, Toggled};
use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, Rect, Shape, Vec2};
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

fn find_painted_text_rect(shapes: &[egui::epaint::ClippedShape], exact: &str) -> Option<Rect> {
    painted_texts(shapes)
        .into_iter()
        .find(|(text, _, _)| text == exact)
        .map(|(_, rect, _)| rect)
}

fn pointer_down_events(pos: Pos2) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn pointer_up_events(pos: Pos2) -> Vec<Event> {
    vec![Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }]
}

fn key_event(key: Key, pressed: bool, repeat: bool) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat,
        modifiers: Modifiers::NONE,
    }
}

fn find_accesskit_node_center_exact(out: &egui::FullOutput, role: Role, expected: &str) -> Pos2 {
    let update = out
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("accesskit enabled");
    for (_, node) in &update.nodes {
        if node.role() == role
            && node.label() == Some(expected)
            && let Some(bounds) = node.bounds()
        {
            return Pos2::new(
                ((bounds.x0 + bounds.x1) * 0.5) as f32,
                ((bounds.y0 + bounds.y1) * 0.5) as f32,
            );
        }
    }
    panic!("missing AccessKit node role={role:?} exact label `{expected}`");
}

fn find_accesskit_node_center_prefix(out: &egui::FullOutput, role: Role, prefix: &str) -> Pos2 {
    let update = out
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("accesskit enabled");
    for (_, node) in &update.nodes {
        if node.role() == role
            && node.label().is_some_and(|l| l.starts_with(prefix))
            && let Some(bounds) = node.bounds()
        {
            return Pos2::new(
                ((bounds.x0 + bounds.x1) * 0.5) as f32,
                ((bounds.y0 + bounds.y1) * 0.5) as f32,
            );
        }
    }
    panic!("missing AccessKit node role={role:?} label starting with `{prefix}`");
}

fn step_dialog_frame(
    ctx: &egui::Context,
    panel: &mut BrowserPasswordsPanel,
    input: BrowserPasswordsInput<'_>,
    viewport: Vec2,
    events: Vec<Event>,
) -> (Option<BrowserPasswordsAction>, bool, egui::FullOutput) {
    let mut emitted = None;
    let mut modal_should_close = false;
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
            focused: true,
            events,
            ..Default::default()
        },
        |ctx| {
            let pre_esc = panel.handle_escape(ctx);
            let resp = egui::Modal::new(egui::Id::new("test-pw-modal")).show(ctx, |ui| {
                let w = (viewport.x - 40.0).clamp(240.0, 780.0);
                ui.set_min_width(w);
                ui.set_max_width(w);
                panel.show(ui, input)
            });
            modal_should_close = resp.should_close();
            emitted = pre_esc.or(resp.inner);
        },
    );
    (emitted, modal_should_close, out)
}

/// Runs two neutral frames so any newly opened `egui::Modal` or `Order::Tooltip` popup `Area`
/// finishes `egui`'s first-frame sizing pass and paints all text galleys & AccessKit bounds.
fn stabilize_dialog(
    ctx: &egui::Context,
    panel: &mut BrowserPasswordsPanel,
    input: BrowserPasswordsInput<'_>,
    viewport: Vec2,
) -> egui::FullOutput {
    let _ = step_dialog_frame(ctx, panel, input, viewport, vec![]);
    let (_, _, out) = step_dialog_frame(ctx, panel, input, viewport, vec![]);
    out
}

/// Dispatches a two-frame primary pointer press + release at `pos`.
fn click_dialog(
    ctx: &egui::Context,
    panel: &mut BrowserPasswordsPanel,
    input: BrowserPasswordsInput<'_>,
    viewport: Vec2,
    pos: Pos2,
) -> (Option<BrowserPasswordsAction>, bool, egui::FullOutput) {
    let (a1, c1, _) = step_dialog_frame(ctx, panel, input, viewport, pointer_down_events(pos));
    let (a2, c2, out) = step_dialog_frame(ctx, panel, input, viewport, pointer_up_events(pos));
    (a1.or(a2), c1 || c2, out)
}

fn sample_sources() -> Vec<BrowserProfileSource> {
    vec![
        BrowserProfileSource {
            browser_id: "chrome".to_owned(),
            family: "chromium".to_owned(),
            profile_dir: "Default".to_owned(),
            label: "Chrome — Default".to_owned(),
        },
        BrowserProfileSource {
            browser_id: "firefox".to_owned(),
            family: "firefox".to_owned(),
            profile_dir: "work-prof".to_owned(),
            label: "Firefox — Work".to_owned(),
        },
        BrowserProfileSource {
            browser_id: "brave".to_owned(),
            family: "chromium".to_owned(),
            profile_dir: "Profile 2".to_owned(),
            label: "Brave — Personal".to_owned(),
        },
    ]
}

fn sample_entries() -> Vec<VaultEntryMetadata> {
    vec![
        VaultEntryMetadata {
            id: "id-zeta".to_owned(),
            origin: "https://zeta.example.test".to_owned(),
            username: "zoe@example.test".to_owned(),
            allow_agents: true,
        },
        VaultEntryMetadata {
            id: "id-alpha-empty".to_owned(),
            origin: "https://alpha.example.test".to_owned(),
            username: String::new(),
            allow_agents: false,
        },
        VaultEntryMetadata {
            id: "id-alpha-bob".to_owned(),
            origin: "https://alpha.example.test".to_owned(),
            username: "bob".to_owned(),
            allow_agents: false,
        },
    ]
}

#[test]
fn toolbar_trigger_header_close_and_empty_state_render_en_and_fr_parity() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();

    // 1. Toolbar `.bp-btn` trigger in English and French.
    for (french, expected_label) in [
        (false, "Saved passwords"),
        (true, "Mots de passe enregistrés"),
    ] {
        let mut trigger_clicked = false;
        let out_first = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 120.0))),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    trigger_clicked =
                        BrowserPasswordsPanel::show_trigger_button(ui, french).clicked();
                });
            },
        );
        assert!(!trigger_clicked);
        assert!(!out_first.shapes.is_empty());
        let btn_pos = find_accesskit_node_center_exact(&out_first, Role::Button, expected_label);

        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 120.0))),
                focused: true,
                events: pointer_down_events(btn_pos),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = BrowserPasswordsPanel::show_trigger_button(ui, french);
                });
            },
        );
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 120.0))),
                focused: true,
                events: pointer_up_events(btn_pos),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    trigger_clicked =
                        BrowserPasswordsPanel::show_trigger_button(ui, french).clicked();
                });
            },
        );
        assert!(
            trigger_clicked,
            "clicking toolbar trigger ({expected_label})"
        );
    }

    // 2. Empty state (`0` entries, `0` sources) in English and French + header `Close` button.
    let mut panel = BrowserPasswordsPanel::default();
    let empty_en = BrowserPasswordsInput {
        vault_revision: 1,
        entries: &[],
        sources: &[],
        can_fill_current_page: false,
        can_manage_agent_access: true,
        browser_import_available: true,
        embedded: false,
        busy: false,
        progress: None,
        status: Some(StatusBanner {
            text: "The encrypted vault could not be opened.",
            warn: true,
        }),
        batch_results: None,
        french: false,
    };
    let vp = Vec2::new(860.0, 720.0);
    let out_en = stabilize_dialog(&ctx, &mut panel, empty_en, vp);
    assert!(has_painted_text(&out_en.shapes, "Saved passwords"));
    assert!(has_painted_text(
        &out_en.shapes,
        "No browser with saved passwords was found on this computer. Import a CSV export instead."
    ));
    assert!(has_painted_text(
        &out_en.shapes,
        "No saved passwords yet. Import them from your browsers or from a CSV export."
    ));
    assert!(has_painted_text(
        &out_en.shapes,
        "The encrypted vault could not be opened."
    ));
    assert!(has_painted_text(&out_en.shapes, "0 accounts"));
    assert!(
        !has_painted_substring(&out_en.shapes, "allowed for agents"),
        "pw-agents count is hidden when entries is empty"
    );

    // Disabled `Import from all browsers` emits nothing; enabled `Import CSV…` emits `ImportCsv`; `Close` emits `Close`.
    let import_all_pos =
        find_accesskit_node_center_exact(&out_en, Role::Button, "Import from all browsers");
    let (disabled_act, _, _) = click_dialog(&ctx, &mut panel, empty_en, vp, import_all_pos);
    assert_eq!(disabled_act, None);

    let csv_pos = find_accesskit_node_center_exact(&out_en, Role::Button, "Import CSV…");
    let (csv_act, _, _) = click_dialog(&ctx, &mut panel, empty_en, vp, csv_pos);
    assert_eq!(csv_act, Some(BrowserPasswordsAction::ImportCsv));

    let close_pos = find_accesskit_node_center_exact(&out_en, Role::Button, "Close");
    let (close_act, _, _) = click_dialog(&ctx, &mut panel, empty_en, vp, close_pos);
    assert_eq!(close_act, Some(BrowserPasswordsAction::Close));

    // French empty state (`0 compte` singular per French `Intl.PluralRules`).
    let empty_fr = BrowserPasswordsInput {
        status: None,
        french: true,
        ..empty_en
    };
    let out_fr = stabilize_dialog(&ctx, &mut panel, empty_fr, vp);
    assert!(has_painted_text(
        &out_fr.shapes,
        "Mots de passe enregistrés"
    ));
    assert!(has_painted_text(&out_fr.shapes, "0 compte"));
    assert!(has_painted_text(
        &out_fr.shapes,
        "Aucun mot de passe enregistré pour l'instant. Importez-les de vos navigateurs ou d'un export CSV."
    ));
}

#[test]
fn populated_rows_sorting_search_pagination_and_four_tier_escape_routing() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();

    let entries: Vec<VaultEntryMetadata> = (0..250)
        .map(|i| VaultEntryMetadata {
            id: format!("entry-{i:03}"),
            origin: format!("https://site-{i:03}.example.test"),
            username: format!("user-{i:03}"),
            allow_agents: i == 0,
        })
        .collect();
    let sources = sample_sources();
    let mut panel = BrowserPasswordsPanel::new();
    let vp = Vec2::new(880.0, 860.0);

    let mk_input = |french: bool| BrowserPasswordsInput {
        vault_revision: 1,
        entries: &entries,
        sources: &sources,
        can_fill_current_page: true,
        can_manage_agent_access: true,
        browser_import_available: true,
        embedded: false,
        busy: false,
        progress: None,
        status: None,
        batch_results: None,
        french,
    };

    // 1. Initial page shows `PAGE_SIZE` (200) entries; scroll `.pw-list` down and click `"Show more (50 left)"`.
    let out_p1 = stabilize_dialog(&ctx, &mut panel, mk_input(false), vp);
    assert_eq!(panel.limit(), PAGE_SIZE);
    assert!(has_painted_text(&out_p1.shapes, "250 accounts"));
    assert!(has_painted_text(&out_p1.shapes, "1 allowed for agents"));
    assert!(has_painted_text(&out_p1.shapes, "Show more (50 left)"));

    let row0_rect = find_painted_text_rect(&out_p1.shapes, "site-000.example.test")
        .expect("first row rendered inside scroll area");
    let _ = step_dialog_frame(
        &ctx,
        &mut panel,
        mk_input(false),
        vp,
        vec![
            Event::PointerMoved(row0_rect.center()),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, -12_000.0),
                modifiers: Modifiers::NONE,
            },
        ],
    );
    // Let the wheel animation settle before using the button's hit bounds.
    for _ in 0..90 {
        let _ = step_dialog_frame(&ctx, &mut panel, mk_input(false), vp, vec![]);
    }
    let out_scrolled = stabilize_dialog(&ctx, &mut panel, mk_input(false), vp);
    let more_pos =
        find_accesskit_node_center_exact(&out_scrolled, Role::Button, "Show more (50 left)");
    let _ = click_dialog(&ctx, &mut panel, mk_input(false), vp, more_pos);
    let out_p2 = stabilize_dialog(&ctx, &mut panel, mk_input(false), vp);
    assert_eq!(panel.limit(), PAGE_SIZE * 2);
    assert!(!has_painted_substring(&out_p2.shapes, "Show more"));

    // 2. Setting a query resets pagination limit back to `PAGE_SIZE` and updates count label.
    panel.set_query("site-042");
    assert_eq!(panel.limit(), PAGE_SIZE);
    let out_filtered = stabilize_dialog(&ctx, &mut panel, mk_input(true), vp);
    assert!(has_painted_text(&out_filtered.shapes, "1 sur 250"));
    assert!(has_painted_text(
        &out_filtered.shapes,
        "1 autorisé pour les agents"
    ));

    panel.set_query("no-such-domain");
    let out_none = stabilize_dialog(&ctx, &mut panel, mk_input(true), vp);
    assert!(has_painted_text(
        &out_none.shapes,
        "Aucun compte ne correspond à « no-such-domain »."
    ));

    // 3. Four-tier Escape routing (including held key-repeat isolation!):
    // Tier A: Open profile dropdown popup -> Escape closes popup, preserves query, keeps modal open.
    panel.set_query("site-000");
    let out_before_popup = stabilize_dialog(&ctx, &mut panel, mk_input(false), vp);
    let combo_pos =
        find_accesskit_node_center_prefix(&out_before_popup, Role::ComboBox, "Browser profile:");
    let _ = click_dialog(&ctx, &mut panel, mk_input(false), vp, combo_pos);
    assert!(panel.is_profile_popup_open());
    let out_with_popup = stabilize_dialog(&ctx, &mut panel, mk_input(false), vp);
    assert!(has_painted_text(&out_with_popup.shapes, "Firefox — Work"));

    let (esc_popup_act, esc_popup_close, _) = step_dialog_frame(
        &ctx,
        &mut panel,
        mk_input(false),
        vp,
        vec![key_event(Key::Escape, true, false)],
    );
    assert_eq!(esc_popup_act, None);
    assert!(!esc_popup_close);
    assert!(!panel.is_profile_popup_open());
    assert_eq!(panel.query(), "site-000");

    // Held repeat frame while Escape is still down MUST NOT clear the query or close the modal:
    let (esc_repeat_act, esc_repeat_close, _) = step_dialog_frame(
        &ctx,
        &mut panel,
        mk_input(false),
        vp,
        vec![key_event(Key::Escape, true, true)],
    );
    assert_eq!(esc_repeat_act, None);
    assert!(!esc_repeat_close);
    assert_eq!(panel.query(), "site-000");
    let _ = step_dialog_frame(
        &ctx,
        &mut panel,
        mk_input(false),
        vp,
        vec![key_event(Key::Escape, false, false)],
    );

    // Tier B: Open delete confirmation -> Escape cancels confirmation, preserves query, keeps modal open.
    let out_for_rm = stabilize_dialog(&ctx, &mut panel, mk_input(false), vp);
    let rm_pos = find_accesskit_node_center_exact(
        &out_for_rm,
        Role::Button,
        "Remove user-000 on site-000.example.test from Tessel",
    );
    let _ = click_dialog(&ctx, &mut panel, mk_input(false), vp, rm_pos);
    assert!(panel.has_pending_confirmation());

    let (esc_conf_act, esc_conf_close, _) = step_dialog_frame(
        &ctx,
        &mut panel,
        mk_input(false),
        vp,
        vec![
            key_event(Key::Escape, true, false),
            key_event(Key::Escape, false, false),
        ],
    );
    assert_eq!(esc_conf_act, None);
    assert!(!esc_conf_close);
    assert!(!panel.has_pending_confirmation());
    assert_eq!(panel.query(), "site-000");

    // Tier C: Non-empty search query -> Escape clears query and keeps modal open.
    let (esc_q_act, esc_q_close, _) = step_dialog_frame(
        &ctx,
        &mut panel,
        mk_input(false),
        vp,
        vec![
            key_event(Key::Escape, true, false),
            key_event(Key::Escape, false, false),
        ],
    );
    assert_eq!(esc_q_act, None);
    assert!(!esc_q_close);
    assert_eq!(panel.query(), "");

    // Tier D: Empty search query -> Escape emits `BrowserPasswordsAction::Close`.
    let (esc_close_act, _, _) = step_dialog_frame(
        &ctx,
        &mut panel,
        mk_input(false),
        vp,
        vec![
            key_event(Key::Escape, true, false),
            key_event(Key::Escape, false, false),
        ],
    );
    assert_eq!(esc_close_act, Some(BrowserPasswordsAction::Close));
}

#[test]
fn profile_dropdown_import_actions_batch_results_and_how_to_export_guide() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();

    let entries = sample_entries();
    let sources = sample_sources();
    let batch_items = vec![
        BatchProfileResultRow {
            index: 0,
            label: "Chrome — Default".to_owned(),
            ok: true,
            warn: false,
            csv: false,
            text: "2 new".to_owned(),
        },
        BatchProfileResultRow {
            index: 2,
            label: "Brave — Personal".to_owned(),
            ok: false,
            warn: true,
            csv: true,
            text: "Browser protection or permissions prevent access. Use a CSV export.".to_owned(),
        },
    ];
    let mut panel = BrowserPasswordsPanel::new();
    let vp = Vec2::new(900.0, 920.0);

    let mk_input =
        |busy: bool, progress: Option<ImportProgress>, title: &'static str, french: bool| {
            BrowserPasswordsInput {
                vault_revision: 1,
                entries: &entries,
                sources: &sources,
                can_fill_current_page: true,
                can_manage_agent_access: true,
                browser_import_available: true,
                embedded: false,
                busy,
                progress,
                status: Some(StatusBanner {
                    text: "Updated.",
                    warn: false,
                }),
                batch_results: Some(BatchResultsInput {
                    title,
                    items: &batch_items,
                }),
                french,
            }
        };

    // 1. Progress label in EN & FR (including empty `batch.title` falling back to `progressLabel`).
    let out_prog_en = stabilize_dialog(
        &ctx,
        &mut panel,
        mk_input(true, Some(ImportProgress { done: 2, total: 3 }), "", false),
        vp,
    );
    assert!(has_painted_text(&out_prog_en.shapes, "Importing 2 of 3…"));

    let out_prog_fr = stabilize_dialog(
        &ctx,
        &mut panel,
        mk_input(true, Some(ImportProgress { done: 2, total: 3 }), "", true),
        vp,
    );
    assert!(has_painted_text(&out_prog_fr.shapes, "Import 2 sur 3…"));

    // While `busy == true`, clicking `Masquer` on batch results is disabled.
    let hide_busy_pos = find_accesskit_node_center_exact(&out_prog_fr, Role::Button, "Masquer");
    let (busy_dismiss, _, _) = click_dialog(
        &ctx,
        &mut panel,
        mk_input(true, None, "3 profils lus", true),
        vp,
        hide_busy_pos,
    );
    assert_eq!(busy_dismiss, None);

    // 2. Idle state: click `Import from all browsers`.
    let idle_en = mk_input(
        false,
        None,
        "3 profiles read: 2 new accounts. 1 need attention.",
        false,
    );
    let out_idle = stabilize_dialog(&ctx, &mut panel, idle_en, vp);
    let badge = find_painted_text_rect(&out_idle.shapes, "1 allowed for agents")
        .expect("agent count badge");
    assert!(
        badge.right() <= vp.x,
        "agent badge must fit the viewport: {badge:?}"
    );

    let import_all_pos =
        find_accesskit_node_center_exact(&out_idle, Role::Button, "Import from all browsers");
    let (all_act, _, _) = click_dialog(&ctx, &mut panel, idle_en, vp, import_all_pos);
    assert_eq!(all_act, Some(BrowserPasswordsAction::ImportAllProfiles));

    // 3. Select `Firefox — Work` (index 1) from dropdown and click `Import this profile`.
    panel.set_selected_source(Some(0));
    let out_ready = stabilize_dialog(&ctx, &mut panel, idle_en, vp);
    let combo_pos =
        find_accesskit_node_center_prefix(&out_ready, Role::ComboBox, "Browser profile:");
    let _ = click_dialog(&ctx, &mut panel, idle_en, vp, combo_pos);
    let out_popup = stabilize_dialog(&ctx, &mut panel, idle_en, vp);
    let ff_rect = find_painted_text_rect(&out_popup.shapes, "Firefox — Work")
        .expect("Firefox option rendered in popup");
    let _ = click_dialog(&ctx, &mut panel, idle_en, vp, ff_rect.center());
    assert_eq!(panel.selected_source(), Some(1));
    assert!(!panel.is_profile_popup_open());

    let out_after_pick = stabilize_dialog(&ctx, &mut panel, idle_en, vp);
    let import_one_pos =
        find_accesskit_node_center_exact(&out_after_pick, Role::Button, "Import this profile");
    let (one_act, _, _) = click_dialog(&ctx, &mut panel, idle_en, vp, import_one_pos);
    assert_eq!(
        one_act,
        Some(BrowserPasswordsAction::ImportProfile {
            source_index: 1,
            browser_id: "firefox".to_owned(),
            profile_dir: "work-prof".to_owned(),
        })
    );

    // 4. Clicking `How to export` on Brave's `.pw-result` row selects index 2 and opens export help;
    //    `set_export_help_open(false)` and clicking footer summary toggle also work cleanly.
    assert!(!panel.export_help_open());
    let how_pos = find_accesskit_node_center_exact(&out_after_pick, Role::Button, "How to export");
    let _ = click_dialog(&ctx, &mut panel, idle_en, vp, how_pos);
    let mut out_export = stabilize_dialog(&ctx, &mut panel, idle_en, vp);
    let summary =
        find_accesskit_node_center_prefix(&out_export, Role::Button, "How to export passwords");
    for _ in 0..20 {
        if has_painted_substring(&out_export.shapes, "brave://password-manager/settings") {
            break;
        }
        let _ = step_dialog_frame(
            &ctx,
            &mut panel,
            idle_en,
            vp,
            vec![
                Event::PointerMoved(summary),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0.0, -12.0),
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        for _ in 0..10 {
            out_export = stabilize_dialog(&ctx, &mut panel, idle_en, vp);
        }
    }
    assert!(panel.export_help_open());
    assert_eq!(panel.selected_source(), Some(2));
    assert!(
        has_painted_substring(&out_export.shapes, "brave://password-manager/settings"),
        "export text and clips: {:?}",
        painted_texts(&out_export.shapes)
    );

    panel.set_export_help_open(false);
    assert!(!panel.export_help_open());
    let out_closed_help = stabilize_dialog(&ctx, &mut panel, idle_en, vp);
    let summary_pos = find_accesskit_node_center_exact(
        &out_closed_help,
        Role::Button,
        "How to export passwords for this browser",
    );
    let _ = click_dialog(&ctx, &mut panel, idle_en, vp, summary_pos);
    assert!(panel.export_help_open());

    // 5. Clicking `Hide` dismisses batch results.
    let out_before_hide = stabilize_dialog(&ctx, &mut panel, idle_en, vp);
    let hide_pos = find_accesskit_node_center_exact(&out_before_hide, Role::Button, "Hide");
    let (dismiss_act, _, _) = click_dialog(&ctx, &mut panel, idle_en, vp, hide_pos);
    assert_eq!(
        dismiss_act,
        Some(BrowserPasswordsAction::DismissBatchResults)
    );
}

#[test]
fn row_allow_fill_delete_confirmation_and_bulk_allow_all_confirmation_flow() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();

    let entries = sample_entries();
    let sources = sample_sources();
    let mut panel = BrowserPasswordsPanel::new();
    let vp = Vec2::new(880.0, 820.0);

    let mk_input = |can_fill: bool| BrowserPasswordsInput {
        vault_revision: 3,
        entries: &entries,
        sources: &sources,
        can_fill_current_page: can_fill,
        can_manage_agent_access: true,
        browser_import_available: true,
        embedded: false,
        busy: false,
        progress: None,
        status: None,
        batch_results: None,
        french: false,
    };

    // 1. `Fill` disabled when `can_fill_current_page == false`, enabled when `true`.
    let out_no_fill = stabilize_dialog(&ctx, &mut panel, mk_input(false), vp);
    assert!(has_painted_text(&out_no_fill.shapes, "(no username)"));

    let fill_rect = find_painted_text_rect(&out_no_fill.shapes, "Fill").expect("Fill button");
    let (no_fill_act, _, _) =
        click_dialog(&ctx, &mut panel, mk_input(false), vp, fill_rect.center());
    assert_eq!(no_fill_act, None);

    let out_can_fill = stabilize_dialog(&ctx, &mut panel, mk_input(true), vp);
    let fill_enabled_rect =
        find_painted_text_rect(&out_can_fill.shapes, "Fill").expect("enabled Fill button");
    let (ok_fill_act, _, _) = click_dialog(
        &ctx,
        &mut panel,
        mk_input(true),
        vp,
        fill_enabled_rect.center(),
    );
    assert_eq!(
        ok_fill_act,
        Some(BrowserPasswordsAction::Fill {
            id: "id-alpha-empty".to_owned(),
            vault_revision: 3,
        })
    );

    // 2. Per-row `allowAgents` toggle emits `SetAllowAgents`.
    let bob_sw_pos = find_accesskit_node_center_exact(
        &out_can_fill,
        Role::CheckBox,
        "Allow my local agents to use bob on alpha.example.test",
    );
    let (allow_act, _, _) = click_dialog(&ctx, &mut panel, mk_input(true), vp, bob_sw_pos);
    assert_eq!(
        allow_act,
        Some(BrowserPasswordsAction::SetAllowAgents {
            id: "id-alpha-bob".to_owned(),
            allow_agents: true,
            vault_revision: 3,
        })
    );

    // 3. Row `Trash2` opens confirmation first; `Cancel` aborts; confirming `Remove` emits `Delete`.
    let rm_empty_pos = find_accesskit_node_center_exact(
        &out_can_fill,
        Role::Button,
        "Remove (no username) on alpha.example.test from Tessel",
    );
    let (rm_open_act, _, _) = click_dialog(&ctx, &mut panel, mk_input(true), vp, rm_empty_pos);
    assert_eq!(rm_open_act, None, "must ask confirmation before deleting");
    assert_eq!(
        panel
            .pending_confirmation_prompt()
            .map(|p| p.title.as_str()),
        Some("Remove this login from Tessel?")
    );
    let out_rm_prompt = stabilize_dialog(&ctx, &mut panel, mk_input(true), vp);
    assert!(has_painted_text(
        &out_rm_prompt.shapes,
        "(no username) on alpha.example.test. It stays saved in your browser."
    ));

    let cancel_pos = find_accesskit_node_center_exact(&out_rm_prompt, Role::Button, "Cancel");
    let (cancel_act, _, _) = click_dialog(&ctx, &mut panel, mk_input(true), vp, cancel_pos);
    assert_eq!(cancel_act, None);
    assert!(!panel.has_pending_confirmation());

    // Open again and click exact `Remove` button on confirmation banner.
    let out_ready_rm2 = stabilize_dialog(&ctx, &mut panel, mk_input(true), vp);
    let rm_empty_pos2 = find_accesskit_node_center_exact(
        &out_ready_rm2,
        Role::Button,
        "Remove (no username) on alpha.example.test from Tessel",
    );
    let _ = click_dialog(&ctx, &mut panel, mk_input(true), vp, rm_empty_pos2);
    let out_rm_prompt2 = stabilize_dialog(&ctx, &mut panel, mk_input(true), vp);
    let confirm_rm_pos = find_accesskit_node_center_exact(&out_rm_prompt2, Role::Button, "Remove");
    let (del_act, _, _) = click_dialog(&ctx, &mut panel, mk_input(true), vp, confirm_rm_pos);
    assert_eq!(
        del_act,
        Some(BrowserPasswordsAction::Delete {
            id: "id-alpha-empty".to_owned(),
            vault_revision: 3,
        })
    );

    // 4. Header bulk `allow-all` switch:
    // 2 of 3 accounts are currently `allow_agents: false` -> prompts `"Allow 2 accounts..."` first!
    let out_before_bulk = stabilize_dialog(&ctx, &mut panel, mk_input(true), vp);
    let bulk_pos = find_accesskit_node_center_exact(
        &out_before_bulk,
        Role::CheckBox,
        "Allow my local agents every account shown (3)",
    );
    let (bulk_ask_act, _, _) = click_dialog(&ctx, &mut panel, mk_input(true), vp, bulk_pos);
    assert_eq!(bulk_ask_act, None, "bulk allow must ask confirmation first");
    let out_bulk_prompt = stabilize_dialog(&ctx, &mut panel, mk_input(true), vp);
    assert!(has_painted_text(
        &out_bulk_prompt.shapes,
        "Allow 2 accounts for your local agents?"
    ));

    let allow_2_pos = find_accesskit_node_center_exact(&out_bulk_prompt, Role::Button, "Allow 2");
    let (bulk_confirm_act, _, _) = click_dialog(&ctx, &mut panel, mk_input(true), vp, allow_2_pos);
    assert_eq!(
        bulk_confirm_act,
        Some(BrowserPasswordsAction::SetBulkAllowAgents {
            target_ids: vec!["id-alpha-empty".to_owned(), "id-alpha-bob".to_owned()],
            allow_agents: true,
            vault_revision: 3,
        })
    );

    // 5. Filtering to `zeta` leaves only 1 account which is ALREADY `allow_agents: true` ->
    // clicking bulk switch immediately forbids without asking confirmation!
    panel.set_query("zeta");
    let out_zeta = stabilize_dialog(&ctx, &mut panel, mk_input(true), vp);
    let forbid_pos = find_accesskit_node_center_exact(
        &out_zeta,
        Role::CheckBox,
        "Forbid my local agents every account shown (1)",
    );
    let (forbid_act, _, _) = click_dialog(&ctx, &mut panel, mk_input(true), vp, forbid_pos);
    assert!(!panel.has_pending_confirmation());
    assert_eq!(
        forbid_act,
        Some(BrowserPasswordsAction::SetBulkAllowAgents {
            target_ids: vec!["id-zeta".to_owned()],
            allow_agents: false,
            vault_revision: 3,
        })
    );
}

#[test]
fn responsive_360px_narrow_780px_wide_all_themes_and_light_mode_without_secrets() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();

    let entries = sample_entries();
    let sources = sample_sources();
    let mut panel = BrowserPasswordsPanel::new();

    // 1. Narrow `360px` container (`<= 560px` breakpoint, French): stacked site + username, no `.pw-list-head`, zero horizontal overflow.
    let mut narrow_rect = Rect::NOTHING;
    let mut run_narrow = || {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 900.0))),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                let resp = egui::SidePanel::left("pw_narrow_360")
                    .default_width(360.0)
                    .max_width(360.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        let _ = panel.show(
                            ui,
                            BrowserPasswordsInput {
                                vault_revision: 1,
                                entries: &entries,
                                sources: &sources,
                                can_fill_current_page: true,
                                can_manage_agent_access: true,
                                browser_import_available: true,
                                embedded: false,
                                busy: false,
                                progress: None,
                                status: None,
                                batch_results: None,
                                french: true,
                            },
                        );
                    });
                narrow_rect = resp.response.rect;
                egui::CentralPanel::default().show(ctx, |_| {});
            },
        )
    };
    let _ = run_narrow();
    let out_narrow = run_narrow();

    assert!(narrow_rect.width() <= 364.0);
    assert!(
        !has_painted_text(&out_narrow.shapes, "NOM D'UTILISATEUR"),
        ".pw-list-head hides at <= 560px matching BrowserPasswords.vue"
    );
    for (text, rect, clip) in painted_texts(&out_narrow.shapes) {
        let vis = rect.intersect(clip);
        if vis.is_positive() && vis.min.x >= narrow_rect.min.x - 2.0 {
            assert!(
                vis.max.x <= narrow_rect.max.x + 2.0,
                "narrow text `{text}` overflowed 360px container: vis={vis:?}, panel={narrow_rect:?}"
            );
        }
    }
    let zeta_site = find_painted_text_rect(&out_narrow.shapes, "zeta.example.test")
        .expect("zeta site rendered");
    let zeta_user =
        find_painted_text_rect(&out_narrow.shapes, "zoe@example.test").expect("zeta user rendered");
    assert!(
        zeta_user.center().y > zeta_site.center().y + 8.0,
        "at <= 560px, username stacks below site"
    );

    // 2. Wide `780px` modal container across all 7 dark themes + light mode: `.pw-list-head` shown, inline site + username on same row, AccessKit checkboxes have Toggled state.
    panel.reset_on_open();
    for theme_id in THEMES {
        theme::apply(&ctx, true, Some(theme_id));
        let out_theme = stabilize_dialog(
            &ctx,
            &mut panel,
            BrowserPasswordsInput {
                vault_revision: 1,
                entries: &entries,
                sources: &sources,
                can_fill_current_page: true,
                can_manage_agent_access: true,
                browser_import_available: true,
                embedded: false,
                busy: false,
                progress: None,
                status: None,
                batch_results: None,
                french: false,
            },
            Vec2::new(860.0, 820.0),
        );
        assert!(has_painted_text(&out_theme.shapes, "SITE"));
        assert!(has_painted_text(&out_theme.shapes, "USERNAME"));
        assert!(has_painted_text(&out_theme.shapes, "AGENTS"));
    }

    theme::apply(&ctx, false, None);
    let out_wide = stabilize_dialog(
        &ctx,
        &mut panel,
        BrowserPasswordsInput {
            vault_revision: 1,
            entries: &entries,
            sources: &sources,
            can_fill_current_page: true,
            can_manage_agent_access: true,
            browser_import_available: true,
            embedded: false,
            busy: false,
            progress: None,
            status: None,
            batch_results: None,
            french: false,
        },
        Vec2::new(860.0, 820.0),
    );
    let wide_site =
        find_painted_text_rect(&out_wide.shapes, "zeta.example.test").expect("wide zeta site");
    let wide_user =
        find_painted_text_rect(&out_wide.shapes, "zoe@example.test").expect("wide zeta user");
    assert!(
        (wide_site.center().y - wide_user.center().y).abs() <= 2.0
            && wide_user.min.x > wide_site.max.x,
        "at > 560px, site and username sit inline on the same row"
    );

    let checkboxes: Vec<(String, Option<Toggled>)> = out_wide
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("accesskit update")
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::CheckBox)
        .map(|(_, n)| (n.label().unwrap_or_default().to_owned(), n.toggled()))
        .collect();
    assert!(checkboxes.iter().any(|(l, t)| {
        l == "Allow my local agents to use zoe@example.test on zeta.example.test"
            && *t == Some(Toggled::True)
    }));
    assert!(checkboxes.iter().any(|(l, t)| {
        l == "Allow my local agents to use bob on alpha.example.test" && *t == Some(Toggled::False)
    }));

    // 3. Structural & debug-format proof that panel state and actions carry metadata only.
    let dbg_dump = format!("{panel:?}");
    assert!(!dbg_dump.contains("password"));
}

#[test]
fn owner_only_honesty_vault_revision_invalidation_and_profile_identity_sync() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();

    let entries = sample_entries();
    let sources = sample_sources();
    let mut panel = BrowserPasswordsPanel::new();
    let vp = Vec2::new(880.0, 820.0);

    // 1. Owner-only runtime (`can_manage_agent_access == false`, `sources: &[]`):
    //    - Zero agent checkboxes (`Role::CheckBox`), no `"AGENTS"` column, no `"allowed for agents"` claim.
    //    - Honest EN & FR owner-only badges and footers.
    //    - CSV import, Fill, and Delete (with confirmation + `vault_revision`) work cleanly.
    let owner_en = BrowserPasswordsInput {
        vault_revision: 7,
        entries: &entries,
        sources: &[],
        can_fill_current_page: true,
        can_manage_agent_access: false,
        browser_import_available: true,
        embedded: false,
        busy: false,
        progress: None,
        status: None,
        batch_results: None,
        french: false,
    };
    let out_owner_en = stabilize_dialog(&ctx, &mut panel, owner_en, vp);
    assert_eq!(panel.observed_vault_revision(), Some(7));
    assert_eq!(panel.selected_source(), None);
    assert_eq!(panel.selected_profile_identity(), None);
    assert!(has_painted_text(&out_owner_en.shapes, "SITE"));
    assert!(has_painted_text(&out_owner_en.shapes, "USERNAME"));
    assert!(
        !has_painted_text(&out_owner_en.shapes, "AGENTS"),
        "AGENTS column must be omitted when can_manage_agent_access is false"
    );
    assert!(has_painted_text(
        &out_owner_en.shapes,
        "Agents: off (owner fill only)"
    ));
    assert!(!has_painted_substring(
        &out_owner_en.shapes,
        "allowed for agents"
    ));
    assert!(has_painted_substring(
        &out_owner_en.shapes,
        "Only you can fill these logins — agent vault access is disabled."
    ));
    let owner_checkbox_count = out_owner_en
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("accesskit update")
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::CheckBox)
        .count();
    assert_eq!(
        owner_checkbox_count, 0,
        "owner-only mode must expose zero fictitious agent-permission toggles"
    );

    let fill_rect =
        find_painted_text_rect(&out_owner_en.shapes, "Fill").expect("owner-only Fill button");
    let (owner_fill_act, _, _) = click_dialog(&ctx, &mut panel, owner_en, vp, fill_rect.center());
    assert_eq!(
        owner_fill_act,
        Some(BrowserPasswordsAction::Fill {
            id: "id-alpha-empty".to_owned(),
            vault_revision: 7,
        })
    );

    let owner_fr = BrowserPasswordsInput {
        french: true,
        ..owner_en
    };
    let out_owner_fr = stabilize_dialog(&ctx, &mut panel, owner_fr, vp);
    assert!(has_painted_text(
        &out_owner_fr.shapes,
        "Agents : désactivés (propriétaire seul)"
    ));
    assert!(has_painted_substring(
        &out_owner_fr.shapes,
        "Vous seul pouvez remplir ces connexions"
    ));

    // 2. Vault revision fencing (`vault_revision`) & confirmation invalidation:
    //    - Opening BulkAllow at rev 10 then bumping `vault_revision` to 11 (e.g. CSV/profile import
    //      replacing a secret under the same `id` with `allow_agents: false`) immediately cancels
    //      the stale confirmation so it can NEVER re-authorize the replaced secret!
    let full_rev = |rev: u64, can_agents: bool| BrowserPasswordsInput {
        vault_revision: rev,
        entries: &entries,
        sources: &sources,
        can_fill_current_page: true,
        can_manage_agent_access: can_agents,
        browser_import_available: true,
        embedded: false,
        busy: false,
        progress: None,
        status: None,
        batch_results: None,
        french: false,
    };
    let out_r10 = stabilize_dialog(&ctx, &mut panel, full_rev(10, true), vp);
    let bulk_pos = find_accesskit_node_center_exact(
        &out_r10,
        Role::CheckBox,
        "Allow my local agents every account shown (3)",
    );
    let _ = click_dialog(&ctx, &mut panel, full_rev(10, true), vp, bulk_pos);
    assert!(panel.has_pending_confirmation());
    assert_eq!(panel.observed_vault_revision(), Some(10));

    let out_r11 = stabilize_dialog(&ctx, &mut panel, full_rev(11, true), vp);
    assert!(!panel.has_pending_confirmation());
    assert_eq!(panel.observed_vault_revision(), Some(11));
    assert!(!has_painted_text(
        &out_r11.shapes,
        "Allow 2 accounts for your local agents?"
    ));

    // Pending Delete confirmation also invalidates on `vault_revision` bump AND via `invalidate_pending_confirmation()`.
    let rm_pos = find_accesskit_node_center_exact(
        &out_r11,
        Role::Button,
        "Remove bob on alpha.example.test from Tessel",
    );
    let _ = click_dialog(&ctx, &mut panel, full_rev(11, true), vp, rm_pos);
    assert!(panel.has_pending_confirmation());
    let _ = stabilize_dialog(&ctx, &mut panel, full_rev(12, true), vp);
    assert!(!panel.has_pending_confirmation());

    let out_r12 = stabilize_dialog(&ctx, &mut panel, full_rev(12, true), vp);
    let rm_pos2 = find_accesskit_node_center_exact(
        &out_r12,
        Role::Button,
        "Remove bob on alpha.example.test from Tessel",
    );
    let _ = click_dialog(&ctx, &mut panel, full_rev(12, true), vp, rm_pos2);
    assert!(panel.has_pending_confirmation());
    panel.invalidate_pending_confirmation();
    assert!(!panel.has_pending_confirmation());

    // Switching `can_manage_agent_access` to `false` also drops any pending `BulkAllow` confirmation.
    let bulk_pos2 = find_accesskit_node_center_exact(
        &out_r12,
        Role::CheckBox,
        "Allow my local agents every account shown (3)",
    );
    let _ = click_dialog(&ctx, &mut panel, full_rev(12, true), vp, bulk_pos2);
    assert!(panel.has_pending_confirmation());
    let _ = stabilize_dialog(&ctx, &mut panel, full_rev(12, false), vp);
    assert!(!panel.has_pending_confirmation());

    // 3. Profile identity `(browser_id, profile_dir)` preservation across reorders and clean reset on removal:
    let out_src_init = stabilize_dialog(&ctx, &mut panel, full_rev(12, true), vp);
    let combo_pos =
        find_accesskit_node_center_prefix(&out_src_init, Role::ComboBox, "Browser profile:");
    let _ = click_dialog(&ctx, &mut panel, full_rev(12, true), vp, combo_pos);
    let out_popup = stabilize_dialog(&ctx, &mut panel, full_rev(12, true), vp);
    let ff_rect = find_painted_text_rect(&out_popup.shapes, "Firefox — Work")
        .expect("Firefox option rendered");
    let _ = click_dialog(&ctx, &mut panel, full_rev(12, true), vp, ff_rect.center());
    assert_eq!(panel.selected_source(), Some(1));
    assert_eq!(
        panel.selected_profile_identity(),
        Some(("firefox", "work-prof"))
    );

    // Reorder `[Chrome, Firefox, Brave]` -> `[Brave, Chrome, Firefox]` (Firefox moves from 1 to 2):
    let reordered = vec![sources[2].clone(), sources[0].clone(), sources[1].clone()];
    let reordered_in = BrowserPasswordsInput {
        sources: &reordered,
        ..full_rev(12, true)
    };
    let out_reordered = stabilize_dialog(&ctx, &mut panel, reordered_in, vp);
    assert_eq!(panel.selected_source(), Some(2));
    assert_eq!(
        panel.selected_profile_identity(),
        Some(("firefox", "work-prof"))
    );
    let import_one_pos =
        find_accesskit_node_center_exact(&out_reordered, Role::Button, "Import this profile");
    let (reordered_import_act, _, _) =
        click_dialog(&ctx, &mut panel, reordered_in, vp, import_one_pos);
    assert_eq!(
        reordered_import_act,
        Some(BrowserPasswordsAction::ImportProfile {
            source_index: 2,
            browser_id: "firefox".to_owned(),
            profile_dir: "work-prof".to_owned(),
        })
    );

    // Remove Firefox completely (`[Brave, Chrome]`) -> resets cleanly to index `0` (`Brave`).
    let without_ff = vec![sources[2].clone(), sources[0].clone()];
    let without_ff_in = BrowserPasswordsInput {
        sources: &without_ff,
        ..full_rev(12, true)
    };
    let _ = stabilize_dialog(&ctx, &mut panel, without_ff_in, vp);
    assert_eq!(panel.selected_source(), Some(0));
    assert_eq!(
        panel.selected_profile_identity(),
        Some(("brave", "Profile 2"))
    );
}

#[test]
fn unavailable_browser_import_is_explicit_and_csv_remains_available() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let sources = sample_sources();
    let vp = Vec2::new(880.0, 820.0);
    for french in [false, true] {
        let mut panel = BrowserPasswordsPanel::new();
        let input = BrowserPasswordsInput {
            vault_revision: 0,
            entries: &[],
            sources: &sources,
            can_fill_current_page: false,
            can_manage_agent_access: false,
            browser_import_available: false,
            embedded: false,
            busy: false,
            progress: None,
            status: None,
            batch_results: None,
            french,
        };
        let out = stabilize_dialog(&ctx, &mut panel, input, vp);
        assert!(has_painted_substring(
            &out.shapes,
            if french {
                "L’import direct du navigateur n’est pas encore disponible."
            } else {
                "Direct browser import is not available yet."
            }
        ));
        let csv = find_painted_text_rect(
            &out.shapes,
            if french {
                "Importer un CSV…"
            } else {
                "Import CSV…"
            },
        )
        .expect("CSV import remains visible");
        let (action, _, _) = click_dialog(&ctx, &mut panel, input, vp, csv.center());
        assert_eq!(action, Some(BrowserPasswordsAction::ImportCsv));
    }
}

#[test]
fn embedded_export_guide_uses_parent_scroll_and_keeps_existing_focus() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut panel = BrowserPasswordsPanel::new();
    panel.set_export_help_open(true);
    let input = BrowserPasswordsInput {
        vault_revision: 0,
        entries: &[],
        sources: &[],
        can_fill_current_page: false,
        can_manage_agent_access: false,
        browser_import_available: false,
        embedded: true,
        busy: false,
        progress: None,
        status: None,
        batch_results: None,
        french: true,
    };
    let other = egui::Id::new("settings-search");
    let mut text = String::new();
    let mut output = None;
    for _ in 0..3 {
        output = Some(ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 1400.0))),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = ui.add(egui::TextEdit::singleline(&mut text).id(other));
                    response.request_focus();
                    let _ = panel.show(ui, input);
                    assert_eq!(ctx.memory(|m| m.focused()), Some(other));
                });
            },
        ));
    }
    let out = output.unwrap();
    assert!(has_painted_substring(
        &out.shapes,
        "Gestionnaire de mots de passe de Google"
    ));
    assert!(has_painted_substring(&out.shapes, "5."));
    let nodes = &out.platform_output.accesskit_update.as_ref().unwrap().nodes;
    assert!(
        !nodes
            .iter()
            .any(|(_, n)| n.role() == Role::Button && n.label() == Some("Fermer"))
    );
}
