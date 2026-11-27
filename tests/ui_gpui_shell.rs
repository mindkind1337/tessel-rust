//! Integration tests for Phase B (`Lot b174`): dual-renderer CLI/env/workspace resolution,
//! 1:1 GPUI CSS theme tokens across all 7 palettes, and embedded `resvg` vector icon assets.

#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

#[allow(dead_code)]
#[path = "../src/gpui_ui/theme.rs"]
mod theme;

#[allow(dead_code)]
#[path = "../src/gpui_ui/icons.rs"]
mod icons;

#[test]
fn all_seven_themes_map_cleanly_to_gpui_tokens() {
    for id in theme_palettes::THEMES {
        let spec = id.spec();
        let tokens = theme::GpuiThemeTokens::from_theme(id);
        assert_eq!(tokens.id, id);
        assert!((tokens.radius_px - spec.radius_px).abs() < f32::EPSILON);
        assert!((tokens.accent_soft_bg.alpha - 0.16).abs() < 1e-5);
        let active_alpha = if id == theme_palettes::ThemeId::Warp {
            0.80
        } else {
            0.70
        };
        assert!((tokens.active_pane_ring.alpha - active_alpha).abs() < 1e-5);
        assert_eq!(tokens.broadcast_pane_ring, tokens.warn);
    }
}

#[test]
fn layout_constants_match_electron_reference_css_pixels() {
    assert!((theme::TOOLBAR_HEIGHT_PX - 40.0).abs() < f32::EPSILON);
    assert!((theme::BROADCAST_BANNER_HEIGHT_PX - 24.0).abs() < f32::EPSILON);
    assert!((theme::VIEW_TABS_HEIGHT_PX - 30.0).abs() < f32::EPSILON);
    assert!((theme::PANE_NAV_HEIGHT_PX - 30.0).abs() < f32::EPSILON);
    assert!((theme::STATUSBAR_HEIGHT_PX - 24.0).abs() < f32::EPSILON);
    assert!((theme::SIDEBAR_RAIL_WIDTH_PX - 52.0).abs() < f32::EPSILON);
    assert!((theme::SPLIT_DIVIDER_PX - 4.0).abs() < f32::EPSILON);
}

#[test]
fn all_embedded_gpui_icons_are_valid_self_contained_svgs() {
    assert_eq!(icons::GpuiIcon::ALL.len(), 52);
    for &icon in icons::GpuiIcon::ALL {
        let raw = std::str::from_utf8(icon.svg_bytes()).expect("icon SVG must be valid UTF-8");
        assert!(
            raw.contains(r#"xmlns="http://www.w3.org/2000/svg""#),
            "missing xmlns on {icon:?}"
        );
        assert!(raw.contains("viewBox="), "missing viewBox on {icon:?}");
        assert!(raw.ends_with("</svg>"), "unclosed <svg> on {icon:?}");
    }
}
