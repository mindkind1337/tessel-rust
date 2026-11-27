//! OpenSSH uses 255 for both a lost transport and a remote program's exit 255.
//! Observe private OSC frames from this launch, without parsing prompts or banners.
use std::time::{Duration, Instant};

pub(crate) struct Channel {
    nonce: String,
    prefix: Vec<u8>,
    matched: usize,
    tag: u8,
    pub opened: Option<Instant>,
    pub remote_255: bool,
    pub automatic: bool,
}

impl Channel {
    pub fn new(automatic: bool) -> Result<Self, String> {
        let nonce = crate::claude::uuid()?;
        Ok(Self {
            prefix: format!("\x1b]9;tessel-ssh-{nonce};").into_bytes(),
            nonce,
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
}

impl Retry {
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
    pub fn due(&mut self, now: Instant, channel: &Channel, exit: Option<i32>) -> bool {
        if exit.is_none() {
            if channel
                .opened
                .is_some_and(|t| now.saturating_duration_since(t) >= Duration::from_secs(60))
            {
                self.attempts = 0;
            }
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

    #[test]
    #[ignore = "requires WSL with OpenSSH server, public-key login to self, and tmux"]
    fn ssh_reconnect_after_transport_loss_keeps_the_remote_process_and_honors_exit_255() {
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
        let host = Launch::Ssh(Host {
            alias: "fixture".into(),
            folder: "~".into(),
            ..Default::default()
        });
        let id = crate::claude::uuid().unwrap();
        let start = |automatic| {
            let channel = Channel::new(automatic).unwrap();
            let mut command = crate::launch::pane_command(
                &host,
                Path::new("."),
                None,
                None,
                Some(&id),
                Some(&channel),
            )
            .unwrap();
            command
                .get_argv_mut()
                .splice(1..1, ["-F".into(), config.as_os_str().to_owned()]);
            let mut terminal = Terminal::spawn_command(command).unwrap();
            terminal.ssh_channel = Some(channel);
            terminal
        };
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
