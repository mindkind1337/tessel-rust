use super::{
    editor_view::{layout_mermaid_flowchart, layout_mermaid_sequence},
    icons::GpuiIcon,
    theme::{GpuiThemeTokens, SPLIT_DIVIDER_PX, color_mix_srgb},
};
use crate::{
    app::{
        Tessel,
        gpui_bridge::{GpuiAction, GpuiCenterMode, GpuiFocusTarget},
    },
    rich_previews::{MermaidDiagram, parse_mermaid_diagram},
    syntax::SyntaxLang,
    terminal::{BroadcastInput, encode_gpui_terminal_key, extract_vt_styled_lines},
    theme_palettes::{THEMES, ThemeId},
    workspace::{KeepAwakeSetting, RendererPreference, SideTab, resolve_startup_renderer},
};
use serde_json::json;

#[cfg(all(windows, feature = "visual-test"))]
#[gpui::test]
fn view_tab_layout_keeps_long_unicode_titles_and_controls_separate(
    test: &mut gpui::TestAppContext,
) {
    use crate::files::Document;
    let platform = gpui_platform::current_platform(true);
    let cx = &mut gpui::TestAppContext::build_with_text_system(
        test.dispatcher.clone(),
        None,
        platform.text_system(),
    );
    let dir = tempfile::tempdir().unwrap();
    let name = format!("É{}.rs", "long".repeat(12));
    let mut docs = Vec::new();
    for (folder, filename) in [("alpha", name.clone()), ("beta", name.to_lowercase())] {
        let parent = dir.path().join(folder);
        std::fs::create_dir(&parent).unwrap();
        let path = parent.join(filename);
        std::fs::write(&path, "original").unwrap();
        docs.push(Document::open(dir.path(), &path).unwrap());
    }
    docs[0].text.push('!');
    let path = docs[0].path.clone();
    docs.push(Document::from_diff(
        dir.path(),
        path,
        format!("{name} (diff)"),
        "+line".into(),
        false,
    ));
    let view = cx.add_window(|_, cx| {
        let mut root = super::TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
        root.app.install_editor_input_fixture(docs, 0);
        root
    });
    cx.refresh().unwrap();
    let snapshot = view
        .update(cx, |root, _, _| root.app.gpui_snapshot(960., 640., 4.))
        .unwrap();
    assert_eq!(snapshot.view_tabs[1].title, format!("{name} · alpha"));
    assert_eq!(
        snapshot.view_tabs[2].title,
        format!("{} · beta", name.to_lowercase())
    );
    assert_eq!(snapshot.view_tabs[3].title, format!("{name} (diff)"));
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    let tab = visual.debug_bounds("view-tab-1").unwrap();
    let label = visual.debug_bounds("view-tab-label-1").unwrap();
    let dot = visual.debug_bounds("view-tab-dirty-1").unwrap();
    let close = visual.debug_bounds("view-tab-close-1").unwrap();
    assert!(tab.size.width <= gpui::px(220.) && label.size.width > gpui::px(0.));
    assert_eq!(dot.size.width, gpui::px(7.));
    assert_eq!(close.size.width, gpui::px(18.));
    assert!(
        label.right() <= dot.left() && dot.right() <= close.left() && close.right() <= tab.right()
    );
    let other = visual.debug_bounds("view-tab-2").unwrap();
    visual.simulate_mouse_move(other.center(), None, Default::default());
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    let other_close = visual.debug_bounds("view-tab-close-2").unwrap();
    visual.simulate_click(other_close.center(), Default::default());
    let after = view
        .update(cx, |root, _, _| root.app.gpui_snapshot(960., 640., 4.))
        .unwrap();
    assert_eq!(after.view_tabs.len(), 3);
    assert_eq!(after.view_tabs[1].title, name);
    assert!(after.view_tabs[1].selected);
}

#[cfg(all(windows, feature = "visual-test"))]
#[gpui::test]
fn view_tab_mouse_gestures_keep_preview_and_confirm_dirty_close(cx: &mut gpui::TestAppContext) {
    use crate::files::Document;
    use gpui::MouseButton::{Left, Middle, Right};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preview.txt");
    std::fs::write(&path, "original").unwrap();
    let mut doc = Document::open(dir.path(), &path).unwrap();
    doc.unpinned_preview = true;
    let view = cx.add_window(|_, cx| {
        let mut root = super::TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
        root.app.install_editor_input_fixture(vec![doc], 0);
        root
    });
    let snapshot = |cx: &mut gpui::TestAppContext| {
        view.update(cx, |root, _, _| root.app.gpui_snapshot(960., 640., 4.))
            .unwrap()
    };
    let click = |cx: &mut gpui::TestAppContext, selector: &'static str, button, click_count| {
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        let position = visual.debug_bounds(selector).unwrap().center();
        visual.simulate_event(gpui::MouseDownEvent {
            position,
            button,
            click_count,
            ..Default::default()
        });
        visual.simulate_event(gpui::MouseUpEvent {
            position,
            button,
            click_count,
            ..Default::default()
        });
    };
    click(cx, "view-tab-1", Left, 1);
    assert!(snapshot(cx).view_tabs[1].preview);
    click(cx, "view-tab-1", Left, 2);
    assert!(!snapshot(cx).view_tabs[1].preview);
    click(cx, "view-tab-0", Middle, 1);
    click(cx, "view-tab-1", Right, 1);
    assert_eq!(snapshot(cx).view_tabs.len(), 2);
    view.update(cx, |root, _, _| {
        let mut doc = Document::open(dir.path(), &path).unwrap();
        doc.text.push('!');
        root.app.install_editor_input_fixture(vec![doc], 0);
    })
    .unwrap();
    click(cx, "view-tab-1", Middle, 1);
    let result = snapshot(cx);
    assert_eq!(result.view_tabs.len(), 2);
    assert!(result.modals.confirm_modal.is_some());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
}

#[cfg(all(windows, feature = "visual-test"))]
#[gpui::test]
fn settings_search_edits_matching_rows_across_pages(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let view = cx.add_window(|_, cx| {
        let mut root = super::TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx);
        root.app
            .apply_gpui_action(GpuiAction::SetLanguage { french: false });
        root.app
            .apply_gpui_action(GpuiAction::OpenSettingsPage("general".into()));
        root.app
            .apply_gpui_action(GpuiAction::SetSettingsSearch("WRAP word".into()));
        root
    });
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    assert!(visual.debug_bounds("gpui-st-ed-dwrap").is_some());
    assert!(visual.debug_bounds("gpui-st-ed-syn").is_none());
    let point = visual.debug_bounds("gpui-st-ed-wrap").unwrap().center();
    let before = view
        .update(cx, |root, _, _| {
            root.app
                .gpui_snapshot(960., 640., 4.)
                .settings
                .unwrap()
                .editor_prefs
                .word_wrap
        })
        .unwrap();
    gpui::VisualTestContext::from_window(view.into(), cx).simulate_click(point, Default::default());
    view.update(cx, |root, _, _| {
        let settings = root.app.gpui_snapshot(960., 640., 4.).settings.unwrap();
        assert_ne!(before, settings.editor_prefs.word_wrap);
        assert_eq!(settings.active_page_id, "general");
    })
    .unwrap();
    cx.refresh().unwrap();
    cx.simulate_keystrokes(view.into(), "space");
    view.update(cx, |root, _, cx| {
        let settings = root.app.gpui_snapshot(960., 640., 4.).settings.unwrap();
        assert_eq!(before, settings.editor_prefs.word_wrap);
        root.dispatch_action(GpuiAction::SetLanguage { french: true }, cx);
        root.dispatch_action(GpuiAction::SetSettingsSearch("cliquer SURVOLER".into()), cx);
    })
    .unwrap();
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    assert!(visual.debug_bounds("gpui-st-tp-focus").is_some());
    assert!(visual.debug_bounds("gpui-st-ed-wrap").is_none());
}

#[cfg(all(windows, feature = "visual-test"))]
#[gpui::test]
fn settings_workspace_resizes_below_toolbar_and_returns_to_app(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let view = cx.add_window(|_, cx| {
        let mut root = super::TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx);
        root.app
            .apply_gpui_action(GpuiAction::OpenSettingsPage("appearance".into()));
        root
    });
    for (w, h, rail) in [(740., 520., 200.), (1000., 750., 240.), (1440., 900., 290.)] {
        cx.simulate_window_resize(view.into(), gpui::size(gpui::px(w), gpui::px(h)));
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        let page = visual.debug_bounds("settings-page").unwrap();
        assert_eq!(page.origin, gpui::point(gpui::px(0.), gpui::px(40.)));
        assert_eq!(page.size, gpui::size(gpui::px(w), gpui::px(h - 40.)));
        assert_eq!(
            visual.debug_bounds("settings-nav").unwrap().size.width,
            gpui::px(rail)
        );
    }
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    let back = visual.debug_bounds("settings-back").unwrap().center();
    visual.simulate_click(back, Default::default());
    cx.refresh().unwrap();
    assert!(
        gpui::VisualTestContext::from_window(view.into(), cx)
            .debug_bounds("settings-page")
            .is_none()
    );
}

#[cfg(all(windows, feature = "visual-test"))]
#[gpui::test]
fn custom_agent_color_arrows_wrap_and_move_focus(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let view = cx.add_window(|_, cx| {
        let mut root = super::TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx);
        root.app
            .apply_gpui_action(GpuiAction::OpenSettingsPage("agents".into()));
        root
    });
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    let position = visual.debug_bounds("settings-body").unwrap().center();
    visual.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-10000.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    let first = visual.debug_bounds("agent-color-0").unwrap().center();
    visual.simulate_click(first, Default::default());
    view.update(cx, |root, w, _| {
        assert!(root.settings_controls.swatches[0].is_focused(w))
    })
    .unwrap();
    for (key, index) in [("left", 7), ("down", 0), ("right", 1), ("up", 0)] {
        gpui::VisualTestContext::from_window(view.into(), cx).simulate_keystrokes(key);
        view.update(cx, |root, w, cx| {
            assert!(root.settings_controls.swatches[index].is_focused(w));
            assert_eq!(
                root.app
                    .gpui_snapshot(960., 640., 4.)
                    .settings
                    .unwrap()
                    .new_custom_agent_accent,
                index
            );
            cx.notify();
        })
        .unwrap();
        cx.refresh().unwrap();
    }
}

#[cfg(all(windows, feature = "visual-test"))]
#[gpui::test]
fn html_folder_trust_controls_cover_decline_reconsider_grant_and_revoke(
    cx: &mut gpui::TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let path = project.join("index.html");
    std::fs::write(&path, "<p>index</p>").unwrap();
    let view = cx.add_window(|_, cx| {
        let mut root = super::TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx);
        root.app
            .apply_gpui_action(GpuiAction::AddProjectPath(project));
        root.app
            .apply_gpui_action(GpuiAction::OpenDocumentPath(path));
        root
    });
    for (selector, expected) in [
        ("restrict-html-folder", Some(false)),
        ("ask-html-trust", None),
        ("trust-html-folder", Some(true)),
        ("restrict-html-folder", Some(false)),
    ] {
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        let point = visual.debug_bounds(selector).unwrap().center();
        visual.simulate_click(point, Default::default());
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app
                    .gpui_snapshot(960., 640., 4.)
                    .html_toolbar
                    .unwrap()
                    .decision,
                expected
            )
        })
        .unwrap();
    }
}

#[cfg(all(windows, feature = "visual-test"))]
#[gpui::test]
fn html_external_prompt_dismisses_or_opens_in_the_owner_browser(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let path = project.join("index.html");
    std::fs::write(&path, "<p>Index</p>").unwrap();
    let view = cx.add_window(|_, cx| {
        let mut root = super::TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx);
        root.app
            .apply_gpui_action(GpuiAction::AddProjectPath(project));
        root.app
            .apply_gpui_action(GpuiAction::OpenDocumentPath(path));
        root
    });
    for (selector, open) in [("dismiss-held-link", false), ("open-held-link", true)] {
        view.update(cx, |root, _, cx| {
            let id = root
                .app
                .gpui_snapshot(960., 640., 4.)
                .active_document
                .unwrap()
                .instance_id;
            root.app.editor_page.queue_external_fixture(
                id,
                url::Url::parse("https://example.invalid/report").unwrap(),
            );
            assert!(root.app.poll_html_page());
            cx.notify();
        })
        .unwrap();
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        let button = visual.debug_bounds(selector).unwrap().center();
        visual.simulate_click(button, Default::default());
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            let snapshot = root.app.gpui_snapshot(960., 640., 4.);
            assert!(snapshot.held_html_link.is_none());
            assert_eq!(
                snapshot.center_mode,
                if open {
                    super::GpuiCenterMode::BrowserTab
                } else {
                    super::GpuiCenterMode::DocumentEditor
                }
            );
        })
        .unwrap();
    }
}

#[cfg(feature = "visual-test")]
#[gpui::test]
fn html_plain_mode_displays_selectable_body_and_switches_back_to_source(
    cx: &mut gpui::TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let path = project.join("index.HTML");
    std::fs::write(
        &path,
        "<h1>Title</h1><p>First &amp; second</p><p>Last<br>line</p><script>hidden()</script>",
    )
    .unwrap();
    let view = cx.add_window(|_, cx| {
        let mut root = super::TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx);
        root.app
            .apply_gpui_action(GpuiAction::AddProjectPath(project));
        root.app
            .apply_gpui_action(GpuiAction::OpenDocumentPath(path));
        root
    });
    for selector in ["html-plain-mode", "html-source-mode", "html-preview-mode"] {
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        let pos = visual.debug_bounds(selector).unwrap().center();
        visual.simulate_click(pos, Default::default());
        cx.refresh().unwrap();
        #[cfg(windows)]
        {
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            let reload = visual.debug_bounds("html-reload");
            assert_eq!(reload.is_some(), selector == "html-preview-mode");
            if let Some(bounds) = reload {
                visual.simulate_click(bounds.center(), Default::default());
                cx.refresh().unwrap();
            }
        }
        view.update(cx, |root, _, cx| {
            let doc = root
                .app
                .gpui_snapshot(960., 640., 4.)
                .active_document
                .unwrap();
            assert_eq!(doc.preview_mode, selector != "html-source-mode");
            assert_eq!(
                root.editor_input.plain().map(|s| s.read(cx).as_str()),
                (selector == "html-plain-mode").then_some("Title\nFirst & second\nLast\nline")
            );
            assert!(doc.text.contains("<script>hidden()</script>"));
        })
        .unwrap();
    }
}

#[cfg(feature = "visual-test")]
#[gpui::test]
fn html_link_context_menu_opens_local_target_and_keeps_index(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let index = project.join("index.html");
    let links = r#"<p><a href="report.pdf">Report</a></p><p><a href="https://example.invalid/">Site</a></p>"#;
    // Exercise the upper end of the editor's HTML size range with links at EOF.
    let padding = " ".repeat(crate::files::MAX_TEXT - links.len() - "<!---->".len());
    std::fs::write(&index, format!("<!--{padding}-->{links}")).unwrap();
    let target = project.join("report.pdf");
    std::fs::write(&target, crate::pdf::fixture()).unwrap();
    let view = cx.add_window(|_, cx| {
        let mut root = super::TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx);
        root.app
            .apply_gpui_action(GpuiAction::AddProjectPath(project));
        root.app
            .apply_gpui_action(GpuiAction::OpenDocumentPath(index));
        root
    });
    for (selector, web) in [("html-link-1", true), ("html-link-0", false)] {
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        let pos = visual.debug_bounds(selector).unwrap().center();
        visual.simulate_mouse_down(pos, gpui::MouseButton::Right, Default::default());
        visual.simulate_mouse_up(pos, gpui::MouseButton::Right, Default::default());
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        assert!(visual.debug_bounds("html-link-menu").is_some());
        view.update(cx, |root, _, _| {
            let doc = root
                .app
                .gpui_snapshot(960., 640., 4.)
                .active_document
                .unwrap();
            assert!(doc.title.ends_with("index.html"));
            assert_eq!(doc.html_link_menu.unwrap().1, web);
        })
        .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        visual.simulate_keystrokes(if web { "escape" } else { "down enter" });
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while view
        .update(cx, |root, _, _| {
            root.app.poll_for_gpui();
            root.app.gpui_document_ids().len() == 1
        })
        .unwrap()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "linked PDF did not open"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    cx.refresh().unwrap();
    view.update(cx, |root, _, _| {
        let report = root
            .app
            .gpui_snapshot(960., 640., 4.)
            .active_document
            .unwrap();
        assert!(report.title.ends_with("report.pdf") && report.pdf_summary.is_some());
        assert_eq!(root.app.gpui_document_ids().len(), 2);
        root.app.apply_gpui_action(GpuiAction::SelectDocumentTab(0));
        let index = root
            .app
            .gpui_snapshot(960., 640., 4.)
            .active_document
            .unwrap();
        assert!(index.title.ends_with("index.html") && index.preview_mode);
    })
    .unwrap();
}

#[cfg(feature = "visual-test")]
#[gpui::test]
fn local_image_menu_opens_on_the_displayed_pixels(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let path = project.join("test.avif");
    std::fs::write(
        &path,
        include_bytes!("../../tests/fixtures/alpha-bars.avif"),
    )
    .unwrap();
    let view = cx.add_window(|_, cx| {
        let mut root = super::TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx);
        root.app
            .apply_gpui_action(GpuiAction::AddProjectPath(project));
        root.app
            .apply_gpui_action(GpuiAction::OpenDocumentPath(path));
        root
    });
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    let pos = visual.debug_bounds("local-preview-image").unwrap().center();
    visual.simulate_mouse_down(pos, gpui::MouseButton::Right, Default::default());
    visual.simulate_mouse_up(pos, gpui::MouseButton::Right, Default::default());
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    assert!(visual.debug_bounds("local-image-menu").is_some());
    visual.simulate_keystrokes("down");
    view.update(cx, |root, window, _| {
        assert!(root.image_menu_item.is_focused(window))
    })
    .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    visual.simulate_keystrokes("escape");
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    assert!(visual.debug_bounds("local-image-menu").is_none());
    visual.simulate_mouse_down(pos, gpui::MouseButton::Right, Default::default());
    visual.simulate_mouse_up(pos, gpui::MouseButton::Right, Default::default());
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    assert!(visual.debug_bounds("local-image-menu").is_some());
    let saved = dir.path().join("original-copy");
    crate::files::TEST_DOWNLOAD_DESTINATION.with(|next| next.set(Some(saved.clone())));
    visual.simulate_keystrokes("down");
    visual.simulate_keystrokes("enter");
    cx.refresh().unwrap();
    assert_eq!(
        std::fs::read(saved).unwrap(),
        include_bytes!("../../tests/fixtures/alpha-bars.avif")
    );
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    assert!(visual.debug_bounds("local-image-menu").is_none());
    visual.simulate_mouse_down(pos, gpui::MouseButton::Right, Default::default());
    visual.simulate_mouse_up(pos, gpui::MouseButton::Right, Default::default());
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    assert!(visual.debug_bounds("local-image-menu").is_some());
    visual.simulate_click(gpui::point(gpui::px(5.), gpui::px(5.)), Default::default());
    cx.refresh().unwrap();
    assert!(
        gpui::VisualTestContext::from_window(view.into(), cx)
            .debug_bounds("local-image-menu")
            .is_none()
    );
}

#[cfg(feature = "visual-test")]
#[gpui::test]
fn tall_image_fits_the_local_viewport_and_the_margin_has_no_download_menu(
    cx: &mut gpui::TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let path = project.join("portrait.png");
    image::RgbaImage::new(100, 2000).save(&path).unwrap();
    let view = cx.add_window(|_, cx| {
        let mut root = super::TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx);
        root.app
            .apply_gpui_action(GpuiAction::AddProjectPath(project));
        root.app
            .apply_gpui_action(GpuiAction::OpenDocumentPath(path));
        root
    });
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    let pane = visual.debug_bounds("local-image-viewport").unwrap();
    let image = visual.debug_bounds("local-preview-image").unwrap();
    let expected_height = f32::from(pane.size.height) - 40.;
    assert!(expected_height > 0. && expected_height < 2000.);
    assert!(
        (f32::from(image.size.height) - expected_height).abs() <= 1.,
        "image {image:?} in {pane:?}; expected {expected_height}"
    );
    assert!((f32::from(image.size.width) - expected_height / 20.).abs() <= 1.);
    let margin = gpui::point(pane.origin.x + gpui::px(10.), image.center().y);
    visual.simulate_mouse_down(margin, gpui::MouseButton::Right, Default::default());
    visual.simulate_mouse_up(margin, gpui::MouseButton::Right, Default::default());
    cx.refresh().unwrap();
    assert!(
        gpui::VisualTestContext::from_window(view.into(), cx)
            .debug_bounds("local-image-menu")
            .is_none()
    );
}

#[test]
fn gpui_remote_files_routes_palette_commands_and_editable_fields_to_the_real_panel() {
    use crate::remote_panel::control::Command;
    let temp = tempfile::tempdir().unwrap();
    let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().to_path_buf()), false, || {});
    app.apply_gpui_action(GpuiAction::Remote(Command::Name("hidden".into())));
    assert!(app.gpui_snapshot(960., 640., 4.).remote.new_name.is_empty());
    app.apply_gpui_action(GpuiAction::PaletteExecute("remote".into()));
    assert_eq!(
        app.gpui_snapshot(960., 640., 4.).center_mode,
        GpuiCenterMode::RemoteSftp
    );
    assert!(app.gpui_handle_text_target_key(
        GpuiFocusTarget::RemoteName,
        "x",
        Some("x"),
        false,
        None
    ));
    assert_eq!(app.gpui_snapshot(960., 640., 4.).remote.new_name, "x");
    app.apply_gpui_action(GpuiAction::Remote(Command::Connect(usize::MAX)));
    let state = app.gpui_snapshot(960., 640., 4.);
    assert!(!state.remote.busy);
    assert!(state.remote.connected.is_none());
    app.apply_gpui_action(GpuiAction::SelectWorkTab);
    assert!(!app.gpui_handle_text_target_key(
        GpuiFocusTarget::RemoteName,
        "y",
        Some("y"),
        false,
        None
    ));
    assert_eq!(app.gpui_snapshot(960., 640., 4.).remote.new_name, "x");
}

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

    assert_eq!(GpuiIcon::ALL.len(), 49);
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

#[test]
fn gpui_vt_styled_lines_and_key_encoder_preserve_ansi_unicode_and_modes() {
    let mut parser = vt100::Parser::new(6, 40, 100);
    parser.process(
        b"\x1b[1;38;2;255;128;64;48;2;10;20;30mRGB\x1b[0m \x1b[3;4mItalUnder\x1b[0m A\xf0\x9f\x99\x82B\r\nSecond line",
    );
    let palette = ThemeId::TokyoNight.spec().term;
    let lines = extract_vt_styled_lines(&parser, &palette, Some(((1, 0), (1, 6))));
    assert_eq!(lines.len(), 6);
    assert!(lines[0].iter().any(|s| {
        s.text == "RGB" && s.bold && s.fg == [255, 128, 64] && s.bg == Some([10, 20, 30])
    }));
    assert!(
        lines[0]
            .iter()
            .any(|s| s.text.contains("ItalUnder") && s.italic && s.underline)
    );
    let joined_row0: String = lines[0].iter().map(|s| s.text.as_str()).collect();
    assert!(joined_row0.contains("A🙂B"));
    assert!(lines[1].iter().any(|s| s.text == "Second" && s.selected));

    assert_eq!(
        encode_gpui_terminal_key("c", Some("c"), true, false, false, false, None),
        Some((vec![3], None))
    );
    assert_eq!(
        encode_gpui_terminal_key("up", None, false, false, false, false, None),
        Some((b"\x1b[A".to_vec(), Some(BroadcastInput::CursorKey(b'A'))))
    );
    assert_eq!(
        encode_gpui_terminal_key("up", None, false, false, false, true, None),
        Some((b"\x1bOA".to_vec(), Some(BroadcastInput::CursorKey(b'A'))))
    );
    assert_eq!(
        encode_gpui_terminal_key("tab", None, false, false, true, false, None),
        Some((b"\x1b[Z".to_vec(), None))
    );
    let (shift_enter, _) =
        encode_gpui_terminal_key("enter", None, false, false, true, false, Some("claude"))
            .expect("agent shift+enter");
    if cfg!(windows) {
        assert_eq!(shift_enter, b"\x1b\r");
    } else {
        assert_eq!(shift_enter, b"\n");
    }
    assert_eq!(
        encode_gpui_terminal_key("é", Some("Québec"), false, false, false, false, None),
        Some(("Québec".as_bytes().to_vec(), None))
    );
}

#[cfg(all(windows, feature = "visual-test"))]
#[gpui::test]
fn codex_file_citation_opens_the_project_document_in_a_tab(test: &mut gpui::TestAppContext) {
    check_chat_file_click(
        test,
        GpuiAction::LaunchCodexChat,
        ":codex-file-citation{path='my report.txt'}",
    );
}

#[cfg(all(windows, feature = "visual-test"))]
#[gpui::test]
fn markdown_file_link_opens_the_project_document_in_a_tab(test: &mut gpui::TestAppContext) {
    check_chat_file_click(
        test,
        GpuiAction::LaunchCodexChat,
        "[Read the report](<my report.txt>)",
    );
}

#[cfg(all(windows, feature = "visual-test"))]
#[gpui::test]
fn claude_markdown_file_link_opens_the_project_document_in_a_tab(test: &mut gpui::TestAppContext) {
    check_chat_file_click(
        test,
        GpuiAction::LaunchClaudeChat,
        "[Read the report](<my report.txt>)",
    );
}

#[cfg(all(windows, feature = "visual-test"))]
fn check_chat_file_click(test: &mut gpui::TestAppContext, launch: GpuiAction, reply: &str) {
    let claude = matches!(launch, GpuiAction::LaunchClaudeChat);
    let cx = &mut gpui::TestAppContext::build_with_text_system(
        test.dispatcher.clone(),
        None,
        gpui_platform::current_platform(true).text_system(),
    );
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("my report.txt"), "original contents").unwrap();
    let code = "  let text = \"[sample](unused.txt)\";\n";
    let reply = format!("{reply}\n\n~~~rust\n{code}~~~");
    let view = cx.add_window(|_, cx| {
        let mut root = super::TesselGpuiRoot::new_in_dir(Some(temp.path().join("state")), false, cx);
        let app = &mut root.app;
        app.synthetic_native_profiles(temp.path());
        app.apply_gpui_action(GpuiAction::AddProjectPath(project));
        app.apply_gpui_action(launch);
        let pane = app.gpui_snapshot(960.,640.,4.).tiles[0].session_id;
        let _receiver = app.gpui_chat_sender_for_test(pane);
        let composer = app.gpui_chat_composer(pane).unwrap().0;
        app.apply_gpui_action(GpuiAction::ApplyNativeChatEdit {pane_id:pane,instance_id:composer,base:String::new(),text:"begin".into()});
        app.apply_gpui_action(GpuiAction::NativeChatSend {pane_id:pane,instance_id:composer,text:"begin".into()});
        if claude {
            app.inject_claude_frame_for_test(pane,json!({"type":"assistant","session_id":"synthetic-session","message":{"id":"reply","content":[{"type":"text","text":reply}]}}));
        } else {
        app.inject_codex_frame_for_test(pane,json!({"method":"turn/started","params":{"threadId":"synthetic-thread","turn":{"id":"turn-1"}}}));
        app.inject_codex_frame_for_test(pane,json!({"method":"item/agentMessage/delta","params":{"threadId":"synthetic-thread","turnId":"turn-1","itemId":"reply","delta":reply}}));
        }
        root
    });
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    let copy = visual
        .debug_bounds("copy-reply-code")
        .expect("copy button")
        .center();
    visual.simulate_mouse_move(copy, None, Default::default());
    visual.simulate_click(copy, Default::default());
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
            .as_deref(),
        Some(code)
    );
    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
    let bounds = visual.debug_bounds("chat-citation-body").unwrap();
    let click = gpui::point(bounds.origin.x + gpui::px(5.), bounds.center().y);
    visual.simulate_mouse_move(click, None, Default::default());
    visual.simulate_click(click, Default::default());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let opened = view
            .update(cx, |root, _, _| {
                root.app.poll_for_gpui();
                root.app.gpui_snapshot(960., 640., 4.).active_document
            })
            .unwrap();
        if let Some(doc) = opened {
            assert!(doc.title.ends_with("my report.txt"));
            assert_eq!(doc.text, "original contents");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "citation did not open"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn gpui_native_chat_approvals_questions_and_mode_switch_roundtrip() {
    let temp = tempfile::tempdir().expect("tempdir");
    let proj_dir = temp.path().join("chat-repo");
    std::fs::create_dir_all(&proj_dir).expect("create project dir");

    let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().to_path_buf()), false, || {});
    app.synthetic_native_profiles(temp.path());
    app.apply_gpui_action(GpuiAction::AddProjectPath(proj_dir));
    app.apply_gpui_action(GpuiAction::LaunchClaudeChat);

    let snap = app.gpui_snapshot(960.0, 640.0, 4.0);
    assert_eq!(snap.tiles.len(), 1);
    let pane_id = snap.tiles[0].session_id;
    assert!(snap.tiles[0].is_native_chat);
    assert!(snap.tiles[0].can_switch_chat_mode);
    assert_eq!(
        snap.tiles[0].native_chat.as_ref().map(|c| c.provider),
        Some("claude")
    );

    let instance_id = app.gpui_chat_composer(pane_id).unwrap().0;
    app.apply_gpui_action(GpuiAction::ApplyNativeChatEdit {
        pane_id,
        instance_id,
        base: String::new(),
        text: "Hello GPUI\n".into(),
    });
    let snap2 = app.gpui_snapshot(960.0, 640.0, 4.0);
    assert_eq!(
        snap2.tiles[0]
            .native_chat
            .as_ref()
            .expect("chat")
            .composer
            .as_str(),
        "Hello GPUI\n"
    );

    // Inject structured AskUserQuestion + tool approval frames
    app.inject_claude_frame_for_test(
        pane_id,
        json!({
            "type": "control_request",
            "request_id": "req-q-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "AskUserQuestion",
                "input": {
                    "questions": [{
                        "header": "Renderer",
                        "question": "Which renderer?",
                        "multiSelect": false,
                        "options": [
                            {"label": "GPUI", "description": "Hardware GPU"},
                            {"label": "Compat", "description": "OpenGL"}
                        ]
                    }]
                }
            }
        }),
    );
    app.inject_claude_frame_for_test(
        pane_id,
        json!({
            "type": "control_request",
            "request_id": "req-tool-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "Bash",
                "input": {"command": "cargo test"}
            }
        }),
    );

    let snap3 = app.gpui_snapshot(960.0, 640.0, 4.0);
    let chat3 = snap3.tiles[0].native_chat.as_ref().expect("chat3");
    assert_eq!(chat3.questions.len(), 1);
    assert!(chat3.approval.is_some());
    assert_eq!(snap3.toolbar.waiting_agents_count, 1);

    app.apply_gpui_action(GpuiAction::ChatSelectQuestionOption {
        pane_id,
        request_instance_id: chat3.questions[0].request_instance_id,
        q_idx: 0,
        opt_idx: 0,
    });
    app.apply_gpui_action(GpuiAction::ChatSubmitQuestions {
        pane_id,
        request_instance_id: chat3.questions[0].request_instance_id,
        skip: false,
    });
    app.apply_gpui_action(GpuiAction::ChatRespondApproval {
        pane_id,
        request_instance_id: chat3.approval.as_ref().unwrap().instance_id,
        allow: false,
    });
    let snap4 = app.gpui_snapshot(960.0, 640.0, 4.0);
    let chat4 = snap4.tiles[0].native_chat.as_ref().expect("chat4");
    assert!(chat4.questions.is_empty());
    assert!(chat4.approval.is_none());

    // Switch session mode in-place to CLI terminal and back to Native Chat
    app.synthetic_wait_for_terminal_operation(pane_id);
    app.apply_gpui_action(GpuiAction::SwitchSessionChatMode(pane_id));
    app.synthetic_wait_for_terminal_operation(pane_id);
    let snap5 = app.gpui_snapshot(960.0, 640.0, 4.0);
    assert_eq!(snap5.tiles[0].session_id, pane_id);
    assert!(!snap5.tiles[0].is_native_chat);
    assert!(snap5.tiles[0].can_switch_chat_mode);

    app.synthetic_wait_for_terminal_operation(pane_id);
    app.apply_gpui_action(GpuiAction::SwitchSessionChatMode(pane_id));
    app.synthetic_wait_for_terminal_operation(pane_id);
    let snap6 = app.gpui_snapshot(960.0, 640.0, 4.0);
    assert_eq!(snap6.tiles[0].session_id, pane_id);
    assert!(snap6.tiles[0].is_native_chat);
}

#[test]
fn gpui_document_editor_rich_previews_and_mermaid_layouts_work_end_to_end() {
    let temp = tempfile::tempdir().expect("tempdir");
    let proj_dir = temp.path().join("editor-repo");
    std::fs::create_dir_all(&proj_dir).expect("create project dir");

    let rs_path = proj_dir.join("lib.rs");
    std::fs::write(&rs_path, "pub fn answer() -> i32 {\n    41\n}\n").expect("write rs");
    let csv_path = proj_dir.join("table.csv");
    std::fs::write(&csv_path, "name,score\nAlice,98\nBob,95\n").expect("write csv");
    let jsonl_path = proj_dir.join("log.jsonl");
    std::fs::write(
        &jsonl_path,
        "{\"step\":1,\"ok\":true}\n{\"step\":2,\"ok\":false}\n",
    )
    .expect("write jsonl");

    let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().to_path_buf()), false, || {});
    app.apply_gpui_action(GpuiAction::AddProjectPath(proj_dir));

    // Open Rust source file, Find/Replace `41` -> `42`, and atomic Save
    app.apply_gpui_action(GpuiAction::OpenDocumentPath(rs_path.clone()));
    let snap = app.gpui_snapshot(960.0, 640.0, 4.0);
    assert_eq!(snap.center_mode, GpuiCenterMode::DocumentEditor);
    let doc = snap.active_document.expect("active doc");
    assert_eq!(doc.lang, SyntaxLang::Rust);
    assert!(!doc.dirty);

    app.apply_gpui_action(GpuiAction::ToggleEditorFind);
    app.apply_gpui_action(GpuiAction::SetEditorFindQuery("41".into()));
    app.apply_gpui_action(GpuiAction::SetEditorReplaceText("42".into()));
    app.apply_gpui_action(GpuiAction::ReplaceEditorAll);
    let snap_edited = app.gpui_snapshot(960.0, 640.0, 4.0);
    let doc_edited = snap_edited.active_document.expect("edited doc");
    assert!(doc_edited.dirty);
    assert!(doc_edited.text.contains("42"));
    assert!(!doc_edited.hunk_lines.is_empty());

    app.apply_gpui_action(GpuiAction::SaveActiveDocument);
    assert!(
        std::fs::read_to_string(&rs_path)
            .expect("read saved")
            .contains("42")
    );
    assert!(
        !app.gpui_snapshot(960.0, 640.0, 4.0)
            .active_document
            .expect("saved doc")
            .dirty
    );

    // Open CSV (auto-opens in SheetView preview) & JSONL (toggles into JsonDocumentPreview)
    app.apply_gpui_action(GpuiAction::OpenDocumentPath(csv_path));
    let csv_doc = app
        .gpui_snapshot(960.0, 640.0, 4.0)
        .active_document
        .expect("csv doc");
    assert!(csv_doc.preview_mode);
    assert_eq!(csv_doc.sheet.as_ref().map(|s| s.rows.len()), Some(3));
    let cached_sheet = csv_doc.sheet.unwrap();
    let repeated_sheet = app
        .gpui_snapshot(960., 640., 4.)
        .active_document
        .unwrap()
        .sheet
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&cached_sheet, &repeated_sheet));
    app.apply_gpui_action(GpuiAction::ToggleActiveDocumentPreview);
    assert!(
        !app.gpui_snapshot(960.0, 640.0, 4.0)
            .active_document
            .expect("csv source doc")
            .preview_mode
    );

    app.apply_gpui_action(GpuiAction::SetEditorFindQuery("Alice".into()));
    app.apply_gpui_action(GpuiAction::SetEditorReplaceText("Alicia".into()));
    app.apply_gpui_action(GpuiAction::ReplaceEditorAll);
    app.apply_gpui_action(GpuiAction::ToggleActiveDocumentPreview);
    let updated_sheet = app
        .gpui_snapshot(960., 640., 4.)
        .active_document
        .unwrap()
        .sheet
        .unwrap();
    assert!(!std::sync::Arc::ptr_eq(&cached_sheet, &updated_sheet));
    assert_eq!(cached_sheet.rows[1][0], "Alice");
    assert_eq!(updated_sheet.rows[1][0], "Alicia");

    app.apply_gpui_action(GpuiAction::OpenDocumentPath(jsonl_path));
    app.apply_gpui_action(GpuiAction::ToggleActiveDocumentPreview);
    let jsonl_doc = app
        .gpui_snapshot(960.0, 640.0, 4.0)
        .active_document
        .expect("jsonl doc");
    assert!(jsonl_doc.json_preview.is_some());

    // Deterministic Mermaid flowchart & sequence geometry
    if let Ok(MermaidDiagram::Flowchart {
        direction,
        nodes,
        edges,
    }) = parse_mermaid_diagram("flowchart LR\n  A[Client] --> B[GPUI]\n  B --> C[PTY]")
    {
        let flow = layout_mermaid_flowchart(direction, &nodes, &edges);
        assert_eq!(flow.nodes.len(), 3);
        assert_eq!(flow.edges.len(), 2);
        assert!(flow.width > 400.0 && flow.height >= 96.0);
    } else {
        panic!("expected Mermaid flowchart");
    }

    if let Ok(MermaidDiagram::Sequence {
        participants,
        messages,
    }) =
        parse_mermaid_diagram("sequenceDiagram\n  User->>Tessel: Ctrl+`\n  Tessel-->>PTY: spawn")
    {
        let seq = layout_mermaid_sequence(&participants, &messages);
        assert_eq!(seq.actor_centers_x.len(), 3);
        assert_eq!(seq.message_rows.len(), 2);
        assert!(seq.width >= 260.0 && seq.height >= 120.0);
    } else {
        panic!("expected Mermaid sequence diagram");
    }
}

#[test]
fn gpui_sidebar_and_seven_side_panels_roundtrip() {
    use crate::{
        project_panel::ProjectAction,
        side_panels_model::{
            ConversationWindow, DashboardGroupBy, DashboardState, SessionHistoryGroupBy,
            SessionHistoryScope, TaskHistoryFilter, TaskHistorySort,
        },
        tasks_board_panel::TasksBoardAction,
        workspace::SidebarPreferences,
    };

    let temp = tempfile::tempdir().expect("tempdir");
    let proj_dir = temp.path().join("panels-repo");
    std::fs::create_dir_all(proj_dir.join("src")).expect("create src");
    std::fs::write(
        proj_dir.join("src").join("main.rs"),
        "fn main() {\n    println!(\"needle_token\");\n}\n",
    )
    .expect("write main.rs");

    let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().to_path_buf()), false, || {});
    app.apply_gpui_action(GpuiAction::AddProjectPath(proj_dir));
    let pid = app.gpui_snapshot(960.0, 640.0, 4.0).projects[0].id;

    // Sidebar folder group assignment + collapse/expand + preferences
    app.apply_gpui_action(GpuiAction::Project(ProjectAction::EditProjectGroup(pid)));
    app.apply_gpui_action(GpuiAction::SetPromptDraft("Platform".into()));
    app.apply_gpui_action(GpuiAction::SubmitPromptModal);
    let snap_grp = app.gpui_snapshot(960.0, 640.0, 4.0);
    assert_eq!(snap_grp.projects[0].group_name.as_deref(), Some("Platform"));

    app.apply_gpui_action(GpuiAction::Project(ProjectAction::ToggleProjectGroup(
        "Platform".into(),
    )));
    assert!(
        app.gpui_snapshot(960.0, 640.0, 4.0)
            .sidebar
            .collapsed_groups
            .contains("Platform")
    );
    app.apply_gpui_action(GpuiAction::Project(ProjectAction::ToggleProjectGroup(
        "Platform".into(),
    )));

    app.apply_gpui_action(GpuiAction::SetSidebarPrefs(SidebarPreferences {
        compact_rows: true,
        hide_agent_task_line: true,
        ..Default::default()
    }));
    let snap_prefs = app.gpui_snapshot(960.0, 640.0, 4.0);
    assert!(snap_prefs.sidebar.prefs.compact_rows);
    assert!(snap_prefs.sidebar.prefs.hide_agent_task_line);

    // 1. Dashboard
    app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::Dashboard));
    app.apply_gpui_action(GpuiAction::SetDashboardGroup(DashboardGroupBy::Project));
    app.apply_gpui_action(GpuiAction::SetDashboardFilter(Some(
        DashboardState::Working,
    )));
    app.apply_gpui_action(GpuiAction::SetDashboardQuery("agent".into()));
    let dash = app.gpui_snapshot(960.0, 640.0, 4.0).side_panels;
    assert_eq!(dash.dashboard_group, DashboardGroupBy::Project);
    assert_eq!(dash.dashboard_filter, Some(DashboardState::Working));
    assert_eq!(dash.dashboard_query, "agent");

    // 2. Files (Explorer + async background content search)
    app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::Files));
    app.apply_gpui_action(GpuiAction::ExplorerSetFilter("needle_token".into()));
    app.apply_gpui_action(GpuiAction::ExplorerToggleSearchMode);
    let mut exp = app.gpui_snapshot(960.0, 640.0, 4.0).side_panels.explorer;
    for _ in 0..80 {
        if !exp.content_hits.is_empty() || !exp.loading {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        exp = app.gpui_snapshot(960.0, 640.0, 4.0).side_panels.explorer;
    }
    assert!(exp.content_search_mode);
    assert!(!exp.content_hits.is_empty());

    // 3. Changes (SCM + commits drawer toggle)
    app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::Changes));
    let initial_commits_open = app
        .gpui_snapshot(960.0, 640.0, 4.0)
        .side_panels
        .commits_open;
    app.apply_gpui_action(GpuiAction::ScmSetCommitMessage("feat: gpui scm".into()));
    app.apply_gpui_action(GpuiAction::ScmToggleCommitsOpen);
    let scm = app.gpui_snapshot(960.0, 640.0, 4.0).side_panels;
    assert_eq!(scm.scm_commit_msg, "feat: gpui scm");
    assert_eq!(scm.commits_open, !initial_commits_open);

    // 4. Tasks Kanban (Add -> Move -> Delete)
    app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::Tasks));
    app.apply_gpui_action(GpuiAction::TasksSetNewTitle("Ship Lot b176".into()));
    app.apply_gpui_action(GpuiAction::TasksAddFromDraft);
    let tasks1 = app.gpui_snapshot(960.0, 640.0, 4.0).side_panels.tasks;
    assert_eq!(tasks1.len(), 1);
    let tid = tasks1[0].id;
    assert_eq!(tasks1[0].column, 0);

    app.apply_gpui_action(GpuiAction::TasksBoard(TasksBoardAction::Move {
        id: tid,
        column: 3,
    }));
    assert_eq!(
        app.gpui_snapshot(960.0, 640.0, 4.0).side_panels.tasks[0].column,
        3
    );

    app.apply_gpui_action(GpuiAction::TasksBoard(TasksBoardAction::Delete(tid)));
    assert!(
        app.gpui_snapshot(960.0, 640.0, 4.0)
            .side_panels
            .tasks
            .is_empty()
    );

    // 5. TaskHistory, 6. History, 7. Conversations
    app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::TaskHistory));
    app.apply_gpui_action(GpuiAction::SetTaskHistoryWindow(TaskHistoryFilter::Days7));
    app.apply_gpui_action(GpuiAction::SetTaskHistorySort(TaskHistorySort::Time));
    app.apply_gpui_action(GpuiAction::SetTaskHistoryQuery("build".into()));

    app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::History));
    app.apply_gpui_action(GpuiAction::SetSessionsScope(SessionHistoryScope::All));
    app.apply_gpui_action(GpuiAction::SetSessionsGroup(SessionHistoryGroupBy::Agent));
    app.apply_gpui_action(GpuiAction::SetSessionsQuery("claude".into()));

    app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::Conversations));
    app.apply_gpui_action(GpuiAction::SetConversationsWindow(
        ConversationWindow::Days30,
    ));
    app.apply_gpui_action(GpuiAction::SetConversationsQuery("relay".into()));

    let sp_final = app.gpui_snapshot(960.0, 640.0, 4.0).side_panels;
    assert_eq!(sp_final.history_window, TaskHistoryFilter::Days7);
    assert_eq!(sp_final.history_sort, TaskHistorySort::Time);
    assert_eq!(sp_final.sessions_scope, SessionHistoryScope::All);
    assert_eq!(sp_final.conv_window, ConversationWindow::Days30);
    assert_eq!(sp_final.conv_query, "relay");
}

#[test]
fn gpui_settings_twenty_two_pages_and_preferences_roundtrip() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().to_path_buf()), false, || {});

    app.apply_gpui_action(GpuiAction::OpenSettings);
    let initial_settings = app
        .gpui_snapshot(960.0, 640.0, 4.0)
        .settings
        .expect("settings open");
    let all_page_ids: Vec<String> = initial_settings
        .groups
        .iter()
        .flat_map(|(_, pages)| pages.iter().map(|(id, _)| id.clone()))
        .collect();
    assert_eq!(all_page_ids.len(), 22);

    for page_id in &all_page_ids {
        app.apply_gpui_action(GpuiAction::OpenSettingsPage(page_id.clone()));
        let cur = app
            .gpui_snapshot(960.0, 640.0, 4.0)
            .settings
            .expect("settings page");
        assert_eq!(&cur.active_page_id, page_id);
    }

    // Page navigation clears the previous filter.
    app.apply_gpui_action(GpuiAction::SetSettingsSearch("git".into()));
    let filtered = app
        .gpui_snapshot(960.0, 640.0, 4.0)
        .settings
        .expect("filtered settings");
    let filtered_count: usize = filtered.groups.iter().map(|(_, p)| p.len()).sum();
    assert_eq!(filtered_count, all_page_ids.len());
    app.apply_gpui_action(GpuiAction::OpenSettingsPage("git".into()));
    assert!(
        app.gpui_snapshot(960.0, 640.0, 4.0)
            .settings
            .expect("cleared search")
            .search_query
            .is_empty()
    );

    // Quick commands, SSH hosts, and Custom CLI agent creation
    app.apply_gpui_action(GpuiAction::SetQuickCommandDraft {
        name: "Cargo Check".into(),
        text: "cargo check".into(),
        press_enter: true,
    });
    app.apply_gpui_action(GpuiAction::SubmitNewQuickCommand);

    app.apply_gpui_action(GpuiAction::SetSshDraft {
        alias: "prod-box".into(),
        folder: "/srv/app".into(),
    });
    app.apply_gpui_action(GpuiAction::AddSshHostFromDraft);

    app.apply_gpui_action(GpuiAction::SetCustomAgentDraft {
        name: "My Bot".into(),
        cmd: "my-bot --chat".into(),
    });
    app.apply_gpui_action(GpuiAction::SubmitNewCustomAgent);

    let updated = app
        .gpui_snapshot(960.0, 640.0, 4.0)
        .settings
        .expect("updated settings");
    assert_eq!(updated.quick_commands.len(), 1);
    assert_eq!(updated.quick_commands[0].name, "Cargo Check");
    assert_eq!(updated.ssh_hosts.len(), 1);
    assert_eq!(updated.ssh_hosts[0].alias, "prod-box");
    assert_eq!(updated.custom_agents.len(), 1);
    assert_eq!(updated.custom_agents[0].name, "My Bot");

    #[cfg(windows)]
    {
        app.apply_gpui_action(GpuiAction::OpenSettingsPage("passwords".into()));
        let pw = app
            .gpui_snapshot(960.0, 640.0, 4.0)
            .settings
            .expect("passwords page");
        assert_eq!(pw.active_page_id, "passwords");
        assert!(pw.vault.is_some());
    }

    app.apply_gpui_action(GpuiAction::CloseOverlays);
    assert!(app.gpui_snapshot(960.0, 640.0, 4.0).settings.is_none());
}

#[test]
fn gpui_modals_palette_finder_launch_notes_teams_and_popovers_roundtrip() {
    use crate::{
        dialogs_toolbar_ui::AddProjectTab, finder_model::LaunchPlacement,
        side_panels_ui::SidePanelsAction, status_strip::StatusBarItemId, teams_ui::TeamsUiAction,
    };

    let temp = tempfile::tempdir().expect("tempdir");
    let proj_dir = temp.path().join("modals-repo");
    std::fs::create_dir_all(&proj_dir).expect("create modals-repo");
    std::fs::write(proj_dir.join("hello.txt"), "hello world\n").expect("write hello.txt");

    let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().to_path_buf()), false, || {});
    app.apply_gpui_action(GpuiAction::AddProjectPath(proj_dir.clone()));
    let pid = app.gpui_snapshot(960.0, 640.0, 4.0).projects[0].id;

    // Command Palette
    app.apply_gpui_action(GpuiAction::ToggleCommandPalette);
    assert!(app.gpui_is_palette_open());
    app.apply_gpui_action(GpuiAction::PaletteSetQuery("theme".into()));
    app.apply_gpui_action(GpuiAction::PaletteStep(1));
    assert!(
        !app.gpui_snapshot(960.0, 640.0, 4.0)
            .modals
            .palette_rows
            .is_empty()
    );
    app.apply_gpui_action(GpuiAction::CloseOverlays);
    assert!(!app.gpui_is_palette_open());

    // Launch Menu
    app.apply_gpui_action(GpuiAction::ToggleLaunchMenu);
    app.apply_gpui_action(GpuiAction::SetLaunchPlacement(LaunchPlacement::Right));
    app.apply_gpui_action(GpuiAction::ToggleLaunchInWorktree);
    let m_launch = app.gpui_snapshot(960.0, 640.0, 4.0).modals;
    assert!(m_launch.launch_menu_open);
    assert_eq!(m_launch.launch_placement, LaunchPlacement::Right);
    assert!(m_launch.launch_in_worktree);
    app.apply_gpui_action(GpuiAction::CloseOverlays);

    // Help Dialog + AddProjectDialog clone URL auto-folder derivation
    app.apply_gpui_action(GpuiAction::OpenHelp);
    app.apply_gpui_action(GpuiAction::SetHelpQuery("palette".into()));
    assert!(
        !app.gpui_snapshot(960.0, 640.0, 4.0)
            .modals
            .help_groups
            .is_empty()
    );
    app.apply_gpui_action(GpuiAction::CloseOverlays);

    app.apply_gpui_action(GpuiAction::OpenAddProjectDialog);
    app.apply_gpui_action(GpuiAction::SetAddProjectTab(AddProjectTab::CloneRepo));
    app.apply_gpui_action(GpuiAction::SetAddProjectCloneUrl(
        "https://github.com/acme/super-widget.git".into(),
    ));
    assert_eq!(
        app.gpui_snapshot(960.0, 640.0, 4.0).modals.add_folder_name,
        "super-widget"
    );
    app.apply_gpui_action(GpuiAction::CloseOverlays);

    // Project Notes `.tessel/notes.md` modal
    app.apply_gpui_action(GpuiAction::OpenNotesModal);
    app.apply_gpui_action(GpuiAction::SetNotesText("# Sprint Notes\nAll green".into()));
    assert!(app.gpui_snapshot(960.0, 640.0, 4.0).modals.notes_dirty);
    app.apply_gpui_action(GpuiAction::SidePanels(SidePanelsAction::SaveNotes {
        force: true,
    }));
    assert!(
        std::fs::read_to_string(proj_dir.join(".tessel").join("notes.md"))
            .expect("notes.md")
            .contains("All green")
    );
    app.apply_gpui_action(GpuiAction::CloseOverlays);

    // Teams Manager + Toolbar & StatusBar popovers
    app.apply_gpui_action(GpuiAction::OpenTeamsManager(pid));
    app.apply_gpui_action(GpuiAction::Teams(TeamsUiAction::CreateTeam {
        project: pid,
        name: "Team 2".into(),
    }));
    assert_eq!(
        app.gpui_snapshot(960.0, 640.0, 4.0).modals.teams_list.len(),
        1
    );
    app.apply_gpui_action(GpuiAction::CloseTeamsManager);

    app.apply_gpui_action(GpuiAction::ToggleUsagePopover);
    assert!(
        app.gpui_snapshot(960.0, 640.0, 4.0)
            .modals
            .usage_popover_open
    );
    app.apply_gpui_action(GpuiAction::ToggleNotificationsPopover);
    let m_notif = app.gpui_snapshot(960.0, 640.0, 4.0).modals;
    assert!(m_notif.notifications_popover_open);
    assert!(!m_notif.usage_popover_open);

    app.apply_gpui_action(GpuiAction::ToggleStatusPopover(StatusBarItemId::Resources));
    assert_eq!(
        app.gpui_snapshot(960.0, 640.0, 4.0).modals.status_popover,
        Some(StatusBarItemId::Resources)
    );
    app.apply_gpui_action(GpuiAction::CloseOverlays);
    assert!(!app.gpui_has_open_modal());
}

#[test]
fn gpui_native_chat_draft_guards_scope_size_and_stale_events() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
    app.apply_gpui_action(GpuiAction::AddProjectPath(project));
    app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
    let pane = app.gpui_snapshot(960., 640., 4.).tiles[0].session_id;
    let id = app.gpui_chat_composer(pane).unwrap().0;
    app.apply_gpui_action(GpuiAction::ApplyNativeChatEdit {
        pane_id: pane,
        instance_id: id,
        base: String::new(),
        text: "draft".into(),
    });
    for (instance_id, base, text) in [
        (id, "", "stale".into()),
        (id + 1, "draft", "other instance".into()),
        (id, "draft", "x".repeat(128 * 1024 + 1)),
    ] {
        app.apply_gpui_action(GpuiAction::ApplyNativeChatEdit {
            pane_id: pane,
            instance_id,
            base: base.into(),
            text,
        });
        assert_eq!(app.gpui_chat_composer(pane), Some((id, "draft")));
    }
    // A stale send cannot even enter the backend (which would set an offline error).
    app.apply_gpui_action(GpuiAction::NativeChatSend {
        pane_id: pane,
        instance_id: id,
        text: "old".into(),
    });
    assert!(
        app.gpui_snapshot(960., 640., 4.).tiles[0]
            .native_chat
            .as_ref()
            .unwrap()
            .error
            .is_empty()
    );
    app.apply_gpui_action(GpuiAction::OpenHelp);
    assert!(!app.gpui_chat_input_allowed(pane));
    app.apply_gpui_action(GpuiAction::ApplyNativeChatEdit {
        pane_id: pane,
        instance_id: id,
        base: "draft".into(),
        text: "hidden".into(),
    });
    assert_eq!(app.gpui_chat_composer(pane), Some((id, "draft")));
}

#[test]
fn gpui_native_approval_delayed_click_cannot_resolve_the_next_request() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
    app.apply_gpui_action(GpuiAction::AddProjectPath(project));
    app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
    let pane = app.gpui_snapshot(960., 640., 4.).tiles[0].session_id;
    for id in ["first", "second"] {
        app.inject_claude_frame_for_test(pane,json!({"type":"control_request","request_id":id,"request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"synthetic command; never executed"}}}));
    }
    let first = app.gpui_chat_request_id(pane, true).unwrap();
    app.apply_gpui_action(GpuiAction::ChatRespondApproval {
        pane_id: pane,
        request_instance_id: first,
        allow: false,
    });
    let second = app.gpui_chat_request_id(pane, true).unwrap();
    assert_ne!(first, second);
    for allow in [true, false] {
        app.apply_gpui_action(GpuiAction::ChatRespondApproval {
            pane_id: pane,
            request_instance_id: first,
            allow,
        });
        assert_eq!(app.gpui_chat_request_id(pane, true), Some(second));
    }
    app.apply_gpui_action(GpuiAction::OpenHelp);
    app.apply_gpui_action(GpuiAction::ChatRespondApproval {
        pane_id: pane,
        request_instance_id: second,
        allow: true,
    });
    assert_eq!(app.gpui_chat_request_id(pane, true), Some(second));
    app.apply_gpui_action(GpuiAction::CloseOverlays);
    app.apply_gpui_action(GpuiAction::ChatRespondApproval {
        pane_id: pane,
        request_instance_id: second,
        allow: false,
    });
    assert!(app.gpui_chat_request_id(pane, true).is_none());
}
