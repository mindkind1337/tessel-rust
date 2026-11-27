use super::{fixtures::*, *};

fn pending_view() -> (OfficeView, mpsc::Sender<Result<Book, String>>) {
    let (sender, receiver) = mpsc::channel();
    (
        OfficeView {
            bytes: Arc::new(document()),
            extension: "docx".into(),
            book: None,
            pending: Some(receiver),
            cancel: Arc::new(AtomicBool::new(false)),
            selected: 0,
            error: String::new(),
            summary: None,
        },
        sender,
    )
}

#[test]
fn gpui_office_worker_completion_preserves_original_and_stops_repainting() {
    let (mut view, sender) = pending_view();
    let original = view.bytes.clone();
    assert!(view.is_rendering());
    assert!(view.gpui_poll_summary().busy);
    sender
        .send(Ok(parse::parse(&document(), "docx").unwrap()))
        .unwrap();
    view.select_section(usize::MAX);
    let ready = view.gpui_poll_summary();
    assert!(!ready.busy && !view.is_rendering());
    assert_eq!(ready.selected, 0);
    assert_eq!(ready.paragraphs[1].1, "A & B");
    assert!(ready.error.is_empty());
    assert!(Arc::ptr_eq(&ready, &view.gpui_poll_summary()));
    assert!(Arc::ptr_eq(&original, &view.bytes));
}

#[test]
fn disconnected_office_worker_reports_error_without_restarting_forever() {
    let (mut view, sender) = pending_view();
    drop(sender);
    let failed = view.gpui_poll_summary();
    assert!(!failed.busy && !view.is_rendering());
    assert!(failed.error.contains("stopped"));
    assert_eq!(view.gpui_poll_summary().error, failed.error);
    assert!(!view.is_rendering());
}

#[test]
fn gpui_office_sections_and_truncation_survive_worker_delivery() {
    let (mut view, sender) = pending_view();
    let mut book = parse::parse(&spreadsheet(), "xlsx").unwrap();
    let second = parse::parse(&document(), "docx")
        .unwrap()
        .sections
        .remove(0);
    book.sections.push(second);
    book.truncated = true;
    sender.send(Ok(book)).unwrap();
    let sheet = view.gpui_poll_summary();
    assert!(sheet.truncated);
    assert_eq!(sheet.sheet.as_ref().unwrap().rows[0][0], "Original value");
    view.select_section(1);
    let document = view.gpui_poll_summary();
    assert!(!Arc::ptr_eq(&sheet, &document));
    assert!(Arc::ptr_eq(&document, &view.gpui_poll_summary()));
    assert_eq!(document.selected, 1);
    assert!(document.sheet.is_none());
    assert_eq!(document.paragraphs[1].1, "A & B");
}

#[test]
fn document_entities_headings_and_empty_runs() {
    let book = parse::parse(&document(), "docx").unwrap();
    assert!(book.sections[0].paragraphs[0].heading);
    assert_eq!(book.sections[0].paragraphs[1].text, "A & B");
    assert_eq!(column(0), "A");
    assert_eq!(column(25), "Z");
    assert_eq!(column(26), "AA");
}
#[test]
fn spreadsheet_uses_cached_values() {
    for extension in ["xlsx", "xlsm"] {
        let book = parse::parse(&spreadsheet(), extension).unwrap();
        assert_eq!(book.sections[0].title, "Example");
        assert_eq!(book.sections[0].rows[0], ["Original value", "42", "43"]);
    }
}
#[test]
fn open_document_text_and_slides() {
    let content = r#"<office:document xmlns:office="urn:o" xmlns:text="urn:t" xmlns:draw="urn:d"><draw:page draw:name="First"><text:h>Heading</text:h><text:p>A<text:s text:c="2"/>B &amp; C</text:p></draw:page><draw:page draw:name="Second"><text:p>Other slide</text:p></draw:page></office:document>"#;
    let bytes = archive(&[("content.xml", content)]);
    let book = parse::parse(&bytes, "odt").unwrap();
    assert_eq!(book.sections[0].paragraphs[1].text, "A  B & C");
    let book = parse::parse(&bytes, "odp").unwrap();
    assert_eq!(book.sections.len(), 2);
    assert_eq!(book.sections[1].title, "Second");
    assert_eq!(book.sections[1].paragraphs[0].text, "Other slide");
}
#[test]
fn presentation_preserves_relationship_order_and_refuses_escape() {
    let rels = r#"<Relationships><Relationship Id="two" Target="slides/slide2.xml"/><Relationship Id="one" Target="slides/slide1.xml"/></Relationships>"#;
    let pres = r#"<p:presentation xmlns:p="urn:p" xmlns:r="urn:r"><p:sldId r:id="two"/><p:sldId r:id="one"/></p:presentation>"#;
    let slides = |rels| {
        archive(&[
            ("ppt/_rels/presentation.xml.rels", rels),
            ("ppt/presentation.xml", pres),
            (
                "ppt/slides/slide1.xml",
                "<a:p><a:r><a:t>One</a:t></a:r></a:p>",
            ),
            (
                "ppt/slides/slide2.xml",
                "<a:p><a:r><a:t>Two</a:t></a:r></a:p>",
            ),
        ])
    };
    let book = parse::parse(&slides(rels), "pptx").unwrap();
    assert_eq!(book.sections[0].paragraphs[0].text, "Two");
    assert_eq!(book.sections[1].paragraphs[0].text, "One");
    assert!(
        parse::parse(
            &slides(&rels.replace("slides/slide2.xml", "../secret")),
            "pptx"
        )
        .is_err()
    );
}
#[test]
fn damaged_archives_and_entities_are_refused() {
    assert!(parse::parse(b"broken", "docx").is_err());
    for xml in [
        "<!DOCTYPE root SYSTEM 'file:///secret'><w:p><w:t>Hi</w:t></w:p>",
        "<w:p><w:t>&unknown;</w:t></w:p>",
        "<w:p><w:t>unfinished",
    ] {
        assert!(parse::parse(&archive(&[("word/document.xml", xml)]), "docx").is_err());
    }
}

#[test]
fn ods_values_and_repeated_cells() {
    let bytes = archive(&[
        ("mimetype", "application/vnd.oasis.opendocument.spreadsheet"),
        (
            "META-INF/manifest.xml",
            r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/></manifest:manifest>"#,
        ),
        (
            "content.xml",
            r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="Sheet"><table:table-row><table:table-cell office:value-type="string"><text:p>Hello</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="12" table:number-columns-repeated="2"/></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#,
        ),
    ]);
    let book = parse::parse(&bytes, "ods").unwrap();
    assert_eq!(book.sections[0].rows[0], ["Hello", "12", "12"]);
}

#[cfg(feature = "visual-test")]
#[gpui::test]
fn office_workbook_tabs_switch_between_data_and_an_empty_sheet(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture.xlsx");
    let bytes = spreadsheet();
    std::fs::write(&path, &bytes).unwrap();
    let mut doc = crate::files::Document::open(dir.path(), &path).unwrap();
    let mut book = parse::parse(&bytes, "xlsx").unwrap();
    book.sections.push(Section {
        title: "Blank".into(),
        rows: vec![],
        paragraphs: vec![],
        omitted_rows: 0,
        omitted_cols: 0,
        start_row: 0,
        start_col: 0,
    });
    doc.office.as_mut().unwrap().book = Some(book);
    let window = cx.add_window(|_, cx| {
        let mut root =
            crate::gpui_ui::TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
        root.app.install_editor_input_fixture(vec![doc], 0);
        root
    });
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let footer = visual.debug_bounds("office-section-tabs").unwrap();
    assert!(visual.debug_bounds("office-empty-sheet").is_none());
    let blank = visual.debug_bounds("office-section-1").unwrap().center();
    visual.simulate_click(blank, Default::default());
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let empty = visual.debug_bounds("office-empty-sheet").unwrap();
    assert!(empty.bottom() <= footer.top());
    let first = visual.debug_bounds("office-section-0").unwrap().center();
    visual.simulate_click(first, Default::default());
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    assert!(visual.debug_bounds("office-empty-sheet").is_none());
    assert_eq!(visual.debug_bounds("office-section-tabs").unwrap(), footer);
}

#[test]
fn empty_office_workbooks_have_no_synthetic_section() {
    let (mut view, tx) = pending_view();
    view.extension = "xlsx".into();
    let book = Book {
        sections: vec![],
        truncated: false,
    };
    assert!(book.validate("xlsx").is_ok());
    assert!(book.validate("docx").is_err());
    tx.send(Ok(book)).unwrap();
    let summary = view.gpui_poll_summary();
    assert!(summary.workbook && summary.section_titles.is_empty() && summary.sheet.is_none());
    assert!(!summary.busy && summary.error.is_empty());
}

#[test]
fn office_sparse_sheet_preserves_coordinates_and_counts_omitted_dimensions() {
    let bytes = spreadsheet_with_sheet(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="5"><c r="D5"><v>7</v></c><c r="GR5"><v>200</v></c><c r="GS5"><v>201</v></c></row><row r="5002"><c r="D5002"><v>99</v></c></row></sheetData></worksheet>"#,
    );
    let book = parse::parse(&bytes, "xlsx").unwrap();
    let sheet = &book.sections[0];
    assert_eq!((sheet.start_row, sheet.start_col), (0, 0));
    assert_eq!((sheet.omitted_rows, sheet.omitted_cols), (2, 1));
    assert_eq!(sheet.rows.len(), 5);
    assert!(sheet.rows[..4].iter().all(Vec::is_empty));
    assert_eq!(&sheet.rows[4][..4], ["", "", "", "7"]);
    assert_eq!(sheet.rows[4].len(), 200);
    assert_eq!(sheet.rows[4][199], "200");
}

#[test]
fn office_excel_dates_times_numbers_and_booleans_match_vue() {
    let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" s="1"><v>25569.5</v></c><c r="B1" s="1"><v>0.25</v></c><c r="C1" s="1"><v>25569</v></c><c r="D1"><v>0.30000000000000004</v></c><c r="E1" t="b"><v>1</v></c><c r="F1"><v>0.00000001</v></c><c r="G1" t="b"><v>0</v></c></row></sheetData></worksheet>"#;
    for (date1904, day) in [(false, "1970-01-01"), (true, "1974-01-02")] {
        let book = parse::parse(&styled_spreadsheet(sheet, date1904), "xlsx").unwrap();
        assert_eq!(
            book.sections[0].rows[0],
            [
                format!("{day} 12:00:00"),
                "06:00:00".into(),
                day.into(),
                "0.3".into(),
                "TRUE".into(),
                "1e-8".into(),
                "FALSE".into()
            ]
        );
    }
}

#[cfg(feature = "visual-test")]
#[gpui::test]
fn office_presentation_shows_all_slides_in_order_including_blank_pages(
    cx: &mut gpui::TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("slides.odp");
    let bytes = archive(&[(
        "content.xml",
        r#"<office:document xmlns:office="urn:o" xmlns:text="urn:t" xmlns:draw="urn:d"><draw:page><text:p>First</text:p></draw:page><draw:page/><draw:page><text:p>Last</text:p></draw:page></office:document>"#,
    )]);
    std::fs::write(&path, &bytes).unwrap();
    let mut doc = crate::files::Document::open(dir.path(), &path).unwrap();
    let office = doc.office.as_mut().unwrap();
    office.book = Some(parse::parse(&bytes, "odp").unwrap());
    let summary = office.gpui_poll_summary();
    let slides = summary.slides.as_ref().unwrap();
    assert_eq!(slides.len(), 3);
    assert_eq!(slides[0][0].1, "First");
    assert!(slides[1].is_empty());
    assert_eq!(slides[2][0].1, "Last");
    let window = cx.add_window(|_, cx| {
        let mut root =
            crate::gpui_ui::TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
        root.app.install_editor_input_fixture(vec![doc], 0);
        root
    });
    cx.refresh().unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let first = visual.debug_bounds("office-slide-1").unwrap();
    let blank = visual.debug_bounds("office-slide-2").unwrap();
    let last = visual.debug_bounds("office-slide-3").unwrap();
    assert!(first.bottom() < blank.top() && blank.bottom() < last.top());
    assert!(visual.debug_bounds("office-section-tabs").is_none());
}
#[test]
fn office_presentation_truncates_only_when_another_slide_exists() {
    for (count, expected) in [(0, false), (200, false), (201, true)] {
        let xml = format!(
            "<office:document xmlns:office='urn:o' xmlns:draw='urn:d'>{}</office:document>",
            "<draw:page/>".repeat(count)
        );
        let book = parse::parse(&archive(&[("content.xml", &xml)]), "odp").unwrap();
        assert_eq!(book.sections.len(), count.min(200));
        assert_eq!(book.truncated, expected);
    }
}
