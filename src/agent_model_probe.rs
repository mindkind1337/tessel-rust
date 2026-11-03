// Adapted from Tessel shared/agentModelProbe.js / Orca.
// Copyright (c) 2026 Lovecast Inc. MIT; see THIRD_PARTY_NOTICES.md.
//! Non-interactive model queries. Muse requires its protocol, never a bare serve capture.
use crate::{
    agent_env::trim,
    agent_model_probe_parse::parse,
    agent_model_process::{self, Limits, Output},
};
use serde_json::Value;

pub const CLAUDE_STDIN: &str = "{\"type\":\"control_request\",\"request_id\":\"orca-model-discovery\",\"request\":{\"subtype\":\"list_models\"}}\n";
pub struct Spec {
    pub exe: &'static str,
    pub args: &'static [&'static str],
    pub stdin: Option<&'static str>,
    pub protocol: Option<&'static str>,
}
pub fn spec(agent: &str) -> Option<Spec> {
    let (exe, args): (&str, &[&str]) = match agent {
        "claude" => (
            "claude",
            &[
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
            ],
        ),
        "codex" => ("codex", &["debug", "models"]),
        "grok" => ("grok", &["models"]),
        "opencode" => ("opencode", &["models"]),
        "pi" => ("pi", &["--list-models"]),
        "cursor" => ("cursor-agent", &["--list-models"]),
        "antigravity" => ("agy", &["models"]),
        "muse" => ("muse", &["serve"]),
        _ => return None,
    };
    Some(Spec {
        exe,
        args,
        stdin: (agent == "claude").then_some(CLAUDE_STDIN),
        protocol: (agent == "muse").then_some("model/list"),
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    NotFound,
    Unsupported,
    ProtocolRequired,
    Process(agent_model_process::Error),
    Failed(String),
    Empty,
}

/// `command` is a resolved executable and optional wrapper prefix. Only fixed
/// discovery arguments are appended; saved interactive/custom arguments are not.
pub fn execute(agent: &str, mut command: tokio::process::Command) -> Result<Vec<Value>, Failure> {
    let spec = spec(agent).ok_or(Failure::Unsupported)?;
    if spec.protocol.is_some() {
        return Err(Failure::ProtocolRequired);
    }
    command.args(spec.args);
    let output = agent_model_process::run(command, spec.stdin, Limits::default())
        .map_err(Failure::Process)?;
    finalize(agent, &output)
}

pub fn finalize(agent: &str, output: &Output) -> Result<Vec<Value>, Failure> {
    spec(agent).ok_or(Failure::Unsupported)?;
    if output.code != Some(0) {
        let text = if output.stderr.is_empty() {
            &output.stdout
        } else {
            &output.stderr
        };
        let lines: Vec<_> = trim(text)
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .filter(|line| !line.is_empty())
            .collect();
        let last = lines[lines.len().saturating_sub(2)..].join(" ");
        let code = output.code.map_or_else(|| "null".into(), |v| v.to_string());
        let detail = format!("{code}: {last}");
        let units: Vec<_> = detail.encode_utf16().take(300).collect();
        return Err(Failure::Failed(String::from_utf16_lossy(&units)));
    }
    let mut models = parse(agent, &output.stdout);
    if models.is_empty() && !trim(&output.stderr).is_empty() {
        models = parse(agent, &output.stderr);
    }
    if models.is_empty() {
        Err(Failure::Empty)
    } else {
        Ok(models)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn listing_specs_are_noninteractive_and_muse_requires_protocol() {
        let claude = spec("claude").unwrap();
        assert_eq!(
            claude.args,
            [
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose"
            ]
        );
        let request: Value = serde_json::from_str(claude.stdin.unwrap()).unwrap();
        assert_eq!(request["request"]["subtype"], "list_models");
        assert_eq!(spec("cursor").unwrap().exe, "cursor-agent");
        assert_eq!(
            execute("muse", tokio::process::Command::new("must-not-run")),
            Err(Failure::ProtocolRequired)
        );
        assert_eq!(
            execute("unknown", tokio::process::Command::new("must-not-run")),
            Err(Failure::Unsupported)
        );
    }
    #[test]
    fn completion_preserves_stderr_fallback_exit_failure_and_empty_semantics() {
        let mut output = Output {
            stdout: String::new(),
            stderr: "provider/model\n".into(),
            code: Some(0),
        };
        assert_eq!(
            finalize("opencode", &output).unwrap()[0]["id"],
            "provider/model"
        );
        output.stdout = "first/model\n".into();
        assert_eq!(
            finalize("opencode", &output).unwrap()[0]["id"],
            "first/model"
        );
        output.code = Some(7);
        output.stderr = "  old\r\nsecond\r\n\r\nlast\n ".into();
        assert_eq!(
            finalize("opencode", &output),
            Err(Failure::Failed("7: second last".into()))
        );
        output.code = None;
        output.stderr = "x".repeat(500);
        let Err(Failure::Failed(detail)) = finalize("codex", &output) else {
            panic!("expected exit failure")
        };
        assert_eq!(detail.len(), 300);
        assert!(detail.starts_with("null: "));
        output.code = Some(0);
        output.stdout.clear();
        output.stderr.clear();
        assert_eq!(finalize("codex", &output), Err(Failure::Empty));
    }
}
