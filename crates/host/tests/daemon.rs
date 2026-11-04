//! The real program: bridge, daemon start, private runtime directory, a bridge killed abruptly,
//! the lock, and refusals, each in a home directory of its own.
#![cfg(target_os = "linux")]
use std::io::{Read, Write};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};
use tessel_host_protocol::*;

const EXE: &str = env!("CARGO_BIN_EXE_tessel-host");
const WAIT: Duration = Duration::from_secs(20);

struct Home {
    path: PathBuf,
    daemons: Vec<u32>,
}

impl Home {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("tessel-host-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            path,
            daemons: Vec::new(),
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(EXE);
        command.args(args).env("HOME", &self.path);
        command
    }

    fn bridge(&self) -> Bridge {
        let mut child = self
            .command(&["connect", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let (sender, frames) = channel();
        std::thread::spawn(move || {
            while let Ok(Some(frame)) = read_frame::<HostFrame>(&mut stdout, MAX_FRAME_BYTES) {
                if sender.send(frame).is_err() {
                    break;
                }
            }
        });
        Bridge {
            child,
            stdin,
            frames,
            next: 0,
        }
    }

    fn version_dir(&self) -> PathBuf {
        self.path
            .join(".tessel-server/host")
            .join(tessel_host::VERSION)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        for pid in &self.daemons {
            // SAFETY: a signal to a daemon this test started.
            unsafe { libc::kill(*pid as i32, libc::SIGTERM) };
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

struct Bridge {
    child: Child,
    stdin: ChildStdin,
    frames: Receiver<HostFrame>,
    next: u64,
}

impl Bridge {
    fn ask(&mut self, message: ClientMessage) -> HostFrame {
        self.next += 1;
        write_frame(
            &mut self.stdin,
            &ClientFrame::request(self.next, message),
            MAX_FRAME_BYTES,
        )
        .unwrap();
        let reply = self.frames.recv_timeout(WAIT).expect("a reply");
        assert_eq!(reply.reply_to, Some(self.next));
        reply
    }

    fn hello(&mut self) -> HelloOk {
        let reply = self.ask(ClientMessage::Hello(Hello {
            versions: vec![1],
            capabilities: vec!["pty".into()],
            budgets: Budgets::SLOW_HOST,
        }));
        match ReplyMessage::from_raw(reply.ok.as_ref().expect("hello_ok")).unwrap() {
            Some(ReplyMessage::HelloOk(ok)) => ok,
            other => panic!("{other:?}"),
        }
    }

    fn info(&mut self) -> HostInfo {
        let reply = self.ask(ClientMessage::HostInfo);
        match ReplyMessage::from_raw(reply.ok.as_ref().unwrap()).unwrap() {
            Some(ReplyMessage::HostInfo(info)) => info,
            other => panic!("{other:?}"),
        }
    }

    fn exit_code(mut self) -> i32 {
        drop(self.stdin);
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.code().unwrap_or(-1);
            }
            assert!(Instant::now() < deadline, "the bridge did not end");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn stderr(&mut self) -> String {
        let mut text = String::new();
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    }
}

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn the_daemon_starts_once_and_outlives_a_killed_bridge() {
    let mut home = Home::new("outlive");
    let mut bridge = home.bridge();
    let first = bridge.hello();
    assert!(first.capabilities.is_empty(), "no capability before gate 3");
    let info = bridge.info();
    home.daemons.push(info.pid);
    assert_eq!(
        (info.version.as_str(), info.arch.as_str()),
        (tessel_host::VERSION, std::env::consts::ARCH)
    );
    assert!(bridge.ask(ClientMessage::Ping).ok.is_some());

    // Private: the daemon's directories 0700, its socket 0600.
    assert_eq!(mode(&home.path.join(".tessel-server/host")), 0o700);
    assert_eq!(mode(&home.version_dir()), 0o700);
    assert_eq!(mode(&home.version_dir().join("daemon.sock")), 0o600);

    // The SSH channel dies without warning: the daemon stays, with the same epoch.
    bridge.child.kill().unwrap();
    bridge.child.wait().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(alive(info.pid));
    let mut again = home.bridge();
    assert_eq!(again.hello().host_epoch, first.host_epoch);
    assert_eq!(again.info().pid, info.pid);

    // A second daemon of this version finds the lock taken and leaves.
    let status = home
        .command(&["serve"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(again.ask(ClientMessage::Ping).ok.is_some());

    // The channel closing normally ends the bridge, not the daemon.
    assert_eq!(again.exit_code(), 0);
    assert!(alive(info.pid));

    // Nothing in the log but operation names and numbers.
    let log = std::fs::read_to_string(home.version_dir().join("daemon.log")).unwrap();
    assert!(log.lines().all(|line| line.split(' ').skip(1).all(|word| {
        word.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'=')
    })));
}

#[test]
fn a_daemon_killed_outright_is_replaced_with_another_epoch() {
    let mut home = Home::new("replace");
    let mut bridge = home.bridge();
    let first = bridge.hello();
    let pid = bridge.info().pid;
    // SAFETY: the daemon this test started.
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    let deadline = Instant::now() + WAIT;
    while alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = bridge.exit_code();
    // Its socket is left behind; the next bridge starts a new daemon over it.
    assert!(home.version_dir().join("daemon.sock").exists());
    let mut next = home.bridge();
    let second = next.hello();
    let info = next.info();
    home.daemons.push(info.pid);
    assert_ne!(info.pid, pid);
    assert_ne!(
        second.host_epoch, first.host_epoch,
        "a client can tell its terminals are gone"
    );
}

#[test]
fn garbage_is_refused_and_the_daemon_goes_on() {
    let mut home = Home::new("garbage");
    let mut good = home.bridge();
    good.hello();
    home.daemons.push(good.info().pid);
    let mut bad = home.bridge();
    bad.stdin
        .write_all(&[0, 0, 0, 5, b'h', b'e', b'l', b'l', b'o'])
        .unwrap();
    let refusal = bad.frames.recv_timeout(WAIT).unwrap();
    assert_eq!(refusal.error.unwrap().code, ErrorCode::BadFrame);
    assert_eq!(bad.exit_code(), 0);
    assert!(good.ask(ClientMessage::Ping).ok.is_some());
}

#[test]
fn unsafe_runtime_directories_are_refused() {
    // ~/.tessel-server a symbolic link to somewhere else.
    let home = Home::new("symlink");
    let elsewhere = home.path.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    symlink(&elsewhere, home.path.join(".tessel-server")).unwrap();
    let mut bridge = home.bridge();
    let stderr = bridge.stderr();
    assert_eq!(bridge.exit_code(), 1);
    assert!(stderr.contains("unsafe runtime directory"), "{stderr}");
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);

    // Writable by others.
    let home = Home::new("open");
    std::fs::create_dir(home.path.join(".tessel-server")).unwrap();
    std::fs::set_permissions(
        home.path.join(".tessel-server"),
        std::fs::Permissions::from_mode(0o777),
    )
    .unwrap();
    assert_eq!(home.bridge().exit_code(), 1);

    // The daemon's own directory readable by the group.
    let home = Home::new("group");
    std::fs::create_dir_all(home.path.join(".tessel-server/host")).unwrap();
    std::fs::set_permissions(
        home.path.join(".tessel-server"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::set_permissions(
        home.path.join(".tessel-server/host"),
        std::fs::Permissions::from_mode(0o750),
    )
    .unwrap();
    assert_eq!(home.bridge().exit_code(), 1);

    // Something that is not a socket where the socket goes: never removed, never used.
    let home = Home::new("notsocket");
    let dir = home.version_dir();
    std::fs::create_dir_all(&dir).unwrap();
    for d in [
        home.path.join(".tessel-server"),
        home.path.join(".tessel-server/host"),
        dir.clone(),
    ] {
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::write(dir.join("daemon.sock"), b"keep me").unwrap();
    let mut bridge = home.bridge();
    let stderr = bridge.stderr();
    assert_eq!(bridge.exit_code(), 1, "{stderr}");
    assert_eq!(std::fs::read(dir.join("daemon.sock")).unwrap(), b"keep me");
}
