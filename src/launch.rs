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

pub fn discover(search: &crate::agent_paths::SearchPath) -> Result<Vec<(String, bool)>> {
    #[cfg(windows)]
    {
        let names: Vec<_> = crate::agent_catalog::ALL
            .iter()
            .map(|preset| preset.executable)
            .collect();
        let found = crate::command_lookup::lookup(&names, &search.value)?;
        Ok(crate::agent_catalog::ALL
            .iter()
            .map(|preset| {
                let available = found
                    .get(preset.executable)
                    .is_some_and(|path| preset.id != "kimi" || !legacy_kimi(path));
                (preset.id.to_owned(), available)
            })
            .collect())
    }
    #[cfg(not(windows))]
    {
        Ok(crate::agent_catalog::ALL
            .iter()
            .map(|preset| (preset.id.to_owned(), installed_on(preset.id, search)))
            .collect())
    }
}

#[cfg(any(not(windows), test))]
pub fn installed_on(program: &str, search: &crate::agent_paths::SearchPath) -> bool {
    let Some(preset) = crate::agent_catalog::find(program) else {
        return false;
    };
    let program = preset.executable;
    let first = search.directories.iter().find_map(|dir| {
        if cfg!(windows) {
            ["exe", "cmd", "bat", "ps1"]
                .iter()
                .map(|ext| dir.join(format!("{program}.{ext}")))
                .find(|path| path.is_file())
        } else {
            let path = dir.join(program);
            path.is_file().then_some(path)
        }
    });
    first.is_some_and(|path| preset.id != "kimi" || !legacy_kimi(&path.to_string_lossy()))
}

/// Source AGENT_PRESETS.kimi.notFrom: only the first resolved command counts.
fn legacy_kimi(path: &str) -> bool {
    let parts: Vec<_> = path.split(['/', '\\']).collect();
    let count = parts.len();
    if count < 4
        || !parts[count - 2].eq_ignore_ascii_case("Scripts")
        || !(parts[count - 1].eq_ignore_ascii_case("kimi")
            || parts[count - 1].eq_ignore_ascii_case("kimi.exe"))
    {
        return false;
    }
    let folder = parts[count - 3];
    let python = folder
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Python"))
        && folder
            .get(6..)
            .is_some_and(|suffix| suffix.bytes().all(|b| b.is_ascii_digit()));
    python
        || (count >= 5
            && !folder.is_empty()
            && parts[count - 4].eq_ignore_ascii_case("local-packages"))
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

#[cfg(any(test, feature = "visual-test"))]
pub fn command(launch: &Launch, cwd: &Path) -> Result<CommandBuilder> {
    command_with_preferences(launch, cwd, None)
}

fn configured_agent_command(
    preset: &crate::agent_catalog::Preset,
    prefs: &crate::agent_preferences::Preferences,
) -> Result<CommandBuilder> {
    let own = crate::agent_env::trim(&prefs.command);
    let args = crate::agent_env::trim(&prefs.args);
    if own.contains('\0') || args.contains('\0') {
        return Err("Agent command contains an invalid character.".into());
    }
    if own.is_empty() && args.is_empty() {
        return Ok(agent_command(preset));
    }
    let command = if own.is_empty() { preset.command } else { own };
    let text = if args.is_empty() {
        command.to_owned()
    } else {
        format!("{command} {args}")
    };
    // These are locally authored shell command/argument fields, as in Vue.
    // Imported settings cannot supply executable overrides (backup::import).
    let mut c = CommandBuilder::new(if cfg!(windows) {
        "powershell.exe"
    } else {
        "/bin/sh"
    });
    if cfg!(windows) {
        c.args(["-NoLogo", "-NoProfile", "-Command", &text]);
    } else {
        c.args(["-c", &text]);
    }
    Ok(c)
}

#[cfg(any(test, feature = "visual-test"))]
pub fn command_with_preferences(
    launch: &Launch,
    cwd: &Path,
    prefs: Option<&crate::agent_preferences::Preferences>,
) -> Result<CommandBuilder> {
    command_with_shell(launch, cwd, prefs, None)
}

pub fn command_with_shell(
    launch: &Launch,
    cwd: &Path,
    prefs: Option<&crate::agent_preferences::Preferences>,
    preferred_shell: Option<&str>,
) -> Result<CommandBuilder> {
    if launch.native_chat().is_some() {
        return Err("Native chats do not launch a terminal.".into());
    }
    let search = crate::agent_paths::SearchPath::current()?;
    let mut command = match launch {
        Launch::CodexChat | Launch::ClaudeChat => {
            return Err("Native chats do not launch a terminal.".into());
        }
        Launch::Shell => {
            if cfg!(windows) && preferred_shell == Some("cmd") {
                CommandBuilder::new("cmd.exe")
            } else if cfg!(windows) && preferred_shell == Some("wsl") {
                CommandBuilder::new("wsl.exe")
            } else {
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
        }
        Launch::Agent(program) => {
            if program.starts_with("custom-") {
                let Some(p) = prefs else {
                    return Err("Custom agent preferences missing.".into());
                };
                let cmd_text = crate::agent_env::trim(&p.command);
                if cmd_text.is_empty() || cmd_text.contains(['\0', '\r', '\n']) {
                    return Err("Custom agent command is empty or invalid.".into());
                }
                let mut c = CommandBuilder::new(if cfg!(windows) {
                    "powershell.exe"
                } else {
                    "/bin/sh"
                });
                if cfg!(windows) {
                    c.args(["-NoLogo", "-NoProfile", "-Command", cmd_text]);
                } else {
                    c.args(["-c", cmd_text]);
                }
                c
            } else {
                let preset = crate::agent_catalog::find(program).ok_or("Unknown agent.")?;
                #[cfg(windows)]
                let available = crate::command_lookup::lookup(&[preset.executable], &search.value)?
                    .get(preset.executable)
                    .is_some_and(|path| preset.id != "kimi" || !legacy_kimi(path));
                #[cfg(not(windows))]
                let available = installed_on(program, &search);
                let custom_command =
                    prefs.is_some_and(|p| !crate::agent_env::trim(&p.command).is_empty());
                if !available && !custom_command {
                    return Err(format!(
                        "{program} is not installed or not on PATH. Install its CLI, then refresh detection."
                    ));
                }
                match prefs {
                    Some(prefs) => configured_agent_command(preset, prefs)?,
                    None => agent_command(preset),
                }
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
    command.env("PATH", &search.value);
    if matches!(launch, Launch::Agent(_))
        && let Some(prefs) = prefs
    {
        // Source ignores the entire invalid block; the settings editor shows
        // its diagnostic. Never pass a partially parsed environment.
        if let Ok(env) = crate::agent_env::parse(&prefs.env) {
            if env.values().any(|value| value.contains('\0')) {
                return Err("Agent environment contains an invalid character.".into());
            }
            for (name, value) in env {
                command.env(name, value);
            }
        }
    }
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
    fn overrides_are_agent_only_and_invalid_environment_is_all_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut prefs = crate::agent_preferences::Preferences {
            command: "echo synthetic".into(),
            env: "RUST_ONLY_FIXTURE=synthetic\nTESSEL_FORBIDDEN=synthetic".into(),
            ..Default::default()
        };
        let configured =
            command_with_preferences(&Launch::Agent("codex".into()), dir.path(), Some(&prefs))
                .unwrap();
        assert!(configured.get_env("RUST_ONLY_FIXTURE").is_none());
        prefs.env = "RUST_ONLY_FIXTURE=synthetic".into();
        let shell = command_with_preferences(&Launch::Shell, dir.path(), Some(&prefs)).unwrap();
        assert!(shell.get_env("RUST_ONLY_FIXTURE").is_none());
        prefs.command = "bad\0command".into();
        assert!(
            command_with_preferences(&Launch::Agent("codex".into()), dir.path(), Some(&prefs))
                .is_err()
        );
    }

    #[test]
    fn kimi_exclusions_match_the_vue_regex_fixture() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/kimi_not_from.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let path = case["path"].as_str().unwrap();
            assert_eq!(
                legacy_kimi(path),
                case["excluded"].as_bool().unwrap(),
                "{path:?}"
            );
        }
    }

    #[test]
    fn old_python_kimi_shadows_newer_install_until_path_order_changes() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("Python313/Scripts");
        let new = temp.path().join(".kimi-code/bin");
        let filename = if cfg!(windows) { "kimi.exe" } else { "kimi" };
        for dir in [&old, &new] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(filename), "fixture only").unwrap();
        }
        let old_first = crate::agent_paths::SearchPath::with_extras(
            std::ffi::OsStr::new(""),
            &[old.clone(), new.clone()],
        )
        .unwrap();
        assert!(!installed_on("kimi", &old_first));
        let new_first =
            crate::agent_paths::SearchPath::with_extras(std::ffi::OsStr::new(""), &[new, old])
                .unwrap();
        assert!(installed_on("kimi", &new_first));
    }
    #[test]
    fn detection_finds_catalog_executable_in_appended_installer_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(if cfg!(windows) { "cn.cmd" } else { "cn" });
        std::fs::write(file, "synthetic fixture, never executed").unwrap();
        let search = crate::agent_paths::SearchPath::with_extras(
            std::ffi::OsStr::new(""),
            &[dir.path().into()],
        )
        .unwrap();
        assert!(installed_on("continue", &search));
        assert!(!installed_on("codex", &search));
        assert!(!installed_on("cn", &search));
    }
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
