// Adapted from Tessel agentSessionOptions.js / Orca.
// Copyright (c) 2026 Lovecast Inc. MIT; see THIRD_PARTY_NOTICES.md.
//! Option-token semantics from shared/agentSessionOptions.js.
//! This tokenizer inspects flags; it is not a shell parser or an execution API.

pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut has = false;
    for ch in text.chars() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            has = true;
            continue;
        }
        if crate::agent_env::trim(&ch.to_string()).is_empty() {
            if has || !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            has = false;
        } else {
            current.push(ch);
        }
    }
    if has || !current.is_empty() {
        out.push(current);
    }
    out
}

pub fn option_tokens(tokens: &[String]) -> &[String] {
    &tokens[..tokens
        .iter()
        .position(|token| token == "--")
        .unwrap_or(tokens.len())]
}

fn attached(token: &str, flag: &str) -> bool {
    token.strip_prefix(flag).is_some_and(|rest| {
        rest.starts_with('=')
            || (!rest.is_empty() && flag.starts_with('-') && !flag.starts_with("--"))
    })
}

pub fn has_flag(tokens: &[String], flags: &[&str]) -> bool {
    option_tokens(tokens).iter().any(|token| {
        flags
            .iter()
            .any(|flag| token == flag || attached(token, flag))
    })
}

pub fn remove_option(tokens: &[String], aliases: &[&str]) -> Vec<String> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if token == "--" {
            result.extend_from_slice(&tokens[index..]);
            break;
        }
        let exact = aliases.contains(&token.as_str());
        if !exact && !aliases.iter().any(|alias| attached(token, alias)) {
            result.push(token.clone());
        } else if exact
            && tokens
                .get(index + 1)
                .is_some_and(|next| !next.is_empty() && !next.starts_with('-'))
        {
            index += 1;
        }
        index += 1;
    }
    result
}

fn codex_attached(token: &str) -> bool {
    [
        "-cmodel_reasoning_effort=",
        "-c=model_reasoning_effort=",
        "--config=model_reasoning_effort=",
    ]
    .iter()
    .any(|prefix| token.starts_with(prefix))
}

pub fn has_codex_effort(tokens: &[String]) -> bool {
    if has_flag(tokens, &["--reasoning-effort"]) {
        return true;
    }
    option_tokens(tokens).iter().enumerate().any(|(i, token)| {
        codex_attached(token)
            || (token.starts_with("model_reasoning_effort=")
                && i > 0
                && matches!(tokens[i - 1].as_str(), "-c" | "--config"))
    })
}

pub fn remove_codex_effort(tokens: &[String]) -> Vec<String> {
    let tokens = remove_option(tokens, &["--reasoning-effort"]);
    let mut result = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if token == "--" {
            result.extend_from_slice(&tokens[index..]);
            break;
        }
        if matches!(token.as_str(), "-c" | "--config")
            && tokens
                .get(index + 1)
                .is_some_and(|next| next.starts_with("model_reasoning_effort="))
        {
            index += 2;
            continue;
        }
        if !codex_attached(token) {
            result.push(token.clone());
        }
        index += 1;
    }
    result
}

pub fn safe_session_value(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=160).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/@[]=+-".contains(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_source_option_and_tokenization_cases() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/agent_arg_options.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let tokens: Vec<String> = serde_json::from_value(case["tokens"].clone()).unwrap();
            let aliases: Vec<String> = serde_json::from_value(case["aliases"].clone()).unwrap();
            let aliases: Vec<_> = aliases.iter().map(String::as_str).collect();
            let actual = serde_json::json!({
                "optionTokens": option_tokens(&tokens), "hasFlag": has_flag(&tokens, &aliases),
                "removed": remove_option(&tokens, &aliases), "codexEffort": has_codex_effort(&tokens),
                "withoutEffort": remove_codex_effort(&tokens)
            });
            assert_eq!(actual, case["expected"], "{}", case["name"]);
        }
        for case in fixture["tokenization"].as_array().unwrap() {
            assert_eq!(
                serde_json::json!(tokenize(case["input"].as_str().unwrap())),
                case["expected"]
            );
        }
        for case in fixture["values"].as_array().unwrap() {
            assert_eq!(
                safe_session_value(case["input"].as_str().unwrap()),
                case["expected"].as_bool().unwrap()
            );
        }
    }
}
