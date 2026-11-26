#[path = "../src/json_outline.rs"]
mod json_outline;
#[allow(dead_code)]
#[path = "../src/outline_tree.rs"]
mod outline_tree;

#[derive(serde::Deserialize)]
struct Case {
    text: String,
    expected: Vec<outline_tree::Symbol>,
}

#[test]
fn matches_installed_monaco_json_provider() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/json_outline.json")).unwrap();
    for c in cases {
        assert_eq!(
            json_outline::document_symbols(&c.text),
            c.expected,
            "{}",
            c.text
        );
    }
}
#[test]
fn editor_offset_after_crlf_and_astral_character() {
    assert_eq!(json_outline::char_offset("a\r\n😀x", 2, 3), 4);
}
