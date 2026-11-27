use crate::workspace::{Host, Result, validate_host};
use portable_pty::CommandBuilder;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Launch {
    Shell,
    CodexChat,
    ClaudeChat,
    Agent(String),
    Ssh(SshLaunch),
}

/// Keep the saved target separate from the directory chosen in Remote Files.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SshLaunch {
    #[serde(flatten)]
    pub target: Host,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_folder: Option<String>,
}

impl SshLaunch {
    pub fn folder(&self) -> &str {
        self.start_folder.as_deref().unwrap_or(&self.target.folder)
    }
}

impl Launch {
    pub fn ssh(target: Host) -> Self {
        Self::Ssh(SshLaunch {
            target,
            start_folder: None,
        })
    }

    pub fn belongs_to_ssh_target(&self, target: &Host) -> bool {
        matches!(self, Self::Ssh(ssh) if ssh.target.alias == target.alias && ssh.target.folder == target.folder)
    }

    pub fn native_chat(&self) -> Option<&'static str> {
        match self {
            Self::CodexChat => Some("Codex"),
            Self::ClaudeChat => Some("Claude"),
            _ => None,
        }
    }
}

pub fn discover(
    search: &crate::agent_paths::SearchPath,
    custom: &[crate::settings_extra_model::CustomAgent],
) -> Result<Vec<(String, bool)>> {
    let mut commands: Vec<_> = crate::agent_catalog::ALL
        .iter()
        .map(|preset| (preset.id, preset.executable))
        .collect();
    commands.extend(custom.iter().map(|agent| {
        let bin = crate::agent_names::js_trim(&agent.command)
            .split(crate::agent_names::js_space)
            .next()
            .unwrap_or_default();
        (agent.id.as_str(), bin)
    }));
    #[cfg(windows)]
    let found = {
        let mut names: Vec<_> = commands
            .iter()
            .map(|(_, bin)| *bin)
            .filter(|bin| crate::command_lookup::valid_name(bin))
            .collect();
        names.sort_unstable_by_key(|name| name.to_ascii_lowercase());
        names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let mut found = std::collections::HashMap::new();
        for part in names.chunks(64) {
            found.extend(crate::command_lookup::lookup(part, &search.value)?);
        }
        found
    };
    Ok(commands
        .into_iter()
        .map(|(id, bin)| {
            #[cfg(windows)]
            let available = found
                .get(&bin.to_ascii_lowercase())
                .is_some_and(|path| id != "kimi" || !legacy_kimi(path));
            #[cfg(not(windows))]
            let available =
                crate::command_lookup::valid_name(bin) && program_on_path(id, bin, search);
            (id.to_owned(), available)
        })
        .collect())
}

#[cfg(test)]
pub fn installed_on(program: &str, search: &crate::agent_paths::SearchPath) -> bool {
    crate::agent_catalog::find(program)
        .is_some_and(|preset| program_on_path(preset.id, preset.executable, search))
}

#[cfg(any(not(windows), test))]
fn program_on_path(id: &str, program: &str, search: &crate::agent_paths::SearchPath) -> bool {
    let first = search.directories.iter().find_map(|dir| {
        #[cfg(windows)]
        {
            ["exe", "cmd", "bat", "ps1"]
                .iter()
                .map(|ext| dir.join(format!("{program}.{ext}")))
                .find(|path| path.is_file())
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join(program);
            let metadata = path.metadata().ok()?;
            (metadata.is_file() && metadata.permissions().mode() & 0o111 != 0).then_some(path)
        }
    });
    first.is_some_and(|path| id != "kimi" || !legacy_kimi(&path.to_string_lossy()))
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

// ssh passes its remote command to the account's login shell. Keep POSIX
// syntax inside sh, and escape these three bytes outside single quotes:
// fish interprets backslashes in quotes and tcsh expands history there.
fn remote_sh_command(script: &str) -> String {
    let mut command = String::from("exec sh -c '");
    for ch in script.chars() {
        if matches!(ch, '\'' | '\\' | '!') {
            command.push_str("'\\");
            command.push(ch);
            command.push('\'');
        } else {
            command.push(ch);
        }
    }
    command.push('\'');
    command
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
    let mut pane_id = None;
    prepare_ssh_tmux_id(launch, &mut pane_id)?;
    command_with_shell(launch, cwd, prefs, None, pane_id.as_deref())
}

/// Allocate once per pane. The SSH alias identifies the machine, not a terminal on it.
pub(crate) fn prepare_ssh_tmux_id(launch: &Launch, id: &mut Option<String>) -> Result<()> {
    if matches!(launch, Launch::Ssh(host) if host.target.persistent_sessions_enabled())
        && !id.as_deref().is_some_and(crate::claude::valid_uuid)
    {
        *id = Some(crate::claude::uuid()?);
    }
    Ok(())
}

pub fn command_with_shell(
    launch: &Launch,
    cwd: &Path,
    prefs: Option<&crate::agent_preferences::Preferences>,
    preferred_shell: Option<&str>,
    ssh_tmux_id: Option<&str>,
) -> Result<CommandBuilder> {
    pane_command(launch, cwd, prefs, preferred_shell, ssh_tmux_id, None)
}

pub(crate) fn pane_command(
    launch: &Launch,
    cwd: &Path,
    prefs: Option<&crate::agent_preferences::Preferences>,
    preferred_shell: Option<&str>,
    ssh_tmux_id: Option<&str>,
    channel: Option<&crate::ssh_reconnect::Channel>,
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
                let available = program_on_path(preset.id, preset.executable, &search);
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
        Launch::Ssh(ssh) => {
            let host = &ssh.target;
            validate_host(&host.alias)?;
            if ssh.folder().contains(['\0', '\n', '\r']) {
                return Err("Invalid remote directory.".into());
            }
            let mut c = if let Some((helper, channel)) =
                channel.and_then(|ch| ch.direct_client.as_ref().map(|helper| (helper, ch)))
            {
                let mut cmd = CommandBuilder::new(helper);
                cmd.env("TESSEL_SSH_CHANNEL", &channel.nonce);
                cmd
            } else {
                CommandBuilder::new("ssh")
            };
            c.args([
                "-tt",
                "-o",
                crate::ssh_config::CONNECT_TIMEOUT,
                "-o",
                crate::ssh_config::KEEPALIVE_INTERVAL,
                "-o",
                crate::ssh_config::KEEPALIVE_COUNT,
            ]);
            if channel.is_some_and(|c| c.automatic) {
                c.args(["-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes"]);
            }
            c.args(["--", &host.alias]);
            if host.persistent_sessions_enabled()
                && !ssh_tmux_id.is_some_and(crate::claude::valid_uuid)
            {
                return Err("Missing persistent SSH pane identity.".into());
            }
            if let Some(remote_script) = crate::ssh_tmux::build_ssh_launch_remote_script(
                &host.alias,
                ssh.folder(),
                host.persistent_sessions_enabled(),
                ssh_tmux_id,
                true,
            )? {
                let script = channel
                    .map(|c| c.wrap(&remote_script))
                    .unwrap_or(remote_script);
                c.arg(remote_sh_command(&script));
            }
            c
        }
    };
    crate::agent_env::inherit_clean_pty(&mut command);
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
    fn ssh_target_roundtrip_keeps_legacy_targets_and_subfolder_ownership() {
        let legacy = serde_json::json!({"Ssh": {"alias":"lab","folder":"/srv"}});
        let launch: Launch = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(serde_json::to_value(&launch).unwrap(), legacy);
        let Launch::Ssh(mut ssh) = launch else {
            panic!("expected SSH")
        };
        let owner = ssh.target.clone();
        ssh.start_folder = Some("/srv/other".into());
        let saved = serde_json::to_vec(&Launch::Ssh(ssh)).unwrap();
        let restored: Launch = serde_json::from_slice(&saved).unwrap();
        assert!(restored.belongs_to_ssh_target(&owner));
        let Launch::Ssh(restored) = restored else {
            panic!("expected SSH")
        };
        assert_eq!(restored.folder(), "/srv/other");
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "requires WSL with fish and tcsh"]
    fn ssh_login_shells_pass_the_remote_script_to_sh_intact() {
        let root = std::env::var("TESSEL_TEST_WSL_SHELL_ROOT").unwrap_or_default();
        let payload = r#"two\\slashes !history 'quotes' "$HOME" ; `id`"#;
        let script = format!("value={}; printf %s \"$value\"", posix_quote(payload));
        for shell in ["bin/sh", "usr/bin/fish", "bin/tcsh"] {
            let output = std::process::Command::new("wsl.exe")
                .args(["--exec", "env"])
                .arg(format!("LD_LIBRARY_PATH={root}/usr/lib/x86_64-linux-gnu"))
                .arg(if shell == "bin/sh" {
                    "/bin/sh".to_owned()
                } else {
                    format!("{root}/{shell}")
                })
                .args(["-c", &remote_sh_command(&script)])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{shell}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.stdout, payload.as_bytes(), "{shell}");
        }
    }

    #[test]
    fn persistent_ssh_panes_keep_distinct_tmux_targets_across_reconnect() {
        let dir = tempfile::tempdir().unwrap();
        let host = Launch::ssh(crate::workspace::Host {
            alias: "fixture".into(),
            ..Default::default()
        });
        let script = |id: &mut Option<String>| {
            prepare_ssh_tmux_id(&host, id).unwrap();
            let command = command_with_shell(&host, dir.path(), None, None, id.as_deref()).unwrap();
            for required in [
                "ConnectTimeout=30",
                "ServerAliveInterval=15",
                "ServerAliveCountMax=12",
            ] {
                assert!(command.get_argv().iter().any(|arg| arg == required));
            }
            command
                .get_argv()
                .last()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };
        let mut first = None;
        let mut second = None;
        let initial = script(&mut first);
        let other = script(&mut second);
        assert_ne!(first, second);
        assert_ne!(initial, other);
        let expected = crate::ssh_tmux::build_ssh_launch_remote_script(
            "fixture",
            "",
            true,
            first.as_deref(),
            true,
        )
        .unwrap()
        .unwrap();
        assert_eq!(initial, remote_sh_command(&expected));
        assert_eq!(script(&mut first), initial);
        assert!(command_with_shell(&host, dir.path(), None, None, None).is_err());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires WSL with tmux installed"]
    fn persistent_ssh_panes_reattach_to_original_process_with_real_tmux() {
        use std::{
            io::Write,
            process::{Command, Stdio},
        };
        let dir = tempfile::tempdir().unwrap();
        let host = Launch::ssh(crate::workspace::Host {
            alias: "fixture".into(),
            folder: "~/".into(),
            ..Default::default()
        });
        let mut setup = String::from(
            r#"set -eu
umask 077
fixture_dir=$(mktemp -d /tmp/tessel-tmux-test.XXXXXXXX)
export TMUX_TMPDIR="$fixture_dir" SHELL=/bin/sh TERM=xterm-256color
cd "$fixture_dir"
unset TMUX
cleanup() {
    tmux -L harness kill-server 2>/dev/null || :
    tmux -L tessel kill-server 2>/dev/null || :
    rm -f first.sh second.sh
    cd /
    socket_dir="$fixture_dir/tmux-$(id -u)"
    rm -f -- "$socket_dir/harness" "$socket_dir/tessel"
    rmdir "$socket_dir" "$fixture_dir"
}
trap cleanup EXIT
export XDG_CONFIG_HOME="$fixture_dir"
"#,
        );
        for name in ["first", "second"] {
            let mut id = None;
            prepare_ssh_tmux_id(&host, &mut id).unwrap();
            let command = command_with_shell(&host, dir.path(), None, None, id.as_deref()).unwrap();
            let script = command.get_argv().last().unwrap().to_str().unwrap();
            setup.push_str(&format!(
                "{name}=tessel-{}\ncat > {name}.sh <<'SCRIPT'\n{script}\nSCRIPT\n",
                id.unwrap()
            ));
        }
        setup.push_str(r#"
attached() {
    for attempt in $(seq 1 100); do
        [ "$(tmux -L tessel display-message -p -t "=$1:" '#{session_attached}' 2>/dev/null)" = "$2" ] && return 0
        sleep 0.05
    done
    return 1
}
tmux -L harness -f /dev/null new-session -d -s a 'env -u TMUX sh first.sh'
attached "$first" 1
test "$(tmux -L tessel display-message -p -t "=$first:" '#{pane_current_path}')" = "$(cd ~ && pwd -P)"
test "$(tmux -L tessel show-options -gv history-limit)" = 50000
test "$(tmux -L tessel display-message -p -t "=$first:" '#{history_limit}')" = 50000
test "$(tmux -L tessel show-options -gv mouse)" = on
tmux -L tessel list-keys -T root WheelUpPane | grep -q 'mouse_any_flag.*PPage'
tmux -L tessel list-keys -T root WheelDownPane | grep -q 'mouse_any_flag.*NPage'
tmux -L tessel show-options -sv terminal-overrides | grep -Fq 'xterm-256color:RGB:Ms='
pid=$(tmux -L tessel display-message -p -t "=$first:" '#{pane_pid}')
tmux -L harness new-session -d -s b 'env -u TMUX sh second.sh'
attached "$second" 1
attached "$first" 1
tmux -L harness kill-session -t '=a'
attached "$first" 0
tmux -L harness new-session -d -s c 'env -u TMUX sh first.sh'
attached "$first" 1
attached "$second" 1
test "$(tmux -L tessel display-message -p -t "=$first:" '#{pane_pid}')" = "$pid"
"#);
        let mut child = Command::new("wsl.exe")
            .args(["--exec", "/bin/sh", "-s"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(setup.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
    }

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

    fn write_command_fixture(path: &Path) {
        std::fs::write(path, "synthetic fixture, never executed").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
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
            write_command_fixture(&dir.join(filename));
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
    fn custom_detection_batches_commands_and_keeps_custom_names_independent() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = if cfg!(windows) {
            "fixture-agent.cmd"
        } else {
            "fixture-agent"
        };
        write_command_fixture(&dir.path().join(cmd));
        let search = crate::agent_paths::SearchPath::with_extras(
            std::ffi::OsStr::new(""),
            &[dir.path().into()],
        )
        .unwrap();
        let mut custom: Vec<_> = (0..30)
            .map(|i| crate::settings_extra_model::CustomAgent {
                id: format!("custom-{i}"),
                name: format!("Agent {i}"),
                command: format!("fixture-missing-{i} --chat"),
                accent: "#abcdef".into(),
                enabled: true,
            })
            .collect();
        custom[24].command = "fixture-agent".into();
        custom[25].command = "\u{feff}fixture-agent\u{00a0}--chat".into();
        custom[26].command = "./fixture-agent".into();
        custom[27].command = "fixture-agent;echo".into();
        custom[28].command = "  ".into();
        custom[29].command = if cfg!(windows) {
            "FIXTURE-AGENT"
        } else {
            "fixture-agent"
        }
        .into();
        let found = discover(&search, &custom).unwrap();
        assert_eq!(found.len(), crate::agent_catalog::ALL.len() + custom.len());
        for (i, agent) in custom.iter().enumerate() {
            assert!(found.contains(&(agent.id.clone(), matches!(i, 24 | 25 | 29))));
        }
    }

    #[test]
    fn detection_finds_catalog_executable_in_appended_installer_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(if cfg!(windows) { "cn.cmd" } else { "cn" });
        write_command_fixture(&file);
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
