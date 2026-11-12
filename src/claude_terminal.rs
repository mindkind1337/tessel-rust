//! Claude-compatible terminal conversation lifecycle. No transcript contents are read.
use crate::agent_preferences::Preferences;
use std::{
    io,
    path::{Path, PathBuf},
};

pub fn supported(agent: &str) -> bool {
    matches!(agent, "claude" | "openclaude")
}

/// Explicit user-authored session flags retain ownership of the lifecycle.
pub fn managed(agent: &str, prefs: &Preferences) -> bool {
    supported(agent)
        && !crate::agent_arg_options::has_flag(
            &crate::agent_arg_options::tokenize(&format!("{} {}", prefs.command, prefs.args)),
            &["--session-id", "--resume", "-r", "--continue", "-c"],
        )
}

pub fn root(
    agent: &str,
    prefs: &Preferences,
    cwd: &Path,
    home: &Path,
    inherited: Option<&str>,
) -> PathBuf {
    let env = crate::agent_env::parse(&prefs.env).unwrap_or_default();
    let override_path = crate::agent_env::effective_value(&env, "CLAUDE_CONFIG_DIR")
        .or(inherited)
        .filter(|s| !s.is_empty());
    if let Some(path) = override_path {
        let path = PathBuf::from(path);
        return if path.is_absolute() {
            path
        } else {
            cwd.join(path)
        };
    }
    home.join(if agent == "openclaude" {
        ".openclaude"
    } else {
        ".claude"
    })
}

pub fn transcript_path(root: &Path, id: &str) -> Result<Option<PathBuf>, String> {
    if !crate::claude::valid_uuid(id) {
        return Err("Invalid terminal conversation identifier.".into());
    }
    let entries = match std::fs::read_dir(root.join("projects")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cannot inspect the agent's conversation folder.".into()),
    };
    for (count, entry) in entries.enumerate() {
        if count >= 100_000 {
            return Err("Conversation folder exceeds the lookup limit.".into());
        }
        let entry = entry.map_err(|_| "Cannot enumerate agent conversations.")?;
        if !entry
            .file_type()
            .map_err(|_| "Cannot inspect agent conversation metadata.")?
            .is_dir()
        {
            continue;
        }
        let candidate = entry.path().join(format!("{id}.jsonl"));
        match std::fs::symlink_metadata(&candidate) {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
                return Ok(Some(candidate));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot verify the saved agent conversation.".into()),
        }
    }
    Ok(None)
}

pub fn exists(root: &Path, id: &str) -> Result<bool, String> {
    transcript_path(root, id).map(|p| p.is_some())
}

pub fn with_session(prefs: &Preferences, id: &str, exists: bool) -> Result<Preferences, String> {
    if !crate::claude::valid_uuid(id) {
        return Err("Invalid terminal conversation identifier.".into());
    }
    let mut effective = prefs.clone();
    // Place our flags before authored arguments so a literal -- cannot hide them.
    effective.args = format!(
        "{} {id} {}",
        if exists { "--resume" } else { "--session-id" },
        prefs.args
    )
    .trim_end()
    .into();
    Ok(effective)
}

#[allow(dead_code)]
pub fn prepare(
    agent: &str,
    prefs: &Preferences,
    cwd: &Path,
    id: &str,
) -> Result<Preferences, String> {
    prepare_with_existing(agent, prefs, cwd, id, false)
}

pub fn prepare_with_existing(
    agent: &str,
    prefs: &Preferences,
    cwd: &Path,
    id: &str,
    require_existing: bool,
) -> Result<Preferences, String> {
    if !managed(agent, prefs) {
        return Err("Conversation is managed by explicit agent arguments.".into());
    }
    let dirs = directories::BaseDirs::new().ok_or("Cannot locate the agent's home directory.")?;
    let inherited = std::env::var("CLAUDE_CONFIG_DIR").ok();
    let root = root(agent, prefs, cwd, dirs.home_dir(), inherited.as_deref());
    let found = exists(&root, id)?;
    if require_existing && (!found || !cwd.is_dir()) {
        return Err(
            "Saved conversation or folder is unavailable; no replacement conversation was started."
                .into(),
        );
    }
    with_session(prefs, id, found)
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "12345678-1234-4234-8234-123456789abc";
    #[test]
    fn transcript_presence_controls_resume_without_reading_contents() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!exists(dir.path(), ID).unwrap());
        let project = dir.path().join("projects").join("synthetic-project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(format!("{ID}.jsonl")), "not read or parsed").unwrap();
        assert!(exists(dir.path(), ID).unwrap());
        let prefs = Preferences {
            args: "--model synthetic -- literal".into(),
            ..Default::default()
        };
        assert_eq!(
            with_session(&prefs, ID, false).unwrap().args,
            format!("--session-id {ID} --model synthetic -- literal")
        );
        assert_eq!(
            with_session(&prefs, ID, true).unwrap().args,
            format!("--resume {ID} --model synthetic -- literal")
        );
        assert!(!exists(dir.path(), "22345678-1234-4234-8234-123456789abc").unwrap());
        assert!(exists(dir.path(), "../../other").is_err());
        assert!(with_session(&prefs, "--help", true).is_err());
    }
    #[test]
    fn manual_session_flags_and_root_overrides_keep_their_scope() {
        let mut prefs = Preferences::default();
        assert!(managed("claude", &prefs));
        assert!(managed("openclaude", &prefs));
        assert!(!managed("codex", &prefs));
        for args in ["--session-id x", "--resume=x", "-rabc", "--continue", "-c"] {
            prefs.args = args.into();
            assert!(!managed("claude", &prefs), "{args}");
        }
        prefs.args.clear();
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            root("openclaude", &prefs, dir.path(), dir.path(), None),
            dir.path().join(".openclaude")
        );
        prefs.env = "CLAUDE_CONFIG_DIR=profile".into();
        assert_eq!(
            root("claude", &prefs, dir.path(), dir.path(), Some("ignored")),
            dir.path().join("profile")
        );
        prefs.env = "invalid env line".into();
        assert_eq!(
            root("claude", &prefs, dir.path(), dir.path(), Some("inherited")),
            dir.path().join("inherited")
        );
    }
    #[test]
    fn required_resume_never_creates_a_replacement_conversation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("profile");
        let prefs = Preferences {
            env: format!("CLAUDE_CONFIG_DIR={}", root.display()),
            ..Default::default()
        };
        let id = "12345678-1234-1234-1234-123456789abc";
        assert!(prepare_with_existing("claude", &prefs, temp.path(), id, true).is_err());
        let project = root.join("projects/synthetic");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(format!("{id}.jsonl")), "{}\n").unwrap();
        let resumed = prepare_with_existing("claude", &prefs, temp.path(), id, true).unwrap();
        assert_eq!(resumed.args, format!("--resume {id}"));
        assert!(!resumed.args.contains("--session-id"));
        assert!(
            prepare_with_existing("claude", &prefs, &temp.path().join("missing"), id, true)
                .is_err()
        );
    }
}
