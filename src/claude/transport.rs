use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
};
const MAX_FRAME: usize = 2 * 1024 * 1024;
pub(super) fn executable() -> Option<PathBuf> {
    for directory in std::env::split_paths(&std::env::var_os("PATH")?) {
        for relative in if cfg!(windows) {
            vec![
                "claude.exe",
                "node_modules/@anthropic-ai/claude-code/bin/claude.exe",
            ]
        } else {
            vec!["claude"]
        } {
            let path = directory.join(relative);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}
pub(super) struct Transport {
    child: Child,
    writer: Option<SyncSender<Value>>,
    pub reader: Receiver<Result<Value, String>>,
    #[cfg(windows)]
    job: Option<Job>,
}
impl Transport {
    pub fn child_id(&self) -> u32 {
        self.child.id()
    }
    pub fn start(
        cwd: &Path,
        session: &str,
        resume: bool,
        model: &str,
        effort: &str,
    ) -> Result<Self, String> {
        let exe = executable().ok_or("Install Claude Code and sign in with its CLI first")?;
        Self::spawn(&mut Self::command(
            &exe, cwd, session, resume, model, effort,
        ))
    }
    fn command(
        exe: &Path,
        cwd: &Path,
        session: &str,
        resume: bool,
        model: &str,
        effort: &str,
    ) -> Command {
        let mut command = Command::new(exe);
        command.args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--replay-user-messages",
            "--permission-prompt-tool",
            "stdio",
            "--permission-mode",
            "default",
            "--safe-mode",
            "--setting-sources",
            "",
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
        ]);
        command
            .arg(if resume { "--resume" } else { "--session-id" })
            .arg(session);
        if !model.is_empty() {
            command.arg("--model").arg(model);
        }
        if !effort.is_empty() {
            command.arg("--effort").arg(effort);
        }
        command.current_dir(cwd);
        command
    }
    fn spawn(command: &mut Command) -> Result<Self, String> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("Cannot start Claude Code (OS error {:?})", e.raw_os_error()))?;
        #[cfg(windows)]
        let job = match Job::attach(&child) {
            Ok(job) => job,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let mut stdin = child.stdin.take().ok_or("No Claude input pipe")?;
        let stdout = child.stdout.take().ok_or("No Claude output pipe")?;
        let (writer, rx) = mpsc::sync_channel::<Value>(32);
        let (tx, reader) = mpsc::sync_channel(32);
        thread::spawn(move || {
            while let Ok(value) = rx.recv() {
                let Ok(mut bytes) = serde_json::to_vec(&value) else {
                    break;
                };
                bytes.push(b'\n');
                if stdin.write_all(&bytes).and_then(|_| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut bytes = Vec::new();
                let result = match reader
                    .by_ref()
                    .take((MAX_FRAME + 1) as u64)
                    .read_until(b'\n', &mut bytes)
                {
                    Ok(0) => Err("Claude process closed the connection".into()),
                    Ok(_) if bytes.len() > MAX_FRAME => Err("Claude frame exceeded 2 MiB".into()),
                    Ok(_) => serde_json::from_slice(&bytes)
                        .map_err(|_| "Invalid JSON frame from Claude".into()),
                    Err(_) => Err("Claude output pipe failed".into()),
                };
                let failed = result.is_err();
                if tx.send(result).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            writer: Some(writer),
            reader,
            #[cfg(windows)]
            job: Some(job),
        })
    }
    pub fn send(&self, value: Value) -> Result<(), String> {
        self.writer
            .as_ref()
            .ok_or("Claude disconnected")?
            .try_send(value)
            .map_err(|_| "Claude input is busy or disconnected".into())
    }
}
impl Drop for Transport {
    fn drop(&mut self) {
        self.writer.take();
        let (_, empty) = mpsc::channel();
        self.reader = empty;
        #[cfg(windows)]
        self.job.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[cfg(windows)]
struct Job(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Job {
    fn attach(child: &Child) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows::{
            Win32::{
                Foundation::{CloseHandle, HANDLE},
                System::JobObjects::*,
            },
            core::PCWSTR,
        };
        unsafe {
            let handle = CreateJobObjectW(None, PCWSTR::null())
                .map_err(|_| "Cannot isolate Claude process tree")?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
            .and_then(|_| AssignProcessToJobObject(handle, HANDLE(child.as_raw_handle())))
            .is_err()
            {
                let _ = CloseHandle(handle);
                return Err("Cannot isolate Claude process tree".into());
            }
            Ok(Self(handle))
        }
    }
}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_effort_and_resume_do_not_weaken_permissions() {
        for resume in [false, true] {
            let command = Transport::command(
                Path::new("synthetic-cli"),
                Path::new("."),
                "synthetic-id",
                resume,
                "claude-opus-4-8[1m]",
                "high",
            );
            let args: Vec<_> = command.get_args().map(|v| v.to_str().unwrap()).collect();
            for pair in [
                ["--model", "claude-opus-4-8[1m]"],
                ["--effort", "high"],
                ["--permission-mode", "default"],
                ["--setting-sources", ""],
                [
                    if resume { "--resume" } else { "--session-id" },
                    "synthetic-id",
                ],
            ] {
                assert!(args.windows(2).any(|w| w == pair));
            }
            assert!(args.contains(&"--safe-mode"));
            assert!(args.contains(&"--strict-mcp-config"));
        }
        let command = Transport::command(
            Path::new("synthetic-cli"),
            Path::new("."),
            "id",
            false,
            "",
            "",
        );
        assert!(
            !command
                .get_args()
                .any(|a| a == "--effort" || a == "--model")
        );
    }
}
