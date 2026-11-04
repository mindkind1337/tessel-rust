#[allow(dead_code)]
#[path = "../src/agent_catalog.rs"]
mod agent_catalog;
#[allow(dead_code)]
#[path = "../src/agent_installs.rs"]
mod agent_installs;
#[allow(dead_code)]
#[path = "../src/finder_model.rs"]
mod finder_model;
#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/launcher_panel.rs"]
mod launcher_panel;
#[allow(dead_code)]
#[path = "../src/syntax.rs"]
mod syntax;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Color32, Event, FontId, PointerButton, Pos2, Rect, Shape, Vec2};
use finder_model::{LaunchMenuAction, LaunchMenuItem, LauncherLang};
use launcher_panel::{LauncherEntry, LauncherInput, LauncherPanel};
use syntax::{
    SYNTAX_MAX_BYTES, SyntaxCache, SyntaxLang, SyntaxPalette, build_layout_job_with_mode,
    changed_line_flags, editor_gutter_width, paint_editor_gutter,
};
use theme_palettes::{THEMES, ThemeId};

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

#[test]
fn syntax_highlighting_covers_all_languages_and_themes_plus_light_mode() {
    let cases = [
        (
            "src/lib.rs",
            SyntaxLang::Rust,
            "// commentaire accentué\n#[inline]\npub fn calc(n: u32) -> String {\n    format!(\"v={}\", n + 1)\n}\n",
        ),
        (
            "ui/App.tsx",
            SyntaxLang::TypeScript,
            "// header\nexport async function run(id: string): Promise<number> {\n  const msg = `hi ${id}`;\n  return 42;\n}\n",
        ),
        (
            "script.py",
            SyntaxLang::Python,
            "# note\n@dataclass\ndef compute(items: list[str]) -> int:\n    return len(items) + 10\n",
        ),
        (
            "deploy.ps1",
            SyntaxLang::Shell,
            "# deploy\nif ($LASTEXITCODE -ne 0) {\n  Write-Host \"failed\" 1\n}\n",
        ),
        (
            "main.go",
            SyntaxLang::CLike,
            "/* block */\nfunc main() {\n    var count int = 7\n    println(\"go\")\n}\n",
        ),
        (
            "package.json",
            SyntaxLang::Json,
            "{\n  \"name\": \"tessel\",\n  \"version\": 1,\n  \"private\": true\n}\n",
        ),
        (
            "Cargo.toml",
            SyntaxLang::Toml,
            "# cargo\n[package]\nname = \"tessel-rust\"\nedition = \"2024\"\n",
        ),
        (
            "ci.yml",
            SyntaxLang::Yaml,
            "# workflow\njobs:\n  build:\n    runs-on: \"ubuntu-latest\"\n    timeout: 30\n",
        ),
        (
            "View.vue",
            SyntaxLang::Markup,
            "<!-- tpl -->\n<template>\n  <div class=\"card\">Bonjour</div>\n</template>\n",
        ),
        (
            "style.css",
            SyntaxLang::Css,
            "/* theme */\n.panel {\n  padding: 12px;\n  content: \"ok\";\n}\n",
        ),
        (
            "query.sql",
            SyntaxLang::Sql,
            "-- fetch\nSELECT id, name FROM projects WHERE active = true LIMIT 10;\n",
        ),
        (
            "README.md",
            SyntaxLang::Markdown,
            "# Title\n\nUse `cargo test` and see [docs](https://example.com).\n",
        ),
    ];

    for (path, expected_lang, sample) in cases {
        assert_eq!(SyntaxLang::from_path(&PathBuf::from(path)), expected_lang);
        for &theme in &THEMES {
            for dark in [true, false] {
                let job = build_layout_job_with_mode(
                    sample,
                    expected_lang,
                    theme,
                    dark,
                    FontId::monospace(13.0),
                    f32::INFINITY,
                    false,
                );
                assert_eq!(job.text, sample);
                let mut cursor = 0usize;
                let mut distinct_colors = std::collections::HashSet::new();
                for sec in &job.sections {
                    assert_eq!(sec.byte_range.start, cursor);
                    assert!(sec.byte_range.end > sec.byte_range.start);
                    assert!(sample.is_char_boundary(sec.byte_range.start));
                    assert!(sample.is_char_boundary(sec.byte_range.end));
                    distinct_colors.insert(sec.format.color);
                    cursor = sec.byte_range.end;
                }
                assert_eq!(cursor, sample.len());
                assert!(
                    distinct_colors.len() >= 2,
                    "expected syntax colors for {path} under {theme:?} dark={dark}"
                );
            }
        }
    }

    let light = SyntaxPalette::for_theme_mode(ThemeId::Classic, false);
    let dark = SyntaxPalette::for_theme_mode(ThemeId::Classic, true);
    assert_ne!(light.plain, dark.plain);
    const { assert!(SYNTAX_MAX_BYTES >= 512 * 1024) };
}

#[test]
fn syntax_cache_memoizes_and_gutter_skips_wrapped_continuation_rows() {
    let ctx = egui::Context::default();
    let mut cache = SyntaxCache::default();
    let original = "fn alpha() {}\nfn beta() { let very_long = \"unchanged\"; }\nfn gamma() {}\n";
    let edited = "fn alpha() {}\nfn beta_modified() { let very_long = \"this line wraps across narrow width\"; }\nfn gamma() {}\n";
    let flags = changed_line_flags(original, edited);
    assert_eq!(flags, vec![false, true, false, false]);

    let mut first_galley = None;
    let mut second_galley = None;
    let mut numbered_lines = 0usize;

    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(360.0, 300.0))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let font = FontId::monospace(13.0);
                let g1 = cache.layout(
                    ui,
                    edited,
                    SyntaxLang::Rust,
                    ThemeId::TokyoNight,
                    true,
                    &font,
                    140.0,
                    true,
                    true,
                );
                let g2 = cache.layout(
                    ui,
                    edited,
                    SyntaxLang::Rust,
                    ThemeId::TokyoNight,
                    true,
                    &font,
                    140.0,
                    true,
                    true,
                );
                assert!(
                    g1.rows.len() > 4,
                    "narrow wrap width should split line 2 into wrapped sub-rows"
                );
                let gw = editor_gutter_width(ui, &font, 4);
                assert!(gw >= 32.0);
                numbered_lines = paint_editor_gutter(
                    ui.painter(),
                    0.0,
                    gw,
                    Pos2::new(gw + 6.0, 10.0),
                    &g1,
                    &font,
                    &flags,
                    true,
                    Color32::GRAY,
                    Color32::DARK_GRAY,
                    Color32::LIGHT_BLUE,
                );
                first_galley = Some(g1);
                second_galley = Some(g2);
            });
        },
    );

    assert!(Arc::ptr_eq(
        first_galley.as_ref().unwrap(),
        second_galley.as_ref().unwrap()
    ));
    assert_eq!(
        numbered_lines, 4,
        "wrapped continuation sub-rows must not paint extra line numbers"
    );
    let texts: Vec<String> = painted_texts(&out.shapes)
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    assert!(texts.contains(&"1".to_string()));
    assert!(texts.contains(&"2".to_string()));
    assert!(texts.contains(&"3".to_string()));
}

#[test]
fn launcher_footer_install_agents_button_emits_open_agent_settings_in_en_and_fr() {
    let ctx = egui::Context::default();
    let mut panel = LauncherPanel::default();
    let entries = vec![LauncherEntry {
        item: LaunchMenuItem::Shell {
            id: "default".into(),
            name: "PowerShell".into(),
        },
        available: true,
        reason: None,
    }];

    for (lang, expected_label) in [
        (LauncherLang::En, "Install agents or add your own…"),
        (
            LauncherLang::Fr,
            "Installer des agents ou ajouter le vôtre…",
        ),
    ] {
        let mut action = None;
        let settle = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(480.0, 400.0))),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = panel.show(
                        ui,
                        LauncherInput {
                            entries: &entries,
                            placements: &[],
                            target_title: None,
                            lang,
                        },
                    );
                });
            },
        );
        assert_eq!(action, None);
        let btn_center = painted_texts(&settle.shapes)
            .into_iter()
            .find(|(t, _)| t == expected_label)
            .map(|(_, r)| r.center())
            .unwrap_or_else(|| panic!("missing footer button {expected_label:?}"));

        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(480.0, 400.0))),
                focused: true,
                events: vec![
                    Event::PointerMoved(btn_center),
                    Event::PointerButton {
                        pos: btn_center,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    Event::PointerButton {
                        pos: btn_center,
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = panel.show(
                        ui,
                        LauncherInput {
                            entries: &entries,
                            placements: &[],
                            target_title: None,
                            lang,
                        },
                    );
                });
            },
        );
        assert_eq!(action, Some(LaunchMenuAction::OpenAgentSettings));
    }
}
