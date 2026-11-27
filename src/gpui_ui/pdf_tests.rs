#[gpui::test]
fn unreadable_pdf_replaces_reading_notice_inside_viewport(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("damaged.pdf");
    let bytes = b"%PDF-1.7\ntruncated".to_vec();
    std::fs::write(&path, &bytes).unwrap();
    let mut doc = crate::files::Document::open(dir.path(), &path).unwrap();
    let sender = doc.pdf.as_mut().unwrap().test_pending_page();
    let window = cx.add_window(|_, cx| {
        let mut root =
            crate::gpui_ui::TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
        root.app
            .apply_gpui_action(crate::app::gpui_bridge::GpuiAction::AddProjectPath(
                dir.path().into(),
            ));
        root.app.install_editor_input_fixture(vec![doc], 0);
        root
    });
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let viewport = visual.debug_bounds("pdf-page-viewport").unwrap();
    assert!(viewport.contains(&visual.debug_bounds("pdf-reading").unwrap().center()));
    sender
        .send(Err("Unsupported or invalid PDF".into()))
        .unwrap();
    window
        .update(cx, |root, _, cx| {
            assert!(root.app.poll_for_gpui());
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    assert!(visual.debug_bounds("pdf-reading").is_none());
    assert_eq!(visual.debug_bounds("pdf-page-viewport").unwrap(), viewport);
    assert!(viewport.contains(&visual.debug_bounds("pdf-read-error").unwrap().center()));
}

#[gpui::test]
fn native_pdf_fits_and_scrolls_between_original_pages(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture.pdf");
    let mut source = crate::pdf::fixture();
    let offset = source.windows(7).rposition(|v| v == b"300 180").unwrap();
    source[offset..offset + 7].copy_from_slice(b"300 980");
    std::fs::write(&path, &source).unwrap();
    let mut doc = crate::files::Document::open(dir.path(), &path).unwrap();
    doc.pdf.as_mut().unwrap().seed_test_pages(source);
    let window = cx.add_window(|_, cx| {
        let mut root =
            crate::gpui_ui::TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
        root.app
            .apply_gpui_action(crate::app::gpui_bridge::GpuiAction::AddProjectPath(
                dir.path().into(),
            ));
        root.app.install_editor_input_fixture(vec![doc], 0);
        root
    });
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let before = visual.debug_bounds("pdf-page-image-0").unwrap();
    assert_eq!(f32::from(before.size.width), 300.);
    let position = visual.debug_bounds("pdf-fit").unwrap().center();
    visual.simulate_click(position, Default::default());
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let expected = (f32::from(visual.debug_bounds("pdf-page-viewport").unwrap().size.width) - 48.)
        .clamp(60., 1200.);
    let actual = f32::from(visual.debug_bounds("pdf-page-image-0").unwrap().size.width);
    assert!(
        (expected - actual).abs() < 2.,
        "expected {expected}, got {actual}"
    );
    let pane = visual.debug_bounds("pdf-page-viewport").unwrap();
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: pane.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-2500.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    assert!(visual.debug_bounds("pdf-page-image-0").is_none());
    let next = visual
        .debug_bounds("pdf-page-image-1")
        .expect("second PDF page");
    assert!(next.origin.y < pane.origin.y);
    assert!(
        ((f32::from(next.size.height) / f32::from(next.size.width)) - 980. / 300.).abs() < 0.03
    );
}
