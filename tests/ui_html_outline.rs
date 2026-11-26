#[path = "../src/html_outline.rs"]
mod html_outline;
#[allow(dead_code)]
#[path = "../src/outline_tree.rs"]
mod outline_tree;

#[derive(serde::Deserialize)]
struct Case {
    text: String,
    expected: Vec<outline_tree::Symbol>,
}

#[test]
fn matches_installed_monaco_html_provider() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/html_outline.json")).unwrap();
    for c in cases {
        assert_eq!(
            html_outline::document_symbols(&c.text),
            c.expected,
            "{}",
            c.text
        );
    }
}
