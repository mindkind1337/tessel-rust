//! Opaque local launch identity. Never persist command/environment text in it.
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub fn of(command: &str, args: &str, env: &BTreeMap<String, String>) -> String {
    let env: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let canonical = serde_json::to_vec(&(command, args, env)).expect("string tuple encodes");
    format!("{:x}", Sha256::digest(canonical))
}

pub fn for_agent(id: &str, prefs: &crate::agent_preferences::Preferences) -> Option<String> {
    let preset = crate::agent_catalog::find(id)?;
    let own = crate::agent_env::trim(&prefs.command);
    Some(of(
        if own.is_empty() { preset.command } else { own },
        crate::agent_env::trim(&prefs.args),
        &crate::agent_env::parse(&prefs.env).unwrap_or_default(),
    ))
}

pub fn valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_signature_hashes_match_and_effective_environment_controls_identity() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/launch_fingerprint.json"))
                .unwrap();
        for case in cases["cases"].as_array().unwrap() {
            let env = serde_json::from_value(case["input"]["env"].clone()).unwrap();
            let hash = of(
                case["input"]["command"].as_str().unwrap(),
                case["input"]["args"].as_str().unwrap(),
                &env,
            );
            assert_eq!(hash, case["expected"].as_str().unwrap());
            assert!(valid(&hash));
        }
        let mut prefs = crate::agent_preferences::Preferences::default();
        let plain = for_agent("codex", &prefs).unwrap();
        prefs.command = "  codex  ".into();
        assert_eq!(for_agent("codex", &prefs).unwrap(), plain);
        prefs.env = "B=two\nA=synthetic-secret".into();
        let first = for_agent("codex", &prefs).unwrap();
        prefs.env = "# comment\nA=synthetic-secret\nB=two".into();
        assert_eq!(for_agent("codex", &prefs).unwrap(), first);
        assert!(!first.contains("synthetic-secret"));
        prefs.env = "A=synthetic-changed\nB=two".into();
        assert_ne!(for_agent("codex", &prefs).unwrap(), first);
        prefs.env = "INVALID".into();
        assert_eq!(for_agent("codex", &prefs).unwrap(), plain);
        assert!(!valid("plaintext"));
        assert!(!valid(&"X".repeat(64)));
    }
}
