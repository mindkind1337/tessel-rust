//! Suggested local filenames for downloads from the file explorer.
pub(crate) fn suggest(path: &str) -> String {
    let leaf = path.rsplit(['/', '\\']).next().unwrap_or_default();
    let sanitized = leaf.replace(
        |c: char| c.is_ascii_control() || r#"<>:"|?*"#.contains(c),
        "_",
    );
    let trimmed = sanitized.trim_end_matches(['.', ' ']);
    let stem = trimmed
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let numbered_device = stem
        .strip_prefix("com")
        .or_else(|| stem.strip_prefix("lpt"))
        .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit());
    let name = if trimmed.is_empty() {
        "download-file".into()
    } else if numbered_device || matches!(stem.as_str(), "con" | "prn" | "aux" | "nul") {
        format!("download-{trimmed}")
    } else {
        trimmed.to_owned()
    };
    let mut units = 0;
    name.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= 200
        })
        .collect()
}

#[test]
fn download_names_match_explorer_suggestions() {
    assert_eq!(suggest(r"docs\CON.csv. "), "download-CON.csv");
    assert_eq!(suggest("a/lpt0"), "download-lpt0");
    assert_eq!(suggest("a/..."), "download-file");
    assert_eq!(suggest("abc\t?.txt"), "abc__.txt");
    assert_eq!(suggest("com10.txt"), "com10.txt");
    assert_eq!(suggest(&format!("{}🚗", "a".repeat(199))), "a".repeat(199));
}
