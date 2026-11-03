#[path = "../src/help_panel.rs"]
mod help_panel;
#[allow(dead_code)]
#[path = "../src/shortcuts.rs"]
mod shortcuts;

use eframe::egui;
use help_panel::{
    HelpPanel, filter_supported_shortcuts, is_shortcut_supported, native_shortcut_text,
};

const APP_INITIAL_SUPPORTED_KEYS: &[&str] = &[
    "Ctrl+Shift+T",
    "Ctrl+Shift+P",
    "Ctrl+Shift+J",
    "Ctrl+Shift+E",
    "Ctrl+Shift+O",
    "Alt+Arrows",
    "Esc",
    "Ctrl+S",
    "Ctrl+Shift+K",
    "Ctrl+Shift+X",
    "Ctrl+Shift+G",
    "Ctrl+,",
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

fn run_modal_frame(
    ctx: &egui::Context,
    panel: &mut HelpPanel,
    french: bool,
    supported_keys: &[&str],
    viewport: egui::Vec2,
    events: Vec<egui::Event>,
) -> (bool, bool, bool, egui::FullOutput) {
    let mut panel_close = false;
    let mut modal_should_close = false;
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
            focused: true,
            events,
            ..Default::default()
        },
        |ctx| {
            let resp = egui::Modal::new(egui::Id::new("test-help-modal")).show(ctx, |ui| {
                ui.set_width((viewport.x - 50.0_f32).clamp(260.0_f32, 880.0_f32));
                panel.show(ui, french, supported_keys)
            });
            panel_close = resp.inner;
            modal_should_close = resp.should_close();
        },
    );
    (
        panel_close || modal_should_close,
        panel_close,
        modal_should_close,
        out,
    )
}

#[test]
fn supported_keys_filtering_omits_unimplemented_shortcuts_and_honors_truthful_native_labels() {
    // Multi-chord support checks:
    assert!(is_shortcut_supported("F7 / Shift+F7", &["F7 / Shift+F7"]));
    assert!(is_shortcut_supported("F7 / Shift+F7", &["F7", "Shift+F7"]));
    assert!(!is_shortcut_supported("F7 / Shift+F7", &["F7"]));
    assert!(!is_shortcut_supported(
        "Ctrl+Shift+B",
        APP_INITIAL_SUPPORTED_KEYS
    ));

    // Truthful native descriptions override `Ctrl+Shift+T` and `Ctrl+Shift+P` without touching `shortcuts.rs`:
    assert_eq!(
        native_shortcut_text(
            "Ctrl+Shift+T",
            "New pane: the default agent, or the default shell",
            false
        ),
        "New pane: default shell"
    );
    assert_eq!(
        native_shortcut_text(
            "Ctrl+Shift+T",
            "Nouveau panneau : l'agent par défaut, ou le shell par défaut",
            true
        ),
        "Nouveau panneau : shell par défaut"
    );
    assert_eq!(
        native_shortcut_text(
            "Ctrl+Shift+P",
            "Command palette: find panes, workspaces, commands",
            false
        ),
        "Command palette: find commands"
    );
    assert_eq!(
        native_shortcut_text(
            "Ctrl+Shift+P",
            "Palette de commandes : trouver panneaux, espaces de travail, commandes",
            true
        ),
        "Palette de commandes : chercher une commande"
    );
    assert_eq!(native_shortcut_text("F1", "This help", false), "This help");

    let en_groups = filter_supported_shortcuts(false, APP_INITIAL_SUPPORTED_KEYS, "");
    let titles: Vec<&str> = en_groups.iter().map(|g| g.title).collect();
    // `Terminal` and `Workspaces` have 0 entries in `APP_INITIAL_SUPPORTED_KEYS` and MUST be omitted:
    assert_eq!(titles, vec!["Panes", "Editor", "App"]);

    let all_keys: Vec<&str> = en_groups
        .iter()
        .flat_map(|g| g.rows.iter().map(|r| r.keys))
        .collect();
    assert_eq!(all_keys.len(), APP_INITIAL_SUPPORTED_KEYS.len());
    assert!(!all_keys.contains(&"Ctrl+Shift+Space"));
    assert!(!all_keys.contains(&"Ctrl+`"));
    assert!(!all_keys.contains(&"Ctrl+W"));
    assert!(!all_keys.contains(&"Ctrl+Shift+B"));
    assert!(!all_keys.contains(&"Win+H"));

    // Searching `"agent"` or `"workspaces"` MUST NOT match overridden `Ctrl+Shift+T` or `Ctrl+Shift+P`:
    assert!(filter_supported_shortcuts(false, APP_INITIAL_SUPPORTED_KEYS, "agent").is_empty());
    assert!(filter_supported_shortcuts(true, APP_INITIAL_SUPPORTED_KEYS, "agent").is_empty());
    assert!(filter_supported_shortcuts(false, APP_INITIAL_SUPPORTED_KEYS, "workspaces").is_empty());
    assert!(filter_supported_shortcuts(true, APP_INITIAL_SUPPORTED_KEYS, "espaces").is_empty());

    // Searching `"shell"` / `"commands"` DOES match the truthful native labels:
    let shell_hits = filter_supported_shortcuts(false, APP_INITIAL_SUPPORTED_KEYS, "shell");
    assert_eq!(shell_hits.len(), 1);
    assert_eq!(shell_hits[0].rows[0].keys, "Ctrl+Shift+T");
}

#[test]
fn accent_insensitive_french_and_english_search_and_multi_chord_keycaps_render_cleanly() {
    // French accent-insensitive queries over group titles, descriptions, and localized keycaps:
    for (query, expected_key) in [
        ("editeur", "Ctrl+S"),
        ("echap", "Esc"),
        ("maj+t", "Ctrl+Shift+T"),
        ("fleches", "Alt+Arrows"),
        ("controle git", "Ctrl+Shift+G"),
        ("parametres", "Ctrl+,"),
        ("taches", "Ctrl+Shift+K"),
    ] {
        let groups = filter_supported_shortcuts(true, APP_INITIAL_SUPPORTED_KEYS, query);
        let found: Vec<&str> = groups
            .iter()
            .flat_map(|g| g.rows.iter().map(|r| r.keys))
            .collect();
        assert!(
            found.contains(&expected_key),
            "query {query:?} expected {expected_key}, got {found:?}"
        );
    }

    // Verify painted keycaps and localized `"or"` / `"ou"` separator for multi-chord rows (`F7 / Shift+F7`):
    // Note: first frame of a newly opened `egui::Modal` (`Area` anchored `CENTER_CENTER`) is `egui`'s sizing pass.
    let ctx = egui::Context::default();
    let mut panel = HelpPanel::new();
    let _ = run_modal_frame(
        &ctx,
        &mut panel,
        true,
        &["F7 / Shift+F7", "Alt+Arrows", "Esc"],
        egui::vec2(820.0_f32, 600.0_f32),
        vec![],
    );
    let (_, _, _, out_fr) = run_modal_frame(
        &ctx,
        &mut panel,
        true,
        &["F7 / Shift+F7", "Alt+Arrows", "Esc"],
        egui::vec2(820.0_f32, 600.0_f32),
        vec![],
    );
    assert!(has_painted_text(&out_fr.shapes, "ÉDITEUR"));
    assert!(has_painted_text(&out_fr.shapes, "PANNEAUX"));
    assert!(has_painted_text(&out_fr.shapes, "F7"));
    assert!(has_painted_text(&out_fr.shapes, "ou"));
    assert!(has_painted_text(&out_fr.shapes, "Maj"));
    assert!(has_painted_text(&out_fr.shapes, "+"));
    assert!(has_painted_text(&out_fr.shapes, "Flèches"));
    assert!(has_painted_text(&out_fr.shapes, "Échap"));

    // Verify narrow 1-column fallback and empty-state EN/FR messages:
    panel.set_query("introuvable-xyz");
    assert_eq!(panel.query(), "introuvable-xyz");
    let (_, _, _, out_empty_fr) = run_modal_frame(
        &ctx,
        &mut panel,
        true,
        APP_INITIAL_SUPPORTED_KEYS,
        egui::vec2(360.0_f32, 520.0_f32),
        vec![],
    );
    assert!(has_painted_text(
        &out_empty_fr.shapes,
        "Aucun raccourci ne correspond."
    ));

    let (_, _, _, out_empty_en) = run_modal_frame(
        &ctx,
        &mut panel,
        false,
        &[],
        egui::vec2(360.0_f32, 520.0_f32),
        vec![],
    );
    assert!(has_painted_text(
        &out_empty_en.shapes,
        "No shortcut matches."
    ));
}

#[test]
fn real_egui_modal_focuses_search_and_two_step_escape_clears_query_without_closing_modal() {
    let ctx = egui::Context::default();
    let mut panel = HelpPanel::default();
    panel.reset();

    // Frame 1 (sizing pass) + Frame 2 (paint pass): open modal -> search input automatically acquires focus, 2-column groups render.
    let (closed_0, _, _, _) = run_modal_frame(
        &ctx,
        &mut panel,
        false,
        APP_INITIAL_SUPPORTED_KEYS,
        egui::vec2(840.0_f32, 620.0_f32),
        vec![],
    );
    assert!(!closed_0);
    let (closed_1, _, _, out_1) = run_modal_frame(
        &ctx,
        &mut panel,
        false,
        APP_INITIAL_SUPPORTED_KEYS,
        egui::vec2(840.0_f32, 620.0_f32),
        vec![],
    );
    assert!(!closed_1);
    assert!(ctx.memory(|m| m.focused().is_some()));
    assert!(has_painted_text(&out_1.shapes, "PANES"));
    assert!(has_painted_text(&out_1.shapes, "EDITOR"));
    assert!(has_painted_text(&out_1.shapes, "APP"));

    // Frame 2: type `"split"` directly (proving search `TextEdit` has focus without mouse click!).
    let (closed_2, _, _, out_2) = run_modal_frame(
        &ctx,
        &mut panel,
        false,
        APP_INITIAL_SUPPORTED_KEYS,
        egui::vec2(840.0_f32, 620.0_f32),
        vec![egui::Event::Text("split".into())],
    );
    assert!(!closed_2);
    assert_eq!(panel.query(), "split");
    let (_, _, _, out_2b) = run_modal_frame(
        &ctx,
        &mut panel,
        false,
        APP_INITIAL_SUPPORTED_KEYS,
        egui::vec2(840.0_f32, 620.0_f32),
        vec![],
    );
    assert!(
        has_painted_text(&out_2.shapes, "Split right")
            || has_painted_text(&out_2b.shapes, "Split right")
    );
    assert!(!has_painted_text(&out_2b.shapes, "Settings"));

    // Frame 3: first `Escape` while `query == "split"` MUST clear `query` AND MUST NOT close `egui::Modal`!
    let (closed_3, panel_close_3, modal_should_close_3, _) = run_modal_frame(
        &ctx,
        &mut panel,
        false,
        APP_INITIAL_SUPPORTED_KEYS,
        egui::vec2(840.0_f32, 620.0_f32),
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(panel.query(), "");
    assert!(!panel_close_3);
    assert!(
        !modal_should_close_3,
        "ModalResponse::should_close must remain false when Escape cleared a non-empty search"
    );
    assert!(!closed_3);
    assert!(
        ctx.memory(|m| m.focused().is_some()),
        "search input must retain focus after Escape clears query"
    );

    // Frame 4: a held key-repeat `Escape` (`repeat: true`) right after clearing MUST NOT close the modal!
    let (closed_repeat, _, modal_repeat_close, _) = run_modal_frame(
        &ctx,
        &mut panel,
        false,
        APP_INITIAL_SUPPORTED_KEYS,
        egui::vec2(840.0_f32, 620.0_f32),
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(!closed_repeat);
    assert!(!modal_repeat_close);

    // Frame 5: typing works immediately because focus stayed in the search input; test the `"×"` clear button too!
    let _ = run_modal_frame(
        &ctx,
        &mut panel,
        false,
        APP_INITIAL_SUPPORTED_KEYS,
        egui::vec2(840.0_f32, 620.0_f32),
        vec![egui::Event::Text("save".into())],
    );
    assert_eq!(panel.query(), "save");
    let (_, _, _, out_with_clear) = run_modal_frame(
        &ctx,
        &mut panel,
        false,
        APP_INITIAL_SUPPORTED_KEYS,
        egui::vec2(840.0_f32, 620.0_f32),
        vec![],
    );
    let clear_pos =
        find_painted_text_center(&out_with_clear.shapes, "×").expect("clear × button rendered");
    let (closed_clear_click, _, _, _) = run_modal_frame(
        &ctx,
        &mut panel,
        false,
        APP_INITIAL_SUPPORTED_KEYS,
        egui::vec2(840.0_f32, 620.0_f32),
        vec![
            egui::Event::PointerMoved(clear_pos),
            egui::Event::PointerButton {
                pos: clear_pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: clear_pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    assert!(!closed_clear_click);
    assert_eq!(panel.query(), "");

    // Frame 6: pressing fresh `Escape` while `query` is empty requests modal closure (`true`)!
    let (closed_final, panel_close_final, _, _) = run_modal_frame(
        &ctx,
        &mut panel,
        false,
        APP_INITIAL_SUPPORTED_KEYS,
        egui::vec2(840.0_f32, 620.0_f32),
        vec![
            egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    assert!(panel_close_final);
    assert!(closed_final);
}
