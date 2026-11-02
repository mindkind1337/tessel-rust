use crate::workspace::{Host, Result, validate_host};
use portable_pty::CommandBuilder;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Launch {
    Shell,
    Agent(String),
    Ssh(Host),
}

pub const AGENTS: &[&str] = &[
    "claude",
    "codex",
    "opencode",
    "gemini",
    "copilot",
    "aider",
    "amp",
    "kimi",
    "qwen",
    "grok",
    "cline",
    "cursor-agent",
];

pub fn installed(program: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        if cfg!(windows) {
            ["exe", "cmd", "bat", "ps1"]
                .iter()
                .any(|ext| dir.join(format!("{program}.{ext}")).is_file())
        } else {
            dir.join(program).is_file()
        }
    })
}

pub fn posix_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

pub fn command(launch: &Launch, cwd: &Path) -> Result<CommandBuilder> {
    let mut command = match launch {
        Launch::Shell => {
            let mut c = CommandBuilder::new(if cfg!(windows) {
                "powershell.exe"
            } else {
                "/bin/sh"
            });
            if cfg!(windows) {
                c.args(["-NoLogo", "-NoProfile"]);
            }
            c
        }
        Launch::Agent(program) => {
            if !AGENTS.contains(&program.as_str()) {
                return Err("Unknown agent.".into());
            }
            if !installed(program) {
                return Err(format!(
                    "{program} is not installed or not on PATH. Install its CLI, then refresh detection."
                ));
            }
            if cfg!(windows) {
                // CLI shims may be .cmd or .ps1. Only a catalog identifier is interpolated.
                let mut c = CommandBuilder::new("powershell.exe");
                c.args([
                    "-NoLogo",
                    "-NoProfile",
                    "-Command",
                    &format!("& '{program}'"),
                ]);
                c
            } else {
                CommandBuilder::new(program)
            }
        }
        Launch::Ssh(host) => {
            validate_host(&host.alias)?;
            if host.folder.contains(['\0', '\n', '\r']) {
                return Err("Invalid remote directory.".into());
            }
            let mut c = CommandBuilder::new("ssh");
            c.args([
                "-tt",
                "-o",
                "ConnectTimeout=15",
                "-o",
                "ServerAliveInterval=30",
                "-o",
                "ServerAliveCountMax=6",
                "--",
                &host.alias,
            ]);
            if !host.folder.is_empty() {
                c.arg(format!(
                    "cd -- {} && exec \"${{SHELL:-/bin/sh}}\" -l",
                    posix_quote(&host.folder)
                ));
            }
            c
        }
    };
    command.cwd(cwd);
    command.env("TERM", "xterm-256color");
    Ok(command)
}

pub fn default_directory() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_owned())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_path_is_single_quoted() {
        assert_eq!(posix_quote("a'b;$(bad)"), "'a'\\''b;$(bad)'");
    }
    #[test]
    fn refuses_arbitrary_agent_commands() {
        assert!(command(&Launch::Agent("cmd;evil".into()), Path::new(".")).is_err());
    }
}
