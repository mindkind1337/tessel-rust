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
    assert_eq!(sheet.sheet.unwrap().rows[0][0], "Original value");
    view.select_section(1);
    let document = view.gpui_poll_summary();
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
