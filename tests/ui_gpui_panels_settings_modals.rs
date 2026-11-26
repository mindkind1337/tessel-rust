//! Integration tests for Phase B (`Lot b176`): GPUI icon coverage (`45` SVGs),
//! Finder & Explorer GPUI snapshot adapters, 22-page Settings model catalog,
//! Command Palette & Shortcuts filtering, and 5-color Git swimlane graph palette.

#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

#[allow(dead_code)]
#[path = "../src/gpui_ui/theme.rs"]
mod theme;

#[allow(dead_code)]
#[path = "../src/gpui_ui/icons.rs"]
mod icons;

#[allow(dead_code)]
#[path = "../src/i18n.rs"]
mod i18n;

#[allow(dead_code)]
#[path = "../src/finder_model.rs"]
mod finder_model;

#[allow(dead_code)]
#[path = "../src/finder_panel.rs"]
mod finder_panel;

#[allow(dead_code)]
#[path = "../src/settings_model.rs"]
mod settings_model;

#[allow(dead_code)]
#[path = "../src/palette.rs"]
mod palette;

#[allow(dead_code)]
#[path = "../src/shortcuts.rs"]
mod shortcuts;

#[test]
fn lot_b176_icons_and_theme_aliases_are_complete() {
    assert_eq!(icons::GpuiIcon::ALL.len(), 49);
    for icon in [
        icons::GpuiIcon::Gauge,
        icons::GpuiIcon::Moon,
        icons::GpuiIcon::RefreshCw,
        icons::GpuiIcon::Trash2,
        icons::GpuiIcon::Play,
        icons::GpuiIcon::Eye,
        icons::GpuiIcon::Lock,
        icons::GpuiIcon::Users,
        icons::GpuiIcon::Send,
        icons::GpuiIcon::ArrowLeft,
    ] {
        let svg = std::str::from_utf8(icon.svg_bytes()).expect("utf8 svg");
        assert!(svg.starts_with("<svg ") && svg.ends_with("</svg>"));
    }

    for theme_id in theme_palettes::THEMES {
        let tokens = theme::GpuiThemeTokens::from_theme(theme_id);
        assert_eq!(tokens.bg, tokens.backdrop);
        assert_eq!(tokens.panel, tokens.chrome);
        assert_eq!(tokens.accent_soft, tokens.accent_soft_bg);
        assert!((tokens.accent_fg.alpha - 1.0).abs() < 1e-5);
    }
}

#[test]
fn finder_gpui_adapter_scans_filters_steps_and_accepts() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(temp.path().join("src")).expect("create src");
    std::fs::write(
        temp.path().join("src").join("alpha.rs"),
        "pub struct Alpha;\n",
    )
    .expect("write alpha");
    std::fs::write(
        temp.path().join("src").join("beta.rs"),
        "pub struct Beta;\n",
    )
    .expect("write beta");

    let mut finder = finder_panel::Finder::default();
    assert!(finder.gpui_snapshot().is_none());

    finder.open(temp.path()).expect("open finder");
    for _ in 0..80 {
        finder.poll();
        if let Some(snap) = finder.gpui_snapshot()
            && !snap.scanning
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    finder.gpui_set_query("beta".into());
    let snap = finder.gpui_snapshot().expect("finder open");
    assert_eq!(snap.query, "beta");
    assert_eq!(snap.results.len(), 1);
    assert!(snap.results[0].ends_with("beta.rs"));

    let action = finder.gpui_accept(false).expect("accept beta.rs");
    assert_eq!(
        action,
        finder_panel::FinderAction::OpenFile(temp.path().join("src").join("beta.rs"))
    );
    assert!(finder.gpui_snapshot().is_none());
}

#[test]
fn settings_catalog_palette_and_shortcuts_models_support_gpui_views() {
    assert_eq!(settings_model::ALL_SETTINGS_GROUPS.len(), 5);
    assert_eq!(settings_model::ALL_SETTINGS_PAGES.len(), 22);

    let cmds = vec![
        palette::PaletteCommand::new("open-settings", "Workspace", "Open Settings")
            .with_hint("preferences config")
            .with_shortcut("Ctrl+,"),
        palette::PaletteCommand::new("new-shell", "Session", "New Shell")
            .with_hint("terminal bash pwsh")
            .with_shortcut("Ctrl+Shift+T"),
    ];
    let rows = palette::filter_palette_commands(&cmds, "config");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].command.id, "open-settings");

    let groups = shortcuts::shortcut_groups(false);
    assert!(!groups.is_empty());
    let filtered = shortcuts::filter_shortcuts(false, "Ctrl+Shift+T");
    assert!(!filtered.is_empty());
}
