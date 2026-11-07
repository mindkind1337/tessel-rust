use super::{
    icons::GpuiIcon,
    theme::{GpuiThemeTokens, SPLIT_DIVIDER_PX, color_mix_srgb},
};
use crate::{
    app::{
        Tessel,
        gpui_bridge::{GpuiAction, GpuiCenterMode},
    },
    theme_palettes::THEMES,
    workspace::{KeepAwakeSetting, RendererPreference, SideTab, resolve_startup_renderer},
};

#[test]
fn gpui_bridge_initializes_and_mutates_shared_workspace_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let proj_dir = temp.path().join("alpha-repo");
    std::fs::create_dir_all(&proj_dir).expect("create project dir");

    let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().to_path_buf()), false, || {});
    let snap0 = app.gpui_snapshot(960.0, 640.0, SPLIT_DIVIDER_PX);
    assert_eq!(snap0.center_mode, GpuiCenterMode::NoProjectsWelcome);
    assert!(snap0.projects.is_empty());
    assert_eq!(snap0.renderer, RendererPreference::AutoGpui);

    // Add a project folder and verify EmptyProjectLauncher mode + initials
    app.apply_gpui_action(GpuiAction::AddProjectPath(proj_dir.clone()));
    let snap1 = app.gpui_snapshot(960.0, 640.0, SPLIT_DIVIDER_PX);
    assert_eq!(snap1.projects.len(), 1);
    assert_eq!(snap1.projects[0].name, "alpha-repo");
    assert_eq!(snap1.projects[0].initials, "AR");
    assert!(snap1.projects[0].active);
    assert_eq!(snap1.center_mode, GpuiCenterMode::EmptyProjectLauncher);

    // Toggle sidebars, right fullscreen, and cycle all 7 SidePanel tabs
    let initial_left = snap1.toolbar.left_open;
    app.apply_gpui_action(GpuiAction::ToggleLeftSidebar);
    assert_eq!(
        app.gpui_snapshot(960.0, 640.0, 4.0).toolbar.left_open,
        !initial_left
    );

    for tab in [
        SideTab::Dashboard,
        SideTab::Files,
        SideTab::Changes,
        SideTab::Tasks,
        SideTab::TaskHistory,
        SideTab::History,
        SideTab::Conversations,
    ] {
        app.apply_gpui_action(GpuiAction::SelectSideTab(tab));
        let snap = app.gpui_snapshot(960.0, 640.0, 4.0);
        assert!(snap.toolbar.right_open);
        assert_eq!(snap.right_tab, tab);
    }

    // Switch themes, language, font zoom, keep-awake, and renderer preference
    for theme in THEMES {
        app.apply_gpui_action(GpuiAction::SetTheme(theme));
        assert_eq!(app.gpui_snapshot(960.0, 640.0, 4.0).theme, theme);
    }

    app.apply_gpui_action(GpuiAction::SetLanguage { french: true });
    assert!(app.gpui_snapshot(960.0, 640.0, 4.0).french);
    app.apply_gpui_action(GpuiAction::SetLanguage { french: false });
    assert!(!app.gpui_snapshot(960.0, 640.0, 4.0).french);

    let f0 = app.gpui_snapshot(960.0, 640.0, 4.0).font_size;
    app.apply_gpui_action(GpuiAction::ZoomFont(1));
    assert!(app.gpui_snapshot(960.0, 640.0, 4.0).font_size > f0);
    app.apply_gpui_action(GpuiAction::ZoomFont(0));
    assert!((app.gpui_snapshot(960.0, 640.0, 4.0).font_size - f0).abs() < f32::EPSILON);

    app.apply_gpui_action(GpuiAction::CycleKeepAwake);
    assert_ne!(
        app.gpui_snapshot(960.0, 640.0, 4.0).status_bar.keep_awake,
        KeepAwakeSetting::Off
    );

    app.apply_gpui_action(GpuiAction::SetRenderer(RendererPreference::CompatEgui));
    assert_eq!(
        app.gpui_snapshot(960.0, 640.0, 4.0).renderer,
        RendererPreference::CompatEgui
    );
}

#[test]
fn gpui_theme_tokens_and_svg_icons_cover_all_palettes_and_glyphs() {
    for theme in THEMES {
        let tokens = GpuiThemeTokens::from_theme(theme);
        assert_eq!(tokens.id, theme);
        assert!(tokens.radius_px >= 4.0);
        assert!((tokens.active_pane_ring.alpha - 0.70).abs() < 1e-5);
    }

    let mixed = color_mix_srgb(gpui::rgb(0xff0000), 0.25, gpui::rgb(0x0000ff));
    assert!((mixed.color.red - 0.25).abs() < 1e-4);
    assert!((mixed.color.blue - 0.75).abs() < 1e-4);

    assert_eq!(GpuiIcon::ALL.len(), 35);
    for icon in GpuiIcon::ALL {
        let svg_text = std::str::from_utf8(icon.svg_bytes()).expect("valid utf8 svg");
        assert!(svg_text.starts_with("<svg "));
        assert!(svg_text.ends_with("</svg>"));
    }

    let args = vec!["tessel".into(), "--renderer=compat".into()];
    assert_eq!(
        resolve_startup_renderer(&args, Some("gpui"), RendererPreference::AutoGpui, false),
        RendererPreference::CompatEgui
    );
}
