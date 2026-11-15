//! Bounded Git process execution, independent of application UI or workspace state.
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, String>;
#[cfg(windows)]
use crate::process_job;
/// Preserve NUL-delimited paths and expose truncation separately from stdout.
/// Structured callers must disable bulk actions when `truncated` is true.
pub fn run_bytes(root: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Capture> {
    run_bytes_with_timeout(root, args, input, Duration::from_secs(20))
}
pub(super) fn run_bytes_with_timeout(
    root: &Path,
    args: &[&str],
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<Capture> {
    run_bytes_config(root, args, input, timeout, false, false)
}
pub fn run_guest_bytes(
    root: &Path,
    args: &[&str],
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<Capture> {
    run_bytes_config(root, args, input, timeout, true, false)
}
/// Exit 1 means no ignored paths only for this fixed check-ignore command.
pub fn run_guest_check_ignore(
    root: &Path,
    prefix: &[&str],
    input: &[u8],
    no_index: bool,
    timeout: Duration,
) -> Result<Capture> {
    // check-ignore treats stdin as literal names and rejects --literal-pathspecs.
    let mut args: Vec<&str> = prefix
        .iter()
        .copied()
        .filter(|arg| *arg != "--literal-pathspecs")
        .collect();
    args.push("check-ignore");
    if no_index {
        args.push("--no-index");
    }
    args.extend(["-z", "--stdin"]);
    run_bytes_config(root, &args, Some(input), timeout, true, true)
}
fn run_bytes_config(
    root: &Path,
    args: &[&str],
    input: Option<&[u8]>,
    timeout: Duration,
    guest: bool,
    allow_no_match: bool,
) -> Result<Capture> {
    let mut command = Command::new("git");
    if guest {
        for (key, _) in std::env::vars_os() {
            if key
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                command.env_remove(key);
            }
        }
    }
    command
        .current_dir(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Suspend before attaching the job so hooks cannot escape during startup.
        command.creation_flags(0x08000000 | 0x00000004);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let cleanup = match GitProcessCleanup::attach(&child) {
        Ok(cleanup) => cleanup,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let mut cleanup = Some(cleanup);
    let mut stdin = child.stdin.take().ok_or("Missing Git input pipe")?;
    let input = input.unwrap_or_default().to_vec();
    let (input_tx, input_result) = mpsc::channel();
    thread::spawn(move || {
        let _ = input_tx.send(stdin.write_all(&input));
    });
    let stdout = child.stdout.take().ok_or("Missing Git output pipe")?;
    let stderr = child.stderr.take().ok_or("Missing Git error pipe")?;
    let out = PipeCapture::start(stdout);
    let err = PipeCapture::start(stderr);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                // Successful or refused Git can intentionally start a detached helper.
                if cleanup.as_mut().unwrap().disarm().is_err() {
                    // Preserve the real Git result even when OS cleanup cannot be released.
                    eprintln!("Git process cleanup could not be disarmed");
                }
                break Ok(s);
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                // Kill the group before reaping its leader: the PID cannot be reused yet.
                drop(cleanup.take());
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!(
                    "Git timed out after {} seconds.",
                    timeout.as_secs()
                ));
            }
            Err(e) => {
                drop(cleanup.take());
                let _ = child.kill();
                let _ = child.wait();
                break Err(e.to_string());
            }
        }
    };
    // On normal exit the group is disarmed; detached helpers retain their lifetime.
    drop(cleanup);
    let status = status?;
    // A background hook may retain a pipe after Git has already completed.
    // Report the actual exit status, with an explicitly incomplete capture.
    let pipe_deadline = deadline.min(Instant::now() + Duration::from_secs(1));
    let mut output = out.finish(pipe_deadline);
    let errors = err.finish(pipe_deadline);
    if !status.success() && !(allow_no_match && status.code() == Some(1)) {
        return Err(errors.text());
    }
    if !matches!(
        input_result.recv_timeout(pipe_deadline.saturating_duration_since(Instant::now())),
        Ok(Ok(()))
    ) {
        output.truncated = true;
    }
    Ok(output)
}

/// Owns only the process group created for this Git invocation.
struct GitProcessCleanup {
    #[cfg(windows)]
    _job: process_job::ProcessJob,
    #[cfg(unix)]
    pid: libc::pid_t,
    #[cfg(unix)]
    armed: bool,
}
impl GitProcessCleanup {
    fn disarm(&mut self) -> Result<()> {
        #[cfg(windows)]
        self._job.disarm()?;
        #[cfg(unix)]
        {
            self.armed = false;
        }
        Ok(())
    }
    fn attach(child: &std::process::Child) -> Result<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            let job = process_job::ProcessJob::attach(child.as_raw_handle())
                .map_err(|_| "Cannot establish Git process cleanup".to_string())?;
            resume_git_thread(child.id())?;
            Ok(Self { _job: job })
        }
        #[cfg(unix)]
        {
            Ok(Self {
                pid: child.id() as libc::pid_t,
                armed: true,
            })
        }
    }
}
#[cfg(unix)]
impl Drop for GitProcessCleanup {
    fn drop(&mut self) {
        // This child was spawned into its own group with process_group(0).
        if self.armed {
            unsafe {
                libc::kill(-self.pid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(windows)]
fn resume_git_thread(pid: u32) -> Result<()> {
    use windows::Win32::{
        Foundation::{CloseHandle, ERROR_NO_MORE_FILES, HANDLE},
        System::{Diagnostics::ToolHelp::*, Threading::*},
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    // std::process does not expose the primary thread handle on stable Rust.
    // A CREATE_SUSPENDED process has not run user code; enumerate its sole thread
    // while retaining the Child process handle (preventing process-ID reuse).
    unsafe {
        let snapshot = Handle(
            CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
                .map_err(|_| "Cannot inspect suspended Git process")?,
        );
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        Thread32First(snapshot.0, &mut entry).map_err(|_| "Cannot locate Git thread")?;
        let mut thread_id = None;
        loop {
            if entry.th32OwnerProcessID == pid && thread_id.replace(entry.th32ThreadID).is_some() {
                return Err("Unexpected threads in suspended Git process".into());
            }
            if let Err(error) = Thread32Next(snapshot.0, &mut entry) {
                if error.code() != ERROR_NO_MORE_FILES.to_hresult() {
                    return Err("Cannot enumerate suspended Git threads".into());
                }
                break;
            }
        }
        let thread = Handle(
            OpenThread(
                THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
                false,
                thread_id.ok_or("Missing suspended Git thread")?,
            )
            .map_err(|_| "Cannot open suspended Git thread")?,
        );
        if GetProcessIdOfThread(thread.0) != pid || ResumeThread(thread.0) != 1 {
            return Err("Cannot resume protected Git process".into());
        }
    }
    Ok(())
}

pub(super) const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
#[derive(Default)]
pub struct Capture {
    pub bytes: Vec<u8>,
    pub truncated: bool,
}
impl Capture {
    pub fn text(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.bytes).into_owned();
        if self.truncated {
            text.push_str("\n[Git output incomplete or truncated at 2 MiB.]\n");
        }
        text
    }
}
struct PipeCapture {
    done: mpsc::Receiver<std::io::Result<()>>,
    capture: Arc<Mutex<Option<Capture>>>,
}
impl PipeCapture {
    fn start(reader: impl Read + Send + 'static) -> Self {
        let capture = Arc::new(Mutex::new(Some(Capture::default())));
        let shared = Arc::clone(&capture);
        let (tx, done) = mpsc::channel();
        thread::spawn(move || {
            let result = drain_chunks(reader, |bytes| {
                if let Some(capture) = shared.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                    append_capture(capture, bytes);
                }
            });
            let _ = tx.send(result);
        });
        Self { done, capture }
    }
    fn finish(self, deadline: Instant) -> Capture {
        let complete = matches!(
            self.done
                .recv_timeout(deadline.saturating_duration_since(Instant::now())),
            Ok(Ok(()))
        );
        let mut capture = self
            .capture
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap_or_default();
        capture.truncated |= !complete;
        capture
    }
}
fn append_capture(capture: &mut Capture, bytes: &[u8]) {
    let room = OUTPUT_LIMIT.saturating_sub(capture.bytes.len());
    capture.truncated |= bytes.len() > room;
    capture
        .bytes
        .extend_from_slice(&bytes[..bytes.len().min(room)]);
}
fn drain_chunks(mut reader: impl Read, mut consume: impl FnMut(&[u8])) -> std::io::Result<()> {
    let mut buffer = [0; 8192];
    loop {
        let n = match reader.read(&mut buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if n == 0 {
            return Ok(());
        }
        consume(&buffer[..n]);
    }
}
#[cfg(test)]
pub(super) fn drain(reader: impl Read) -> std::io::Result<Capture> {
    let mut capture = Capture::default();
    drain_chunks(reader, |bytes| append_capture(&mut capture, bytes))?;
    Ok(capture)
}

#[cfg(test)]
#[path = "process_env_tests.rs"]
mod environment_tests;
