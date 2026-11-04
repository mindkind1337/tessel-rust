#[path = "../src/agent_resume.rs"]
mod agent_resume;
use agent_resume::*;

#[test]
fn synthetic_source_resume_comparisons() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/agent_resume.json")).unwrap();
    for case in data["cases"].as_array().unwrap() {
        let kind = case["kind"].as_str().unwrap();
        let id = case["id"].as_str().unwrap();
        let raw = case["target"].as_str().unwrap();
        let target = match raw {
            "unavailable" => Target::Unavailable,
            "missing" => Target::Missing,
            "verified" => Target::Verified {
                transcript_path: None,
            },
            path => Target::Verified {
                transcript_path: Some(path),
            },
        };
        assert_eq!(
            serde_json::to_value(resume_args(kind, id, target)).unwrap(),
            case["expected"],
            "{case}"
        );
    }
    for case in data["choices"].as_array().unwrap() {
        let id = case["id"].as_str();
        let resume = case["resume"].as_bool().unwrap();
        let exists = case["exists"].as_bool();
        let choice = claude_start_choice(id, resume, exists, || "fresh123".into());
        assert_eq!(choice.id, case["claude"]["id"].as_str().unwrap());
        assert_eq!(choice.resume, case["claude"]["resume"].as_bool().unwrap());
        assert_eq!(
            codex_resumes(id, resume, exists),
            case["codex"].as_bool().unwrap()
        );
    }
}

#[test]
fn ids_and_unverified_targets_are_rejected() {
    assert_eq!(OWN_ID_AGENTS.len(), 15);
    assert!(!safe_session_id("12345\n"));
    assert!(!safe_session_id("123456\0"));
    for kind in OWN_ID_AGENTS {
        assert!(resume_args(kind, "abc123", Target::Missing).is_none());
    }
}
