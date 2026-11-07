//! Integration tests for Phase B (`Lot b175`): syntax token-span extraction across all 7 themes,
//! rich-preview Markdown/Mermaid/Sheet/JSONL models, and headless PTY OAuth device-login detection.

#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

#[allow(dead_code)]
#[path = "../src/theme.rs"]
mod theme;

#[allow(dead_code)]
#[path = "../src/i18n.rs"]
mod i18n;

#[allow(dead_code)]
#[path = "../src/syntax.rs"]
mod syntax;

#[allow(dead_code)]
#[path = "../src/rich_previews.rs"]
mod rich_previews;

#[allow(dead_code)]
#[path = "../src/device_login.rs"]
mod device_login;

#[test]
fn syntax_tokenize_spans_produces_ordered_non_overlapping_utf8_ranges() {
    let sample = r#"pub fn greet(name: &str) -> String { format!("Bonjour {name}") } // Québec"#;
    let spans = syntax::tokenize_spans(sample, syntax::SyntaxLang::Rust);
    assert!(!spans.is_empty());
    let mut prev_end = 0usize;
    for (start, end, _kind) in &spans {
        assert!(*start >= prev_end);
        assert!(*end > *start);
        assert!(sample.is_char_boundary(*start));
        assert!(sample.is_char_boundary(*end));
        prev_end = *end;
    }
    assert!(
        spans
            .iter()
            .any(|(_, _, k)| *k == syntax::TokenKind::Keyword)
    );
    assert!(
        spans
            .iter()
            .any(|(_, _, k)| *k == syntax::TokenKind::String)
    );
    assert!(
        spans
            .iter()
            .any(|(_, _, k)| *k == syntax::TokenKind::Comment)
    );

    for theme in theme_palettes::THEMES {
        let pal = syntax::SyntaxPalette::for_theme_mode(theme, true);
        assert_ne!(pal.keyword, pal.comment);
    }
}

#[test]
fn rich_previews_parse_markdown_mermaid_csv_and_jsonl_for_gpui_stage() {
    let md = "# Architecture\n\n```mermaid\nflowchart TD\n  UI[GPUI] --> Core[Tessel]\n```\n";
    assert_eq!(
        rich_previews::classify_markdown_size(md.len()),
        rich_previews::MarkdownSizeDecision::AutoRender
    );
    let segments = rich_previews::extract_markdown_segments(md);
    assert_eq!(segments.len(), 2);

    let sheet = rich_previews::parse_delimited_sheet("col_a;col_b\n1;2\n3;4\n", None);
    assert_eq!(sheet.delimiter_label, "CSV (;)");
    assert_eq!(sheet.rows.len(), 3);
    assert_eq!(rich_previews::sheet_col_letter(0), "A");
    assert_eq!(rich_previews::sheet_col_letter(27), "AB");

    let jsonl =
        rich_previews::parse_json_or_jsonl("{\"a\":1}\n{\"b\":2}\n", "jsonl").expect("valid jsonl");
    assert!(jsonl.is_jsonl);
    assert_eq!(jsonl.total_records, 2);
    assert_eq!(jsonl.invalid_lines, 0);
}

#[test]
fn headless_device_login_detector_debounces_and_extracts_oauth_prompt() {
    let mut det = device_login::DeviceLoginDetector::default();
    let screen = "To authenticate your CLI session:\nOpen this link: https://github.com/login/device\nEnter code: ABCD-EFGH\n";

    // Initial tick arms the 400 ms debounce without immediately surfacing the banner:
    let wait = det.tick_headless(1, 80, || screen.to_owned());
    assert!(wait.is_some());
    assert!(det.active_prompt().is_none());

    std::thread::sleep(std::time::Duration::from_millis(430));
    let wait_after = det.tick_headless(1, 80, || screen.to_owned());
    assert!(wait_after.is_none());
    let prompt = det.active_prompt().expect("debounced OAuth prompt");
    assert_eq!(prompt.url, "https://github.com/login/device");
    assert_eq!(prompt.host, "github.com");
    assert!(prompt.known_host);
    assert_eq!(prompt.device_code.as_deref(), Some("ABCD-EFGH"));

    det.dismiss_current();
    assert!(det.active_prompt().is_none());
}
