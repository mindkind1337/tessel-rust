#[allow(dead_code)]
#[path = "../src/outline_tree.rs"]
mod outline_tree;
#[path = "../src/script_outline.rs"]
mod script_outline;

#[derive(serde::Deserialize)]
struct Case {
    extension: String,
    text: String,
    expected: Vec<outline_tree::Symbol>,
}
#[test]
fn matches_installed_typescript_worker() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/script_outline.json")).unwrap();
    for c in cases {
        assert_eq!(
            script_outline::document_symbols(&c.text, c.extension == "ts"),
            c.expected,
            "{}",
            c.text
        );
    }
}

#[test]
fn isolated_worker_reads_the_draft_and_emits_original_source_ranges() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/script_outline.json")).unwrap();
    for c in cases
        .iter()
        .filter(|c| c.text.contains("Café") || c.text.contains("class Box"))
    {
        let mut child = Command::new(env!("CARGO_BIN_EXE_tessel-rust"))
            .args(["--script-outline-worker", &c.extension])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(c.text.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        let symbols: Vec<outline_tree::Symbol> = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(symbols, c.expected);
    }
}
