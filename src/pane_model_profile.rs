//! Local, memory-only model lookup scope captured from the terminal command.
//! Never serialized, logged, exported or sent to guests. It contains no complete
//! environment or command line; only the selected provider root and working dir.
use portable_pty::CommandBuilder;
use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Eq)]
pub struct Profile {
    agent: String,
    cwd: PathBuf,
    root: PathBuf,
}

impl Profile {
    /// Capture on the launch worker after preferences have been applied. The
    /// builder contains the environment actually passed to the child, including
    /// inherited values. Later Settings edits cannot change this snapshot.
    /// Arbitrary shell wrappers may override it internally: callers must not
    /// attach a profile to such launches without a provider handshake.
    pub fn capture(agent: &str, command: &CommandBuilder, home: &Path) -> Option<Self> {
        let (variable, default) = match agent {
            "codex" => ("CODEX_HOME", ".codex"),
            "claude" => ("CLAUDE_CONFIG_DIR", ".claude"),
            "openclaude" => ("CLAUDE_CONFIG_DIR", ".openclaude"),
            _ => return None,
        };
        let cwd = PathBuf::from(command.get_cwd()?);
        if !cwd.is_absolute() || !home.is_absolute() {
            return None;
        }
        let root = command
            .get_env(variable)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    cwd.join(path)
                }
            })
            .unwrap_or_else(|| home.join(default));
        Some(Self {
            agent: agent.into(),
            cwd,
            root,
        })
    }

    pub fn agent(&self) -> &str {
        &self.agent
    }
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_uses_the_child_environment_and_survives_later_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = CommandBuilder::new("synthetic-agent");
        command.env_clear();
        command.cwd(dir.path());
        command.env("CODEX_HOME", "profile-a");
        command.env("UNRELATED_PRIVATE_VALUE", "not-retained");
        let profile = Profile::capture("codex", &command, dir.path()).unwrap();
        command.env("CODEX_HOME", "profile-b");
        command.cwd(dir.path().join("different"));
        assert_eq!(profile.root(), dir.path().join("profile-a"));
        assert_eq!(profile.cwd(), dir.path());
        assert_eq!(profile.agent(), "codex");
        let other = Profile::capture("codex", &command, dir.path()).unwrap();
        assert!(other != profile);
        command.env("CODEX_HOME", dir.path().join("absolute-profile"));
        assert_eq!(
            Profile::capture("codex", &command, dir.path())
                .unwrap()
                .root(),
            dir.path().join("absolute-profile")
        );
    }

    #[test]
    fn defaults_are_provider_specific_and_unknown_scopes_are_not_guessed() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = CommandBuilder::new("synthetic-agent");
        command.env_clear();
        command.cwd(dir.path());
        for (agent, folder) in [
            ("codex", ".codex"),
            ("claude", ".claude"),
            ("openclaude", ".openclaude"),
        ] {
            assert_eq!(
                Profile::capture(agent, &command, dir.path())
                    .unwrap()
                    .root(),
                dir.path().join(folder)
            );
        }
        command.env("CLAUDE_CONFIG_DIR", "claude-profile");
        for agent in ["claude", "openclaude"] {
            assert_eq!(
                Profile::capture(agent, &command, dir.path())
                    .unwrap()
                    .root(),
                dir.path().join("claude-profile")
            );
        }
        assert!(Profile::capture("shell", &command, dir.path()).is_none());
        command.clear_cwd();
        assert!(Profile::capture("codex", &command, dir.path()).is_none());
    }
}
