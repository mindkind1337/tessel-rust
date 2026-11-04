//! Local terminal lifecycle; discovery and saved-conversation verification run off UI.
use crate::agent_preferences::Preferences;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

pub fn managed(agent: &str, prefs: &Preferences) -> bool {
    if agent != "codex" {
        return false;
    }
    let tokens = crate::agent_arg_options::tokenize(&format!("{} {}", prefs.command, prefs.args));
    !tokens.iter().any(|s| {
        matches!(
            s.as_str(),
            "resume"
                | "fork"
                | "exec"
                | "review"
                | "app-server"
                | "mcp-server"
                | "--help"
                | "-h"
                | "--version"
                | "-V"
        )
    })
}

pub fn root(
    prefs: &Preferences,
    cwd: &Path,
    home: &Path,
    inherited: Option<&str>,
) -> Result<PathBuf, String> {
    let env =
        crate::agent_env::parse(&prefs.env).map_err(|_| "Invalid Codex environment settings.")?;
    let chosen = crate::agent_env::effective_value(&env, "CODEX_HOME")
        .or(inherited)
        .filter(|p| !p.is_empty());
    Ok(match chosen {
        Some(path) if Path::new(path).is_absolute() => path.into(),
        Some(path) => cwd.join(path),
        None => home.join(".codex"),
    })
}

fn current_root(prefs: &Preferences, cwd: &Path) -> Result<PathBuf, String> {
    let dirs = directories::BaseDirs::new().ok_or("Cannot locate the agent's home directory.")?;
    root(
        prefs,
        cwd,
        dirs.home_dir(),
        std::env::var("CODEX_HOME").ok().as_deref(),
    )
}

pub fn prepare(prefs: &Preferences, cwd: &Path, id: &str) -> Result<Preferences, String> {
    if !managed("codex", prefs) {
        return Err("Conversation is managed by explicit Codex arguments.".into());
    }
    if !crate::codex_sessions::verify(&current_root(prefs, cwd)?, cwd, id)? {
        return Err("Saved Codex conversation was not found in this profile and folder; no replacement conversation was started.".into());
    }
    let mut effective = prefs.clone();
    effective.args = format!("resume {id} {}", prefs.args).trim_end().into();
    Ok(effective)
}

pub struct Discovery {
    root: PathBuf,
    cwd: PathBuf,
    since_ms: i64,
    next: Instant,
    pending: Option<Receiver<Result<Option<String>, String>>>,
}

impl Discovery {
    pub fn new(prefs: &Preferences, cwd: &Path) -> Result<Self, String> {
        Ok(Self {
            root: current_root(prefs, cwd)?,
            cwd: cwd.into(),
            since_ms: chrono::Utc::now().timestamp_millis(),
            next: Instant::now() + Duration::from_secs(2),
            pending: None,
        })
    }

    /// Only one scan per pane at a time, at most once every ten seconds. The
    /// caller checks claims again when accepting a result from this snapshot.
    pub fn poll(&mut self, excluded: &HashSet<String>) -> Option<Result<Option<String>, String>> {
        if let Some(pending) = &self.pending {
            let result = match pending.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Codex discovery stopped without a result.".into()))
                }
            };
            if result.is_some() {
                self.pending = None;
                self.next = Instant::now() + Duration::from_secs(10);
            }
            return result;
        }
        if Instant::now() < self.next {
            return None;
        }
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        let root = self.root.clone();
        let cwd = self.cwd.clone();
        let exclude = excluded.clone();
        let since_ms = self.since_ms;
        std::thread::spawn(move || {
            let result = crate::codex_sessions::find(
                &root,
                crate::codex_sessions::Query {
                    cwd: &cwd,
                    since_ms,
                    now_ms: chrono::Utc::now().timestamp_millis(),
                    exclude: &exclude,
                    latest: false,
                    active_since_ms: None,
                },
            );
            let _ = sender.send(result);
        });
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_modes_keep_ownership_and_environment_selects_the_profile() {
        let temp = tempfile::tempdir().unwrap();
        let mut prefs = Preferences::default();
        assert!(managed("codex", &prefs));
        assert!(!managed("claude", &prefs));
        for args in [
            "resume --last",
            "fork",
            "exec prompt",
            "review",
            "app-server",
            "--help",
        ] {
            prefs.args = args.into();
            assert!(!managed("codex", &prefs));
        }
        prefs.args = "--model synthetic".into();
        assert!(managed("codex", &prefs));
        prefs.env = "CODEX_HOME=profile".into();
        assert_eq!(
            root(&prefs, temp.path(), temp.path(), Some("ignored")).unwrap(),
            temp.path().join("profile")
        );
        prefs.env.clear();
        assert_eq!(
            root(&prefs, temp.path(), temp.path(), Some("inherited")).unwrap(),
            temp.path().join("inherited")
        );
        prefs.env = "invalid env".into();
        assert!(root(&prefs, temp.path(), temp.path(), None).is_err());
    }
}
