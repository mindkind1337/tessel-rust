//! Source-compatible parser for Settings > Agents environment text.
//! Parsing is all-or-nothing: callers never receive a partially valid environment.
use std::collections::BTreeMap;

pub const MAX_TEXT: usize = 16 * 1024;

// ECMAScript WhiteSpace + LineTerminator, deliberately not Rust's broader
// Unicode White_Space (which includes U+0085 but excludes U+FEFF).
fn js_space(c: char) -> bool {
    matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' |
        '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' |
        '\u{205f}' | '\u{3000}' | '\u{feff}')
}
pub(crate) fn trim(text: &str) -> &str {
    text.trim_matches(js_space)
}
fn excerpt(text: &str) -> String {
    String::from_utf16_lossy(&text.encode_utf16().take(40).collect::<Vec<_>>())
}

pub fn parse(text: &str) -> Result<BTreeMap<String, String>, String> {
    if text.encode_utf16().count() > MAX_TEXT {
        return Err("Environment text is too large to use safely.".into());
    }
    let mut env = BTreeMap::new();
    for raw in text.split('\n') {
        let line = trim(raw);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=').filter(|(name, _)| !name.is_empty()) else {
            return Err(format!("\"{}\" is not NAME=value.", excerpt(line)));
        };
        let name = trim(name);
        let valid = name
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_alphabetic() || b == b'_' || (i > 0 && b.is_ascii_digit()));
        if name.is_empty() || !valid {
            return Err(format!(
                "\"{}\" is not a valid variable name.",
                excerpt(name)
            ));
        }
        if name
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("TESSEL_"))
        {
            return Err(format!("{name} is set by Tessel and cannot be changed."));
        }
        // Object.prototype.__proto__ is an inherited setter in the source's
        // plain JS object: assigning a string does not create an env entry.
        if name != "__proto__" {
            env.insert(name.into(), trim(value).into());
        }
    }
    if env.len() > 50 {
        return Err("At most 50 variables.".into());
    }
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_source_environment_fixtures() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/agent_env.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let actual = match parse(case["input"].as_str().unwrap()) {
                Ok(env) => serde_json::json!({"env": env}),
                Err(error) => serde_json::json!({"error": error}),
            };
            assert_eq!(actual, case["expected"], "case {}", case["name"]);
        }
    }
    #[test]
    fn invalid_later_line_never_returns_partial_values() {
        assert!(parse("VALID=synthetic\nTESSEL_OWNER=synthetic").is_err());
        assert_eq!(
            parse("NAME=first\nNAME=last").unwrap().get("NAME").unwrap(),
            "last"
        );
    }
}
