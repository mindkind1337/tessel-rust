//! OpenSSH uses 255 for both a lost transport and a remote program's exit 255.
//! Observe private OSC frames from this launch, without parsing prompts or banners.
use std::time::{Duration, Instant};

pub(crate) struct Channel {
    pub(super) nonce: String,
    pub direct_client: Option<std::path::PathBuf>,
    prefix: Vec<u8>,
    matched: usize,
    tag: u8,
    pub opened: Option<Instant>,
    pub remote_255: bool,
    pub automatic: bool,
}

// Preserve the user's selected SSH client. Only the Windows system client
// has been validated with the companion's diagnostic format.
fn system_openssh_selected() -> bool {
    #[cfg(windows)]
    {
        let Some(root) = std::env::var_os("SystemRoot") else {
            return false;
        };
        let system = std::path::PathBuf::from(root).join("System32/OpenSSH/ssh.exe");
        let Ok(expected) = system.canonicalize() else {
            return false;
        };
        let Ok(search) = crate::agent_paths::SearchPath::current() else {
            return false;
        };
        let first = crate::command_lookup::lookup(&["ssh"], &search.value)
            .ok()
            .and_then(|paths| paths.get("ssh").cloned())
            .and_then(|path| std::fs::canonicalize(path).ok());
        first.as_ref() == Some(&expected)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

impl Channel {
    pub fn for_launch(
        launch: &crate::launch::Launch,
        automatic: bool,
    ) -> Result<Option<Self>, String> {
        if cfg!(windows)
            && matches!(launch, crate::launch::Launch::Ssh(host) if host.target.persistent_sessions_enabled() || !host.folder().is_empty())
        {
            Self::new(automatic).map(Some)
        } else if cfg!(windows) && matches!(launch, crate::launch::Launch::Ssh(_)) {
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            let helper = executable.with_file_name("tessel-ssh.exe");
            #[cfg(test)]
            let helper = if !helper.is_file()
                && let Some(parent) = executable.parent().and_then(|p| p.parent())
            {
                parent.join("tessel-ssh.exe")
            } else {
                helper
            };
            if helper.is_file() && system_openssh_selected() {
                let mut channel = Self::new(automatic)?;
                channel.direct_client = Some(helper);
                return Ok(Some(channel));
            }
            Ok(None)
        } else {
            Ok(None)
        }
    }
    pub fn new(automatic: bool) -> Result<Self, String> {
        let nonce = crate::claude::uuid()?;
        Ok(Self {
            prefix: format!("\x1b]9;tessel-ssh-{nonce};").into_bytes(),
            nonce,
            direct_client: None,
            matched: 0,
            tag: 0,
            opened: None,
            remote_255: false,
            automatic,
        })
    }
    pub fn wrap(&self, script: &str) -> String {
        // A normal 255 leaves OpenSSH as 254; the matching frame restores its
        // display value even if the last PTY bytes arrive after process exit.
        format!(
            r#"printf '\033]9;tessel-ssh-{0};R\007'; ( {script} ); _t_status=$?; if [ "$_t_status" = 255 ]; then printf '\033]9;tessel-ssh-{0};X\007'; exit 254; fi; exit "$_t_status""#,
            self.nonce
        )
    }
    pub fn observe(&mut self, bytes: &[u8]) {
        if self.matched == 0 && !bytes.contains(&27) {
            return;
        }
        for &byte in bytes {
            if self.matched == 0 && byte != 27 {
                continue;
            }
            if self.prefix.get(self.matched) == Some(&byte) {
                self.matched += 1;
            } else if self.matched == self.prefix.len() && matches!(byte, b'R' | b'X') {
                self.tag = byte;
                self.matched += 1;
            } else {
                if self.matched == self.prefix.len() + 1 && byte == 7 {
                    if self.tag == b'R' {
                        self.opened.get_or_insert_with(Instant::now);
                    } else {
                        self.remote_255 = true;
                    }
                }
                self.matched = usize::from(byte == 27);
            }
        }
    }
    pub fn permits_input(&self) -> bool {
        !self.automatic || self.opened.is_some()
    }
}

const DELAYS: [u64; 9] = [1, 2, 5, 5, 10, 10, 10, 30, 30];

#[derive(Default)]
pub(crate) struct Retry {
    attempts: usize,
    deadline: Option<Instant>,
    last_lost_session: Option<Instant>,
    history: Option<Box<vt100::Parser>>,
}

impl Retry {
    pub fn retain_output(&mut self, terminal: &mut crate::terminal::Terminal) {
        self.history = Some(Box::new(terminal.take_history_for_reconnect()));
    }
    pub fn restore_output(&mut self, terminal: &mut crate::terminal::Terminal, replaced: bool) {
        if let Some(previous) = self.history.take() {
            terminal.restore_history_after_restart(*previous, replaced);
        }
    }

    pub fn automatic(&self) -> bool {
        self.attempts != 0
    }
    pub fn give_up(&mut self) {
        self.deadline = None;
        self.attempts = DELAYS.len();
    }
    pub fn exhausted(&self) -> bool {
        self.attempts >= DELAYS.len()
    }
    pub fn pending(&self) -> bool {
        self.deadline.is_some()
    }
    pub fn notice(&self, french: bool, transport_loss: bool) -> Option<&'static str> {
        if self.pending() {
            Some(if french {
                "Nouvelle tentative automatique prévue."
            } else {
                "Automatic reconnect scheduled."
            })
        } else if self.exhausted() && transport_loss {
            Some(if french {
                "Les tentatives automatiques sont terminées."
            } else {
                "Automatic retries have ended."
            })
        } else {
            None
        }
    }
    pub fn due(&mut self, now: Instant, channel: &Channel, exit: Option<i32>) -> bool {
        // Inspect uptime on the first exit observation too. The UI may have
        // slept across the stability threshold. Time spent waiting after exit
        // must never make a short-lived connection look stable.
        if let Some(opened) = channel.opened
            && (exit.is_none() || self.last_lost_session != Some(opened))
        {
            if now.saturating_duration_since(opened) >= Duration::from_secs(60) {
                self.attempts = 0;
            }
            if exit.is_some() {
                self.last_lost_session = Some(opened);
            }
        }
        if exit.is_none() {
            return false;
        }
        if exit != Some(255)
            || channel.remote_255
            || (channel.opened.is_none() && !self.automatic())
        {
            self.deadline = None;
            return false;
        }
        let Some(delay) = DELAYS.get(self.attempts) else {
            return false;
        };
        let deadline = self
            .deadline
            .get_or_insert_with(|| now + Duration::from_secs(*delay));
        if now < *deadline {
            return false;
        }
        self.deadline = None;
        self.attempts += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnected_ssh_history_keeps_styled_rows_within_the_new_limit() {
        let mut old = vt100::Parser::new(3, 12, 20);
        old.process(b"discard\r\n\x1b[31mfirst\x1b[m\r\n\r\nlast\x1b[?2004h\x1b[?1049hprivate TUI");
        let mut new = vt100::Parser::new(2, 15, 3);
        new.process(b"fresh");
        new.prepend_history(old);
        assert_eq!(new.screen().contents(), "fresh");
        assert!(!new.screen().alternate_screen());
        assert!(!new.screen().bracketed_paste());
        new.set_scrollback(usize::MAX);
        assert_eq!(new.screen().scrollback(), 3);
        assert!(new.screen().contents().starts_with("first"));
        assert_eq!(
            new.screen().cell(0, 0).unwrap().fgcolor(),
            vt100::Color::Idx(1)
        );
        new.set_scrollback(1);
        assert_eq!(new.screen().contents(), "last\nfresh");
    }

    #[test]
    fn reconnect_keeps_the_reader_at_the_same_archived_line() {
        let mut old = vt100::Parser::new(2, 20, 10);
        old.process(b"first\r\nsecond\r\nthird\r\nfourth");
        old.set_scrollback(1);
        assert_eq!(old.screen().contents(), "second\nthird");
        let mut new = vt100::Parser::new(2, 20, 10);
        new.prepend_history(old);
        new.process(b"new shell");
        assert_eq!(new.screen().contents(), "second\nthird");
        new.set_scrollback(0);
        assert_eq!(new.screen().contents(), "new shell");
    }

    #[test]
    fn frames_require_the_same_launch_and_survive_arbitrary_read_boundaries() {
        let mut channel = Channel::new(true).unwrap();
        let other = Channel::new(true).unwrap();
        let frame = |c: &Channel, tag: &str| format!("\x1b]9;tessel-ssh-{};{tag}\x07", c.nonce);
        channel.observe(frame(&other, "R").as_bytes());
        channel.observe(frame(&channel, "Rr").as_bytes());
        assert!(!channel.permits_input());
        for byte in frame(&channel, "R").bytes() {
            channel.observe(&[byte]);
        }
        let opened = channel.opened.unwrap();
        assert!(channel.permits_input());
        channel.observe(frame(&channel, "R").as_bytes());
        assert_eq!(channel.opened, Some(opened));
        channel.observe(frame(&channel, "X").as_bytes());
        assert!(channel.remote_255);
    }

    #[test]
    fn retry_budget_uses_uptime_at_first_observed_exit() {
        let opened = Instant::now();
        let mut channel = Channel::new(true).unwrap();
        channel.opened = Some(opened);
        let mut retry = Retry {
            attempts: DELAYS.len(),
            ..Default::default()
        };
        let lost = opened + Duration::from_secs(60);
        assert!(!retry.due(lost, &channel, Some(255)));
        assert!(retry.due(lost + Duration::from_secs(1), &channel, Some(255)));

        let mut retry = Retry {
            attempts: DELAYS.len() - 1,
            ..Default::default()
        };
        let lost = opened + Duration::from_secs(59);
        assert!(!retry.due(lost, &channel, Some(255)));
        assert!(!retry.due(lost + Duration::from_secs(1), &channel, Some(255)));
        assert!(retry.due(lost + Duration::from_secs(30), &channel, Some(255)));
        assert!(retry.exhausted());
        assert!(!retry.due(lost + Duration::from_secs(31), &channel, Some(255)));
        assert!(!retry.pending());
    }

    #[test]
    fn retry_ladder_only_starts_after_open_and_stops_after_nine_attempts() {
        let mut retry = Retry::default();
        let mut channel = Channel::new(false).unwrap();
        let mut now = Instant::now();
        assert!(!retry.due(now, &channel, Some(255)));
        assert!(!retry.pending());
        channel.opened = Some(now);
        for delay in DELAYS {
            assert!(!retry.due(now, &channel, Some(255)));
            assert!(retry.pending());
            now += Duration::from_secs(delay);
            assert!(!retry.due(now - Duration::from_millis(1), &channel, Some(255)));
            assert!(retry.due(now, &channel, Some(255)));
            channel = Channel::new(true).unwrap();
        }
        assert!(retry.exhausted());
        assert!(!retry.due(now, &channel, Some(255)));
        assert!(!retry.pending());
        channel.opened = Some(now);
        retry.due(now + Duration::from_secs(60), &channel, None);
        assert!(!retry.automatic());
        for exit in [0, 1, 254] {
            assert!(!retry.due(now, &channel, Some(exit)));
            assert!(!retry.pending());
        }
        channel.remote_255 = true;
        assert!(!retry.due(now, &channel, Some(255)));
        assert!(!retry.pending());
    }
}

#[cfg(all(test, windows))]
mod openssh_tests {
    use super::*;
    use crate::{launch::Launch, terminal::Terminal, workspace::Host};
    use std::{path::Path, process::Command, thread};

    fn wsl(args: &[&str]) -> String {
        let out = Command::new("wsl.exe")
            .arg("--exec")
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    struct Fixture(String);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = Command::new("wsl.exe").args(["--exec", "sh", "-c",
                r#"case "$1" in /tmp/tessel-reconnect.????????) TMUX_TMPDIR="$1" tmux -L tessel kill-server 2>/dev/null || :; rm -rf -- "$1";; esac"#, "sh", &self.0]).status();
        }
    }

    fn wait_until(terminal: &mut Terminal, check: impl Fn(&Terminal) -> bool) {
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            terminal.poll();
            if check(terminal) {
                return;
            }
            assert!(
                Instant::now() < end,
                "fixture terminal timed out: {:?}",
                terminal.exit_code()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn isolated_server() -> (Fixture, tempfile::TempDir, std::path::PathBuf) {
        let fixture = Fixture(wsl(&["mktemp", "-d", "/tmp/tessel-reconnect.XXXXXXXX"]));
        let root = fixture.0.as_str();
        let setup = include_str!("../tests/fixtures/ssh_reconnect_setup.sh").replace("\r\n", "\n");
        wsl(&["sh", "-c", &setup, "sh", root]);
        let local = tempfile::tempdir().unwrap();
        let config = local.path().join("config");
        let key = local.path().join("client");
        let known = local.path().join("known_hosts");
        std::fs::File::create(&key).unwrap();
        let acl = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command",
            r#"$ErrorActionPreference='Stop'; $sid=[Security.Principal.WindowsIdentity]::GetCurrent().User; $acl=[Security.AccessControl.FileSecurity]::new(); $acl.SetAccessRuleProtection($true,$false); $acl.SetAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid,'FullControl','Allow')); [IO.File]::SetAccessControl($env:TESSEL_FIXTURE_KEY,$acl)"#])
            .env("TESSEL_FIXTURE_KEY",&key).status().unwrap();
        assert!(acl.success());

        std::fs::write(
            &key,
            format!("{}\n", wsl(&["cat", &format!("{root}/client")])),
        )
        .unwrap();
        std::fs::write(
            &known,
            format!("{}\n", wsl(&["cat", &format!("{root}/known")])),
        )
        .unwrap();
        std::fs::write(&config, format!(
            "Host fixture\n HostName 127.0.0.1\n User {}\n IdentityFile \"{}\"\n IdentitiesOnly yes\n BatchMode yes\n StrictHostKeyChecking yes\n UserKnownHostsFile \"{}\"\n GlobalKnownHostsFile none\n ProxyCommand wsl.exe --exec {root}/proxy\n",
            wsl(&["id","-un"]), key.to_string_lossy().replace('\\',"/"), known.to_string_lossy().replace('\\',"/")
        )).unwrap();
        (fixture, local, config)
    }

    #[test]
    #[ignore = "requires WSL sshd and Windows OpenSSH"]
    fn saved_host_connection_test_authenticates_refuses_unknown_keys_and_times_out() {
        use crate::ssh_connection_test::{Failure, run};
        let (fixture, local, config) = isolated_server();
        let ssh = || {
            let mut cmd = tokio::process::Command::new("ssh");
            cmd.arg("-F").arg(&config);
            cmd
        };
        assert_eq!(run(ssh(), "fixture", Duration::from_secs(10)), Ok(()));
        let mut unknown = ssh();
        let absent = local.path().join("absent_known_hosts");
        unknown
            .arg("-o")
            .arg(format!("UserKnownHostsFile={}", absent.display()));
        assert!(matches!(
            run(unknown, "fixture", Duration::from_secs(10)),
            Err(Failure::Detail(_))
        ));
        assert!(!absent.exists());
        wsl(&[
            "sh",
            "-c",
            r#"printf '#!/bin/sh\nsleep 3\n' > "$1/login""#,
            "sh",
            &fixture.0,
        ]);
        assert_eq!(
            run(ssh(), "fixture", Duration::from_millis(500)),
            Err(Failure::Timeout)
        );
    }

    fn fixture_terminal(
        config: &Path,
        host: &Launch,
        id: Option<&str>,
        automatic: bool,
    ) -> Terminal {
        let channel = Channel::new(automatic).unwrap();
        let mut command =
            crate::launch::pane_command(host, Path::new("."), None, None, id, Some(&channel))
                .unwrap();
        command
            .get_argv_mut()
            .splice(1..1, ["-F".into(), config.as_os_str().to_owned()]);
        let mut terminal = Terminal::spawn_command(command).unwrap();
        terminal.ssh_channel = Some(channel);
        terminal
    }

    #[test]
    #[ignore = "requires WSL with OpenSSH server and public-key login to self"]
    fn ssh_reconnect_preserves_searchable_output_after_real_transport_loss() {
        let (fixture, _local, config) = isolated_server();
        let host = Launch::ssh(Host {
            alias: "fixture".into(),
            folder: "~".into(),
            persistent_sessions: Some(false),
            ..Default::default()
        });
        let mut first = fixture_terminal(&config, &host, None, false);
        wait_until(&mut first, |t| {
            t.ssh_channel.as_ref().unwrap().opened.is_some() && t.output().contains('$')
        });
        first
            .run_quick_command(r#"seq 1 80 | sed 's/^/history-line-/'"#, true)
            .unwrap();
        wait_until(&mut first, |t| t.output().contains("history-line-80"));
        wsl(&[
            "sh",
            "-c",
            r#"kill -KILL "$(cat "$1/transport")""#,
            "sh",
            &fixture.0,
        ]);
        wait_until(&mut first, Terminal::exited);
        assert_eq!(first.exit_code(), Some(255));

        let mut retry = Retry::default();
        retry.retain_output(&mut first);
        first.stop_for_restart().unwrap();
        drop(first);
        let mut next = fixture_terminal(&config, &host, None, true);
        retry.restore_output(&mut next, true);
        wait_until(&mut next, |t| {
            t.ssh_channel.as_ref().unwrap().opened.is_some() && t.output().contains('$')
        });
        assert!(!next.output().contains("history-line-42"));
        next.gpui_find_set_query("history-line-42".into());
        assert!(next.output().contains("history-line-42"));
        assert_eq!(next.take_selection().as_deref(), Some("history-line-42"));
    }

    #[test]
    #[ignore = "requires WSL with OpenSSH server and public-key login to self"]
    fn direct_ssh_login_preserves_shell_request_and_reports_both_255_outcomes() {
        let (fixture, local, config) = isolated_server();
        let host = Launch::ssh(Host {
            alias: "fixture".into(),
            folder: String::new(),
            persistent_sessions: Some(false),
            ..Default::default()
        });
        let open = |diagnostics: &Path| {
            let mut command =
                crate::launch::pane_command(&host, local.path(), None, None, None, None).unwrap();
            assert_eq!(command.get_argv().last().unwrap(), "fixture");
            command.get_argv_mut().splice(
                1..1,
                [
                    "-E".into(),
                    diagnostics.as_os_str().to_owned(),
                    "-o".into(),
                    "LogLevel=ERROR".into(),
                    "-F".into(),
                    config.as_os_str().to_owned(),
                ],
            );
            let mut shell = Terminal::spawn_command(command).unwrap();
            wait_until(&mut shell, |t| t.output().contains("fixture-login>"));
            shell
        };
        let log = local.path().join("client-errors");
        let mut shell = open(&log);
        // Remote output can imitate OpenSSH. Only the client diagnostic stream is evidence.
        shell
            .run_quick_command("echo Connection reset by 'peer' >&2", true)
            .unwrap();
        wait_until(&mut shell, |t| {
            t.output().contains("Connection reset by peer")
        });
        shell.run_quick_command("exit 255", true).unwrap();
        wait_until(&mut shell, Terminal::exited);
        assert_eq!(shell.exit_code(), Some(255));
        assert!(std::fs::read(&log).unwrap().is_empty());
        let mut shell = open(&log);
        wsl(&[
            "sh",
            "-c",
            r#"kill -KILL "$(cat "$1/transport")""#,
            "sh",
            &fixture.0,
        ]);
        wait_until(&mut shell, Terminal::exited);
        assert_eq!(shell.exit_code(), Some(255));
        let diagnostic = std::fs::read_to_string(log).unwrap().to_ascii_lowercase();
        assert!(
            diagnostic
                .lines()
                .any(|line| line.starts_with("client_loop: send disconnect:"))
                || diagnostic.contains("closed")
                || diagnostic.contains("reset")
                || diagnostic.contains("broken pipe")
        );
    }

    #[test]
    #[ignore = "requires the console companion, Windows OpenSSH and WSL sshd"]
    fn direct_login_retries_only_transport_loss_and_opens_a_new_shell() {
        let (fixture, local, config) = isolated_server();
        let host = Launch::ssh(Host {
            alias: "fixture".into(),
            persistent_sessions: Some(false),
            ..Default::default()
        });
        let open = |automatic| {
            let channel = Channel::for_launch(&host, automatic).unwrap().unwrap();
            assert!(channel.direct_client.is_some());
            let mut command =
                crate::launch::pane_command(&host, local.path(), None, None, None, Some(&channel))
                    .unwrap();
            assert_eq!(command.get_argv().last().unwrap(), "fixture");
            command
                .get_argv_mut()
                .splice(1..1, ["-F".into(), config.as_os_str().to_owned()]);
            let mut terminal = Terminal::spawn_command(command).unwrap();
            assert_eq!(channel.permits_input(), !automatic);
            terminal.ssh_channel = Some(channel);
            wait_until(&mut terminal, |t| {
                t.ssh_channel.as_ref().unwrap().opened.is_some()
                    && t.output().contains("fixture-login>")
            });
            terminal
        };
        let mut first = open(false);
        wsl(&[
            "sh",
            "-c",
            r#"kill -KILL "$(cat "$1/transport")""#,
            "sh",
            &fixture.0,
        ]);
        wait_until(&mut first, Terminal::exited);
        assert!(first.ssh_transport_lost());
        let mut retry = Retry::default();
        let now = Instant::now();
        let channel = first.ssh_channel.as_ref().unwrap();
        assert!(!retry.due(now, channel, first.exit_code()));
        assert!(retry.due(now + Duration::from_secs(1), channel, first.exit_code()));
        let mut next = open(true);
        next.run_quick_command(
            "echo client_loop: send disconnect: 'spoof' >&2; exit 255",
            true,
        )
        .unwrap();
        wait_until(&mut next, Terminal::exited);
        assert_eq!(next.exit_code(), Some(255));
        assert!(!next.ssh_transport_lost());
        assert!(!retry.due(
            Instant::now(),
            next.ssh_channel.as_ref().unwrap(),
            next.exit_code()
        ));
        assert!(!retry.pending());
    }

    fn assert_clean_command(terminal: &mut Terminal, root: &str, marker: &str) {
        terminal
            .run_quick_command(&format!(r#"echo {marker} > "$HOME/first-command""#), true)
            .unwrap();
        let observed = wsl(&[
            "sh",
            "-c",
            r#"
            for i in 1 2 3 4 5 6 7 8 9 10; do
                if test -s "$1/first-command"; then cat "$1/first-command"; exit; fi
                sleep 0.1
            done
            TMUX_TMPDIR="$1" tmux -L tessel capture-pane -p
        "#,
            "sh",
            root,
        ]);
        assert_eq!(observed, marker);
        wsl(&["rm", "--", &format!("{root}/first-command")]);
    }

    #[test]
    #[ignore = "requires WSL with OpenSSH server, public-key login to self, and tmux"]
    fn ssh_reconnect_after_transport_loss_keeps_the_remote_process_and_honors_exit_255() {
        let (fixture, _local, config) = isolated_server();
        let root = fixture.0.as_str();
        let host = Launch::ssh(Host {
            alias: "fixture".into(),
            folder: "~".into(),
            ..Default::default()
        });
        let id = crate::claude::uuid().unwrap();
        let start = |automatic| fixture_terminal(&config, &host, Some(&id), automatic);
        let pid = || {
            wsl(&[
                "env",
                &format!("TMUX_TMPDIR={root}"),
                "tmux",
                "-L",
                "tessel",
                "list-panes",
                "-F",
                "#{pane_pid}",
            ])
        };
        let mut first = start(false);
        wait_until(&mut first, |t| {
            t.ssh_channel.as_ref().unwrap().opened.is_some() && t.output().contains('$')
        });
        assert_clean_command(&mut first, root, "initial");
        let first_pid = pid();
        assert!(first_pid.parse::<u32>().is_ok());
        wsl(&[
            "sh",
            "-c",
            r#"kill -KILL "$(cat "$1/transport")""#,
            "sh",
            root,
        ]);
        wait_until(&mut first, Terminal::exited);
        assert_eq!(first.exit_code(), Some(255));
        let mut retry = Retry::default();
        let now = Instant::now();
        let channel = first.ssh_channel.as_ref().unwrap();
        assert!(!retry.due(now, channel, first.exit_code()));
        assert!(retry.due(now + Duration::from_secs(1), channel, first.exit_code()));
        first.stop_for_restart().unwrap();
        drop(first);
        let mut second = start(retry.automatic());
        assert!(!second.accepts_broadcast_input());
        wait_until(&mut second, |t| {
            t.ssh_channel.as_ref().unwrap().opened.is_some() && t.output().contains('$')
        });
        assert!(second.accepts_broadcast_input());
        assert_eq!(pid(), first_pid);
        assert_clean_command(&mut second, root, "reattached");
        // End the remote session directly. tmux reports success when its last
        // pane closes; check a remote program's explicit 255 separately below.
        wsl(&[
            "env",
            &format!("TMUX_TMPDIR={root}"),
            "tmux",
            "-L",
            "tessel",
            "kill-session",
            "-t",
            &format!("={}", crate::ssh_tmux::tmux_session_name(&id)),
        ]);
        wait_until(&mut second, Terminal::exited);
        assert_eq!(second.exit_code(), Some(0));
        let channel = Channel::new(false).unwrap();
        let mut cmd = portable_pty::CommandBuilder::new("ssh");
        cmd.arg("-F");
        cmd.arg(&config);
        cmd.args(["-tt", "fixture", &channel.wrap("exit 255")]);
        let mut normal = Terminal::spawn_command(cmd).unwrap();
        normal.ssh_channel = Some(channel);
        wait_until(&mut normal, |t| {
            t.exited() && t.ssh_channel.as_ref().unwrap().remote_255
        });
        assert_eq!(normal.exit_code(), Some(255));
        assert!(!retry.due(
            Instant::now(),
            normal.ssh_channel.as_ref().unwrap(),
            normal.exit_code()
        ));
        assert!(!retry.pending());
    }
}
