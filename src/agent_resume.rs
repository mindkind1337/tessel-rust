//! Pure resume planning. A caller must verify the conversation on its owning host.
//! This module performs no transcript lookup and starts no process.

pub const OWN_ID_AGENTS: &[&str] = &[
    "opencode",
    "cline",
    "copilot",
    "kimi",
    "droid",
    "grok",
    "pi",
    "omp",
    "antigravity",
    "devin",
    "cursor",
    "zcode",
    "qoder",
    "dsh",
    "muse",
];

pub fn safe_session_id(id: &str) -> bool {
    (6..=80).contains(&id.len())
        && id
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_alphanumeric() || b == b'_' || (i > 0 && b == b'-'))
}

/// Lookup outcome, distinct from an older backend with no lookup capability.
#[derive(Clone, Copy, Debug)]
pub enum Target<'a> {
    Unavailable,
    Missing,
    Verified { transcript_path: Option<&'a str> },
}

pub fn resume_args(kind: &str, id: &str, target: Target<'_>) -> Option<Vec<String>> {
    if !OWN_ID_AGENTS.contains(&kind) || !safe_session_id(id) {
        return None;
    }
    let path = match target {
        Target::Missing => return None,
        Target::Unavailable if !["opencode", "cline", "copilot", "kimi"].contains(&kind) => {
            return None;
        }
        Target::Unavailable => None,
        Target::Verified { transcript_path } => transcript_path,
    };
    let args = match kind {
        "cline" => vec!["--id".into(), id.into()],
        "opencode" | "kimi" => vec!["--session".into(), id.into()],
        "antigravity" => vec!["--conversation".into(), id.into()],
        "copilot" => vec![format!("--resume={id}")],
        "pi" => vec!["--session".into(), path.filter(|p| !p.is_empty())?.into()],
        "omp" => vec![
            "--resume".into(),
            path.map(str::trim)
                .filter(|p| !p.is_empty())
                .unwrap_or(id)
                .into(),
        ],
        "muse" => vec!["resume".into(), id.into()],
        _ => vec!["--resume".into(), id.into()],
    };
    args.iter()
        .all(|a: &String| {
            !a.is_empty()
                && a.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.:=/-".contains(&b))
        })
        .then_some(args)
}

#[derive(Debug, PartialEq, Eq)]
pub struct ClaudeStart {
    pub resume: bool,
    pub id: String,
}

/// Mirrors source choice policy; ID validation belongs at the launch boundary.
pub fn claude_start_choice(
    id: Option<&str>,
    resume: bool,
    exists: Option<bool>,
    new_id: impl FnOnce() -> String,
) -> ClaudeStart {
    if let Some(id) = id.filter(|s| !s.is_empty()) {
        if resume && exists != Some(false) {
            return ClaudeStart {
                resume: true,
                id: id.into(),
            };
        }
        if exists == Some(false) {
            return ClaudeStart {
                resume: false,
                id: id.into(),
            };
        }
    }
    ClaudeStart {
        resume: false,
        id: new_id(),
    }
}

pub fn codex_resumes(id: Option<&str>, resume: bool, exists: Option<bool>) -> bool {
    id.is_some_and(|s| !s.is_empty()) && resume && exists != Some(false)
}
