use crate::workspace::{Host, Result, validate_host};
use portable_pty::CommandBuilder;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Launch {
    Shell,
    CodexChat,
    ClaudeChat,
    Agent(String),
    Ssh(Host),
}

impl Launch {
    pub fn native_chat(&self) -> Option<&'static str> {
        match self {
            Self::CodexChat => Some("Codex"),
            Self::ClaudeChat => Some("Claude"),
            _ => None,
        }
    }
}

pub fn installed(program: &str) -> bool {
    let Some(preset) = crate::agent_catalog::find(program) else {
        return false;
    };
    let program = preset.executable;
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

fn agent_command(preset: &crate::agent_catalog::Preset) -> CommandBuilder {
    if cfg!(windows) {
        // CLI shims may be .cmd or .ps1. Only static catalog executable names and quoted static arguments are used.
        let mut c = CommandBuilder::new("powershell.exe");
        c.args([
            "-NoLogo",
            "-NoProfile",
            "-Command",
            &format!(
                "& '{}'{}",
                preset.executable,
                preset
                    .args
                    .iter()
                    .map(|arg| format!(" '{}'", arg.replace('\'', "''")))
                    .collect::<String>()
            ),
        ]);
        c
    } else {
        let mut c = CommandBuilder::new(preset.executable);
        c.args(preset.args);
        c
    }
}

pub fn command(launch: &Launch, cwd: &Path) -> Result<CommandBuilder> {
    let mut command = match launch {
        Launch::CodexChat | Launch::ClaudeChat => {
            return Err("Native chats do not launch a terminal.".into());
        }
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
            let preset = crate::agent_catalog::find(program).ok_or("Unknown agent.")?;
            if !installed(program) {
                return Err(format!(
                    "{program} is not installed or not on PATH. Install its CLI, then refresh detection."
                ));
            }
            agent_command(preset)
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
    fn native_chats_never_launch_shells() {
        for launch in [Launch::CodexChat, Launch::ClaudeChat] {
            assert!(launch.native_chat().is_some());
            assert!(command(&launch, Path::new(".")).is_err());
            let encoded = serde_json::to_string(&launch).unwrap();
            let restored: Launch = serde_json::from_str(&encoded).unwrap();
            assert_eq!(launch.native_chat(), restored.native_chat());
        }
    }
    #[test]
    fn remote_path_is_single_quoted() {
        assert_eq!(posix_quote("a'b;$(bad)"), "'a'\\''b;$(bad)'");
    }
    #[test]
    fn refuses_arbitrary_agent_commands() {
        assert!(command(&Launch::Agent("cmd;evil".into()), Path::new(".")).is_err());
    }

    #[test]
    fn catalog_launches_use_executable_and_static_arguments_without_starting_processes() {
        for (id, expected) in [
            ("kiro", vec!["kiro-cli", "chat", "--tui"]),
            ("hermes", vec!["hermes", "--tui"]),
            ("continue", vec!["cn"]),
            ("cursor-agent", vec!["cursor-agent"]),
        ] {
            let command = agent_command(crate::agent_catalog::find(id).unwrap());
            let argv: Vec<_> = command
                .get_argv()
                .iter()
                .map(|arg| arg.to_str().unwrap())
                .collect();
            if cfg!(windows) {
                let script = format!(
                    "& {}",
                    expected
                        .iter()
                        .map(|word| format!("'{word}'"))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                assert_eq!(
                    argv,
                    vec![
                        "powershell.exe",
                        "-NoLogo",
                        "-NoProfile",
                        "-Command",
                        &script
                    ]
                );
            } else {
                assert_eq!(argv, expected);
            }
        }
    }
}
