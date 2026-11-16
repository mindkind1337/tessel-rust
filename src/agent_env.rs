//! Source-compatible parser for Settings > Agents environment text.
//! Parsing is all-or-nothing: callers never receive a partially valid environment.
use std::collections::BTreeMap;

// Keep in sync with Vue src/main/cleanEnv.js. These identify the launching
// session, not user account configuration. Rust's own parent broker capability
// is also excluded; a fresh pane binding may be installed afterwards.
const SESSION_VARS: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING",
    "CLAUDE_CODE_ENABLE_TASKS",
    "CLAUDE_CODE_SSE_PORT",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "CLAUDE_AGENT_SDK_VERSION",
    "MCP_CONNECTION_NONBLOCKING",
    "CODEX_SANDBOX",
    "CODEX_SANDBOX_NETWORK_DISABLED",
    "ELECTRON_RUN_AS_NODE",
    "TESSEL_PTYHOST_PIPE",
    "TESSEL_PTYHOST_TOKEN",
    "TESSEL_LOG_DIR",
    "TESSEL_TEAM_MCP_PANE",
    "TESSEL_TEAM_MCP_GENERATION",
    "TESSEL_TEAM_MCP_KEY",
];

fn parent_session_var(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        SESSION_VARS
            .iter()
            .any(|key| name.eq_ignore_ascii_case(key))
    })
}

/// Remove inherited and explicitly configured parent-session keys, without
/// retaining or logging their values. Apply before fresh pane/account bindings.
#[allow(dead_code)] // This parser is also compiled by standalone UI fixtures.
pub fn clean_command(command: &mut std::process::Command) {
    let keys: Vec<_> = std::env::vars_os()
        .map(|(key, _)| key)
        .chain(command.get_envs().map(|(key, _)| key.to_owned()))
        .filter(|key| parent_session_var(key))
        .collect();
    for key in keys {
        command.env_remove(key);
    }
}

/// Filter the PTY's initial environment before caller preferences and bindings.
/// Keep portable-pty's registry refresh (Windows) and SHELL fallback (Unix).
#[allow(dead_code)]
pub fn inherit_clean_pty(command: &mut portable_pty::CommandBuilder) {
    remove_pty_parent_keys(command, std::env::vars_os().map(|(key, _)| key));
}
fn remove_pty_parent_keys(
    command: &mut portable_pty::CommandBuilder,
    inherited_keys: impl IntoIterator<Item = std::ffi::OsString>,
) {
    // Enumerate names, never Unicode-filtered values: a private value may be
    // non-Unicode. Constants additionally cover Windows registry-only entries;
    // portable-pty's Windows key lookup is case-insensitive.
    for key in inherited_keys
        .into_iter()
        .filter(|key| parent_session_var(key))
    {
        command.env_remove(key);
    }
    for key in SESSION_VARS {
        command.env_remove(key);
    }
}

#[cfg(test)]
fn set_clean_pty(
    command: &mut portable_pty::CommandBuilder,
    env: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) {
    let mut keys = Vec::new();
    for (key, value) in env {
        command.env(&key, value);
        keys.push(key);
    }
    remove_pty_parent_keys(command, keys);
}

pub const MAX_TEXT: usize = 16 * 1024;

/// Match the environment actually passed by launch.rs to portable-pty: Windows
/// keys are case-insensitive and the last BTreeMap entry applied wins.
pub fn effective_value<'a>(env: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    if cfg!(windows) {
        env.iter()
            .rev()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    } else {
        env.get(name).map(String::as_str)
    }
}

// ECMAScript WhiteSpace + LineTerminator, deliberately not Rust's broader
// Unicode White_Space (which includes U+0085 but excludes U+FEFF).
pub(crate) fn js_space(c: char) -> bool {
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

#[allow(dead_code)]
#[must_use]
pub fn invalid_env_unused_note(french: bool) -> &'static str {
    if french {
        "Non utilisées tant que ce n'est pas corrigé."
    } else {
        "Not used until fixed."
    }
}

#[allow(dead_code)]
#[must_use]
pub fn format_invalid_env_callout(err: &str, french: bool) -> String {
    format!("{err} {}", invalid_env_unused_note(french))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean_parent_environment_preserves_user_configuration() {
        let mut command = std::process::Command::new("never-executed");
        let mut env = Vec::new();
        for key in SESSION_VARS {
            for key in [key.to_string(), key.to_ascii_lowercase()] {
                command.env(&key, "synthetic-private");
                env.push((key.into(), "synthetic-private".into()));
            }
        }
        let keep = [
            "PATH",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "HTTPS_PROXY",
            "CLAUDE_CODE_USE_BEDROCK",
            "CODEX_HOME",
            "CLAUDE_CONFIG_DIR",
            "TESSEL_CUSTOM",
        ];
        for key in keep {
            command.env(key, "synthetic-user");
            env.push((key.into(), "synthetic-user".into()));
        }
        clean_command(&mut command);
        let mut pty = portable_pty::CommandBuilder::new("never-executed");
        pty.env("SYSTEM_REFRESH_FIXTURE", "synthetic-system");
        set_clean_pty(&mut pty, env);
        assert_eq!(
            pty.get_env("SYSTEM_REFRESH_FIXTURE"),
            Some(std::ffi::OsStr::new("synthetic-system"))
        );
        for (key, value) in command.get_envs() {
            if parent_session_var(key) {
                assert!(value.is_none());
            }
        }
        for key in SESSION_VARS {
            assert!(pty.get_env(key).is_none());
            assert!(pty.get_env(key.to_ascii_lowercase()).is_none());
        }
        for key in keep {
            assert_eq!(
                pty.get_env(key),
                Some(std::ffi::OsStr::new("synthetic-user"))
            );
            assert!(
                command
                    .get_envs()
                    .any(|(k, v)| k == key && v == Some(std::ffi::OsStr::new("synthetic-user")))
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn clean_parent_environment_preserves_non_unicode_user_values() {
        use std::os::unix::ffi::OsStringExt;
        let value = std::ffi::OsString::from_vec(vec![0xff, 0xfe]);
        let mut command = portable_pty::CommandBuilder::new("never-executed");
        set_clean_pty(
            &mut command,
            [
                ("claude_code_messaging_token".into(), value.clone()),
                ("USER_DEFINED".into(), value.clone()),
            ],
        );
        assert!(command.get_env("claude_code_messaging_token").is_none());
        assert_eq!(command.get_env("USER_DEFINED"), Some(value.as_os_str()));
    }

    #[test]
    fn profile_override_matches_the_native_child_environment() {
        let env = parse(
            "CODEX_HOME=first\ncodex_home=last\nCLAUDE_CONFIG_DIR=one\nclaude_config_dir=two",
        )
        .unwrap();
        let mut command = portable_pty::CommandBuilder::new("synthetic-never-executed");
        command.env_clear();
        for (key, value) in &env {
            command.env(key, value);
        }
        for key in ["CODEX_HOME", "CLAUDE_CONFIG_DIR"] {
            assert_eq!(
                effective_value(&env, key),
                command.get_env(key).and_then(|v| v.to_str())
            );
        }
    }
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
