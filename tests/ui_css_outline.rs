#[path = "../src/css_outline.rs"]
mod css_outline;
#[allow(dead_code)]
#[path = "../src/outline_tree.rs"]
mod outline_tree;

#[derive(serde::Deserialize)]
struct Case {
    text: String,
    expected: Vec<outline_tree::Symbol>,
}

#[test]
fn matches_installed_monaco_css_provider() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/css_outline.json")).unwrap();
    for c in cases {
        assert_eq!(
            css_outline::document_symbols(&c.text),
            c.expected,
            "{}",
            c.text
        );
    }
}
