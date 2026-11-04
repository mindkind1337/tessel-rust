//! Bounded installer exec channels. Call from a worker, never an egui frame.
use crate::native_host_install::{CommandOutput, HostTransport, MAX_STDOUT, TransportFailed};
use std::{
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Connector {
    // Immutable for this Client, including the installed daemon's versioned
    // connect path. A different installation requires a new Client: terminal
    // epoch invalidation relies on reconnections preserving this scope.
    args: Vec<String>,
}
struct Guard {
    child: Option<std::process::Child>,
    #[cfg(windows)]
    job: Option<crate::remote::ProcessJob>,
}
impl Drop for Guard {
    fn drop(&mut self) {
        #[cfg(windows)]
        drop(self.job.take());
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = std::thread::Builder::new()
                .name("host-reap".into())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
    }
}
impl tessel_host_client::Connector for Connector {
    fn connect(&self) -> std::io::Result<tessel_host_client::Pipe> {
        let mut command = std::process::Command::new("ssh");
        command.args(&self.args);
        open_process(&mut command)
    }
}
fn open_process(command: &mut std::process::Command) -> std::io::Result<tessel_host_client::Pipe> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command.spawn()?;
    let mut guard = Guard {
        child: Some(child),
        #[cfg(windows)]
        job: None,
    };
    let child = guard.child.as_mut().expect("newly spawned child");
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        guard.job = Some(
            crate::remote::ProcessJob::attach(child.as_raw_handle())
                .map_err(std::io::Error::other)?,
        );
    }
    let reader = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("No SSH output"))?;
    let writer = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("No SSH input"))?;
    Ok(tessel_host_client::Pipe {
        reader: Box::new(reader),
        writer: Box::new(writer),
        guard: Some(Box::new(guard)),
    })
}
pub fn connector(
    alias: &str,
    command: &str,
) -> Result<Box<dyn tessel_host_client::Connector>, TransportFailed> {
    Ok(Box::new(Connector {
        args: verified_args(alias, command)?,
    }))
}

pub fn verified_args(alias: &str, command: &str) -> Result<Vec<String>, TransportFailed> {
    crate::workspace::validate_host(alias).map_err(|_| TransportFailed)?;
    if command.is_empty() || command.len() > 8192 || command.chars().any(char::is_control) {
        return Err(TransportFailed);
    }
    Ok([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=2",
        "-o",
        "ControlMaster=no",
        "-o",
        "ControlPath=none",
        "-o",
        "ClearAllForwardings=yes",
        "--",
        alias,
        command,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect())
}

pub struct SshTransport {
    alias: String,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    runtime: tokio::runtime::Runtime,
}
impl SshTransport {
    pub fn new(alias: String, cancel: Arc<AtomicBool>) -> Result<Self, TransportFailed> {
        crate::workspace::validate_host(&alias).map_err(|_| TransportFailed)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| TransportFailed)?;
        Ok(Self {
            alias,
            cancel,
            deadline: Instant::now() + Duration::from_secs(120),
            runtime,
        })
    }
    fn execute(
        &self,
        command: &str,
        stdin: &[u8],
        limit: Duration,
        cancel: &AtomicBool,
    ) -> Result<CommandOutput, TransportFailed> {
        let mut process = tokio::process::Command::new("ssh");
        process.args(verified_args(&self.alias, command)?);
        self.runtime
            .block_on(run_process(&mut process, stdin, limit, cancel))
    }
}
impl HostTransport for SshTransport {
    fn run(&mut self, command: &str, stdin: &[u8]) -> Result<CommandOutput, TransportFailed> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        self.execute(
            command,
            stdin,
            remaining.min(Duration::from_secs(60)),
            &self.cancel,
        )
    }
    fn cleanup(&mut self, command: &str) -> Result<CommandOutput, TransportFailed> {
        // Only the installer's validated abort command is passed here. Cancellation
        // ends the upload but must still allow a short attempt to remove its .part.
        self.execute(
            command,
            &[],
            Duration::from_secs(3),
            &AtomicBool::new(false),
        )
    }
}

async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn run_process(
    command: &mut tokio::process::Command,
    input: &[u8],
    limit: Duration,
    cancel: &AtomicBool,
) -> Result<CommandOutput, TransportFailed> {
    if limit.is_zero() || cancel.load(Ordering::Acquire) || input.len() > 64 * 1024 * 1024 {
        return Err(TransportFailed);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().map_err(|_| TransportFailed)?;
    #[cfg(windows)]
    let job = crate::remote::ProcessJob::attach(child.raw_handle().ok_or(TransportFailed)?)
        .map_err(|_| TransportFailed)?;
    let mut writer = child.stdin.take().ok_or(TransportFailed)?;
    let reader = child.stdout.take().ok_or(TransportFailed)?;
    let result = {
        let exchange = async {
            let writing = async {
                // An upload rejected before reading stdin closes its pipe early.
                // Preserve the exit status; a partial upload claiming success is
                // independently rejected by the installer's remote digest check.
                let _ = writer.write_all(input).await;
                let _ = writer.shutdown().await;
                drop(writer);
                Ok::<_, TransportFailed>(())
            };
            let reading = async {
                let mut bytes = Vec::new();
                reader
                    .take(MAX_STDOUT as u64 + 1)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|_| TransportFailed)?;
                if bytes.len() > MAX_STDOUT {
                    return Err(TransportFailed);
                }
                Ok(bytes)
            };
            let waiting = async { child.wait().await.map_err(|_| TransportFailed) };
            let (_, stdout, status) = tokio::try_join!(writing, reading, waiting)?;
            Ok(CommandOutput {
                success: status.success(),
                stdout,
            })
        };
        tokio::select! {
            biased;
            _ = cancelled(cancel) => Err(TransportFailed),
            result = tokio::time::timeout(limit,exchange) => result.map_err(|_|TransportFailed).and_then(|result|result),
        }
    };
    #[cfg(windows)]
    drop(job);
    if result.is_err() {
        let _ = tokio::time::timeout(Duration::from_secs(2), child.kill()).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    fn fixture(mode: &str) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "native_host_transport::tests::child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("TESSEL_HOST_TRANSPORT_FIXTURE", mode);
        command
    }
    #[test]
    #[ignore = "Synthetic child invoked by bounded transport tests"]
    fn child_fixture() {
        match std::env::var("TESSEL_HOST_TRANSPORT_FIXTURE")
            .unwrap()
            .as_str()
        {
            "copy" => {
                let mut bytes = Vec::new();
                std::io::stdin().read_to_end(&mut bytes).unwrap();
                std::io::stdout()
                    .write_all(format!("received:{}", bytes.len()).as_bytes())
                    .unwrap();
            }
            "flood" => {
                let _ = std::io::stdout().write_all(&vec![b'x'; MAX_STDOUT + 1024]);
            }
            "sleep" => std::thread::sleep(Duration::from_secs(30)),
            "tree" => {
                let mut command = std::process::Command::new(std::env::current_exe().unwrap());
                command
                    .args([
                        "--exact",
                        "native_host_transport::tests::child_fixture",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env("TESSEL_HOST_TRANSPORT_FIXTURE", "sleep")
                    .stdin(Stdio::null())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::null());
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    command.creation_flags(0x08000000);
                }
                let mut child = command.spawn().unwrap();
                println!("descendant-ready");
                std::io::stdout().flush().unwrap();
                let _ = child.wait();
            }
            "exit" => std::process::exit(7),
            _ => panic!("unknown fixture"),
        }
    }
    #[test]
    fn ssh_options_preserve_trust_and_the_single_fixed_remote_command() {
        let command = "\"$HOME/.tessel-server/host/bin/tessel-host-0.1.0-x86_64\" connect --stdio";
        let args = verified_args("owner@fixture", command).unwrap();
        assert_eq!(&args[args.len() - 3..], ["--", "owner@fixture", command]);
        for required in [
            "StrictHostKeyChecking=yes",
            "BatchMode=yes",
            "ControlPath=none",
            "ClearAllForwardings=yes",
        ] {
            assert!(args.iter().any(|v| v == required));
        }
        for alias in ["-oProxyCommand=bad", "host;command", "host name"] {
            assert!(verified_args(alias, command).is_err());
        }
        assert!(verified_args("fixture", "bad\ncommand").is_err());
    }
    #[test]
    fn connector_drop_closes_descendant_output_without_waiting_on_caller() {
        use std::io::BufRead;
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "native_host_transport::tests::child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("TESSEL_HOST_TRANSPORT_FIXTURE", "tree");
        let pipe = open_process(&mut command).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(pipe.reader).lines() {
                if line.unwrap_or_default().contains("descendant-ready") {
                    let _ = sender.send(false);
                }
            }
            let _ = sender.send(true);
        });
        assert!(!receiver.recv_timeout(Duration::from_secs(10)).unwrap());
        let start = Instant::now();
        drop(pipe.guard);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(receiver.recv_timeout(Duration::from_secs(5)).unwrap());
        reader.join().unwrap();
    }
    #[test]
    fn real_child_accepts_binary_stdin_and_reports_nonzero_status() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let cancel = AtomicBool::new(false);
        let bytes = vec![42; 2 * 1024 * 1024];
        let result = runtime
            .block_on(run_process(
                &mut fixture("copy"),
                &bytes,
                Duration::from_secs(10),
                &cancel,
            ))
            .unwrap();
        assert!(result.success);
        assert!(
            String::from_utf8(result.stdout)
                .unwrap()
                .contains("received:2097152")
        );
        assert!(
            !runtime
                .block_on(run_process(
                    &mut fixture("exit"),
                    &bytes,
                    Duration::from_secs(10),
                    &cancel
                ))
                .unwrap()
                .success
        );
    }
    #[test]
    fn output_flood_stalled_writer_timeout_and_cancel_are_bounded() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(
            runtime
                .block_on(run_process(
                    &mut fixture("flood"),
                    &[],
                    Duration::from_secs(5),
                    &cancel
                ))
                .is_err()
        );
        let started = Instant::now();
        assert!(
            runtime
                .block_on(run_process(
                    &mut fixture("sleep"),
                    &vec![1; 2 * 1024 * 1024],
                    Duration::from_millis(150),
                    &cancel
                ))
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        let flag = cancel.clone();
        let cancelling = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::Release);
        });
        let started = Instant::now();
        assert!(
            runtime
                .block_on(run_process(
                    &mut fixture("sleep"),
                    &[],
                    Duration::from_secs(10),
                    &cancel
                ))
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        cancelling.join().unwrap();
    }
}
