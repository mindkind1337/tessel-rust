//! Whether terminals outlive the SSH login that started the daemon, read from the host's
//! policy rather than assumed: systemd-logind kills a session's processes at logout when
//! KillUserProcesses is on, and a user's systemd manager (and its scopes) stops at the last
//! logout unless lingering is enabled. Nothing here changes either setting.
use std::ffi::CStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tessel_host_protocol::{Persistence, PersistenceMethod, Survival};

fn systemd_running() -> bool {
    Path::new("/run/systemd/system").is_dir()
}

fn output_within(command: &mut Command, limit: Duration) -> Option<String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let mut text = String::new();
                child
                    .stdout
                    .take()?
                    .take(4096)
                    .read_to_string(&mut text)
                    .ok()?;
                return Some(text);
            }
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn user_name() -> Option<String> {
    let mut buffer = vec![0u8; 16 * 1024];
    // SAFETY: passwd is plain data filled by getpwuid_r inside `buffer`, which outlives its use.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let result = unsafe {
        libc::getpwuid_r(
            libc::geteuid(),
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut found,
        )
    };
    if result != 0 || found.is_null() || entry.pw_name.is_null() {
        return None;
    }
    // SAFETY: pw_name points into `buffer`, NUL-terminated by getpwuid_r.
    let name = unsafe { CStr::from_ptr(entry.pw_name) }
        .to_str()
        .ok()?
        .to_owned();
    (!name.is_empty() && !name.contains('/')).then_some(name)
}

/// Some(true) when this user lingers (its systemd manager runs without a login).
pub fn linger() -> Option<bool> {
    if !systemd_running() {
        return None;
    }
    Some(
        Path::new("/var/lib/systemd/linger")
            .join(user_name()?)
            .exists(),
    )
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "on" | "1" => Some(true),
        "no" | "false" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// The value logind runs with, else the last one its configuration sets.
pub fn kill_user_processes() -> Option<bool> {
    if !systemd_running() {
        return None;
    }
    let asked = output_within(
        Command::new("busctl").args([
            "--system",
            "get-property",
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "KillUserProcesses",
        ]),
        Duration::from_secs(2),
    );
    match asked.as_deref().map(str::trim) {
        Some("b true") => return Some(true),
        Some("b false") => return Some(false),
        _ => {}
    }
    let mut files = vec![PathBuf::from("/etc/systemd/logind.conf")];
    for dir in [
        "/usr/lib/systemd/logind.conf.d",
        "/etc/systemd/logind.conf.d",
        "/run/systemd/logind.conf.d",
    ] {
        let mut more: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "conf"))
            .collect();
        more.sort();
        files.extend(more);
    }
    let mut value = None;
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for line in text.lines() {
            if let Some(setting) = line.trim().strip_prefix("KillUserProcesses=") {
                value = parse_bool(setting).or(value);
            }
        }
    }
    value
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

/// A user scope only helps when the user's manager outlives the login, that is with lingering.
pub fn choose_method(linger: Option<bool>) -> PersistenceMethod {
    let bus =
        std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|dir| Path::new(&dir).join("bus").exists());
    if linger == Some(true) && bus && on_path("systemd-run") {
        PersistenceMethod::UserScope
    } else {
        PersistenceMethod::Session
    }
}

pub fn describe(method: PersistenceMethod) -> Persistence {
    let linger = linger();
    let kill_user_processes = kill_user_processes();
    let survives_logout = match (method, linger, kill_user_processes) {
        (PersistenceMethod::UserScope, Some(true), _) => Survival::Yes,
        (_, _, Some(false)) => Survival::Yes,
        (PersistenceMethod::Session, _, Some(true)) => Survival::No,
        _ => Survival::Unknown,
    };
    Persistence {
        method,
        linger,
        kill_user_processes,
        survives_logout,
    }
}
