#[path = "../src/css_outline.rs"]
mod css_outline;
#[allow(dead_code)]
#[path = "../src/outline_tree.rs"]
mod outline_tree;

#[derive(serde::Deserialize)]
struct Case {
    #[serde(default)]
    language: String,
    text: String,
    expected: Vec<outline_tree::Symbol>,
}

#[test]
fn matches_installed_monaco_css_provider() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/css_outline.json")).unwrap();
    let dialects: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/stylesheet_dialects.json")).unwrap();
    for c in cases.into_iter().chain(dialects) {
        let language = match c.language.as_str() {
            "scss" => css_outline::Language::Scss,
            "less" => css_outline::Language::Less,
            _ => css_outline::Language::Css,
        };
        assert_eq!(
            css_outline::document_symbols(&c.text, language),
            c.expected,
            "{}",
            c.text
        );
    }
}
