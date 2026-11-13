#[path = "../src/share/protected_paths.rs"]
mod protected_paths;
#[test]
fn built_in_policy_matches_vue_reference_vectors() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/protected_paths_reference.json")).unwrap();
    assert_eq!(data["sha256"].as_str().unwrap().len(), 64);
    for case in data["cases"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let extra = case["extra"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            protected_paths::is_protected(input, &extra),
            case["expected"].as_bool().unwrap(),
            "{input:?} {extra:?}"
        );
    }
}
#[test]
fn ci_extensions_and_exact_pattern_boundaries_are_explicit() {
    for input in [
        ".github/workflows/test.yml",
        "folder/.gitea/workflows/test.yml",
        ".woodpecker/task.yml",
        ".woodpecker.yaml",
        "Jenkinsfile",
        ".circleci/config.yml",
    ] {
        assert!(protected_paths::is_protected(input, &[]), "{input}");
    }
    for input in [
        "Jenkinsfile.md",
        "Jenkinsfile.txt",
        "Jenkinsfile.rst",
        "Jenkinsfile.adoc",
        "commitlint-notes.txt",
        "lint-staged.config.json",
        ".lintstagedrcbackup",
        ".codex-backup/config.json",
        "README.md",
    ] {
        assert!(!protected_paths::is_protected(input, &[]), "{input}");
    }
}
