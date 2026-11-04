//! `tessel-host connect --stdio`: what an SSH channel runs. Connects to this user's daemon
//! (starting it when there is none), then copies bytes both ways without reading them. When the
//! channel goes, the bridge goes; the daemon and what it owns stay.
use crate::persistence::{choose_method, linger};
use crate::runtime::{Runtime, random_bytes, runtime};
use std::fs::File;
use std::io::{self, Read, Write};
use std::mem::ManuallyDrop;
use std::net::Shutdown;
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tessel_host_protocol::PersistenceMethod;

const START_TIMEOUT: Duration = Duration::from_secs(10);

pub fn connect_stdio() -> i32 {
    let stream = match runtime().and_then(|runtime| connect_or_start(&runtime)) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("tessel-host: {error}");
            return 1;
        }
    };
    pump(stream)
}

fn connect_or_start(runtime: &Runtime) -> io::Result<UnixStream> {
    if let Ok(stream) = UnixStream::connect(&runtime.socket) {
        return Ok(stream);
    }
    start_daemon()?;
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match UnixStream::connect(&runtime.socket) {
            Ok(stream) => return Ok(stream),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn quiet(command: &mut Command) -> &mut Command {
    // The daemon must not hold the SSH channel open: none of its descriptors are the channel's.
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
}

/// Returns once the daemon is ready (its first process waits for that), or failed.
fn start_daemon() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    if choose_method(linger()) == PersistenceMethod::UserScope {
        let suffix: [u8; 4] = random_bytes()?;
        let unit = format!(
            "tessel-host-{}-{}",
            crate::VERSION,
            suffix
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let scoped = quiet(
            Command::new("systemd-run")
                .args(["--user", "--scope", "--quiet", "--collect", "--unit", &unit])
                .arg(&exe)
                .args(["serve", "--daemonize", "--method", "user_scope"]),
        )
        .status();
        if scoped.is_ok_and(|status| status.success()) {
            return Ok(());
        }
    }
    let status =
        quiet(Command::new(&exe).args(["serve", "--daemonize", "--method", "session"])).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("the host daemon did not start"))
    }
}

/// Plain reads and writes: whatever arrives is passed on at once. (Not `io::copy`, whose
/// Linux shortcuts through the kernel held a reply back in this pipe-to-socket setup.)
fn relay(from: &mut impl Read, to: &mut impl Write) -> io::Result<()> {
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = match from.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        to.write_all(&buffer[..n])?;
    }
}

fn pump(stream: UnixStream) -> i32 {
    let Ok(mut to_daemon) = stream.try_clone() else {
        return 1;
    };
    std::thread::spawn(move || {
        // SAFETY: descriptor 0 stays open for the process's life; ManuallyDrop never closes it.
        let mut stdin = ManuallyDrop::new(unsafe { File::from_raw_fd(0) });
        let _ = relay(&mut *stdin, &mut to_daemon);
        let _ = to_daemon.shutdown(Shutdown::Write);
    });
    let mut from_daemon = stream;
    // SAFETY: as above, for descriptor 1. Unbuffered: a frame goes out as soon as it arrives.
    let mut stdout = ManuallyDrop::new(unsafe { File::from_raw_fd(1) });
    match relay(&mut from_daemon, &mut *stdout) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}
