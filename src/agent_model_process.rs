//! Bounded, direct process capture for model discovery, run on a worker thread.
//! No shell is added, no prompt is generated, and output is never logged here.
use std::{
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Start,
    Io,
    Timeout,
    TooMuch,
}

#[derive(Debug)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
}

pub struct Limits {
    pub timeout: Duration,
    /// Combined UTF-16 units, as in the source JavaScript string budget.
    pub output: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            output: crate::agent_model_probe_parse::MAX_OUTPUT,
        }
    }
}

/// Handles UTF-8 split across read boundaries without double-counting surrogate
/// pairs or replacement characters. Invalid UTF-8 has String::from_utf8_lossy semantics.
#[derive(Default)]
pub(crate) struct Units {
    carry: Vec<u8>,
}
impl Units {
    pub(crate) fn push(&mut self, bytes: &[u8], eof: bool) -> usize {
        self.carry.extend_from_slice(bytes);
        let mut remaining = self.carry.as_slice();
        let mut units = 0;
        loop {
            match std::str::from_utf8(remaining) {
                Ok(s) => {
                    units += s.encode_utf16().count();
                    remaining = &[];
                    break;
                }
                Err(e) => {
                    units += std::str::from_utf8(&remaining[..e.valid_up_to()])
                        .unwrap()
                        .encode_utf16()
                        .count();
                    remaining = &remaining[e.valid_up_to()..];
                    if let Some(n) = e.error_len() {
                        units += 1;
                        remaining = &remaining[n..];
                    } else {
                        if eof {
                            units += 1;
                            remaining = &[];
                        }
                        break;
                    }
                }
            }
        }
        self.carry = remaining.to_vec();
        units
    }
}

async fn read<R: AsyncRead + Unpin>(
    mut pipe: R,
    total: Arc<AtomicUsize>,
    limit: usize,
) -> Result<String, Error> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    let mut units = Units::default();
    loop {
        let n = pipe.read(&mut buffer).await.map_err(|_| Error::Io)?;
        let count = units.push(&buffer[..n], n == 0);
        let before = total.fetch_add(count, Ordering::Relaxed);
        if count > limit.saturating_sub(before) {
            return Err(Error::TooMuch);
        }
        bytes.extend_from_slice(&buffer[..n]);
        if n == 0 {
            return Ok(String::from_utf8_lossy(&bytes).into_owned());
        }
    }
}

/// Caller supplies a resolved executable, argument array, working directory and
/// environment. Never call from the egui thread; this owns its Tokio runtime.
pub fn run(mut command: Command, stdin: Option<&str>, limits: Limits) -> Result<Output, Error> {
    crate::agent_env::clean_command(command.as_std_mut());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Error::Start)?;
    runtime.block_on(async {
        command
            .kill_on_drop(true)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command.spawn().map_err(|_| Error::Start)?;
        let total = Arc::new(AtomicUsize::new(0));
        let mut readers = tokio::task::JoinSet::new();
        let stdout = child.stdout.take().ok_or(Error::Io)?;
        let stderr = child.stderr.take().ok_or(Error::Io)?;
        let out_total = Arc::clone(&total);
        readers.spawn(async move { (true, read(stdout, out_total, limits.output).await) });
        readers.spawn(async move { (false, read(stderr, total, limits.output).await) });
        let result = tokio::time::timeout(limits.timeout, async {
            if let Some(input) = stdin
                && let Some(mut pipe) = child.stdin.take()
            {
                // A CLI may close input before responding; match source's ignored EPIPE.
                let _ = pipe.write_all(input.as_bytes()).await;
                drop(pipe);
            }
            let mut output = Output {
                stdout: String::new(),
                stderr: String::new(),
                code: None,
            };
            let mut finished = false;
            loop {
                while let Some(item) = readers.try_join_next() {
                    let (is_stdout, text) = item.map_err(|_| Error::Io)?;
                    let text = text?;
                    if is_stdout {
                        output.stdout = text;
                    } else {
                        output.stderr = text;
                    }
                }
                if !finished && let Some(status) = child.try_wait().map_err(|_| Error::Io)? {
                    finished = true;
                    output.code = status.code();
                }
                if finished && readers.is_empty() {
                    return Ok(output);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or(Err(Error::Timeout));
        readers.abort_all();
        if result.is_err() {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
        }
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn utf16_budget_matches_lossy_decoding_at_every_chunk_boundary() {
        for data in [
            "a😀é中文".as_bytes(),
            &[0xff, 0xe2, 0x82, b'a', 0xf0, 0x90, 0x80],
        ] {
            for split in 0..=data.len() {
                let mut units = Units::default();
                let count = units.push(&data[..split], false) + units.push(&data[split..], true);
                assert_eq!(count, String::from_utf8_lossy(data).encode_utf16().count());
                assert!(units.carry.is_empty());
            }
        }
    }

    fn fixture(mode: &str) -> Command {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "agent_model_process::tests::child_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("TESSEL_MODEL_PROBE_TEST", mode);
        cmd
    }

    #[test]
    fn readers_share_the_exact_utf16_budget_and_preserve_split_characters() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let input = format!("{}😀é", "a".repeat(8191));
            let limit = input.encode_utf16().count();
            let total = Arc::new(AtomicUsize::new(0));
            assert_eq!(
                read(input.as_bytes(), Arc::clone(&total), limit)
                    .await
                    .unwrap(),
                input
            );
            assert_eq!(
                read(b"x".as_slice(), total, limit).await,
                Err(Error::TooMuch)
            );
        });
    }

    #[test]
    fn direct_process_receives_stdin_and_keeps_both_streams_and_exit_code() {
        let output = run(
            fixture("echo"),
            Some("synthetic-list-request\n"),
            Limits::default(),
        )
        .unwrap();
        assert_eq!(output.code, Some(0));
        assert!(output.stdout.contains("synthetic-list-request"));
        assert!(output.stderr.contains("synthetic-stderr"));
        let output = run(fixture("failure"), None, Limits::default()).unwrap();
        assert_eq!(output.code, Some(7));
    }

    #[test]
    fn model_process_filters_parent_session_capabilities_without_losing_account_context() {
        let mut command = fixture("environment");
        command
            .env("TESSEL_TEAM_MCP_KEY", "synthetic-secret")
            .env("TESSEL_TEAM_MCP_PANE", "synthetic-pane")
            .env("TESSEL_TEAM_MCP_GENERATION", "synthetic-generation")
            .env("CLAUDE_CODE_SESSION_ID", "synthetic-session")
            .env("OPENAI_API_KEY", "synthetic-account")
            .env("CODEX_HOME", "synthetic-profile");
        let output = run(command, None, Limits::default()).unwrap();
        assert_eq!(output.code, Some(0));
        assert!(output.stdout.contains("1 passed"));
    }

    #[test]
    fn output_limit_and_timeout_stop_synthetic_processes() {
        let limits = || Limits {
            timeout: Duration::from_secs(2),
            output: 1024,
        };
        assert!(matches!(
            run(fixture("flood"), None, limits()),
            Err(Error::TooMuch)
        ));
        let started = std::time::Instant::now();
        assert!(matches!(
            run(
                fixture("sleep"),
                None,
                Limits {
                    timeout: Duration::from_millis(150),
                    output: 1024
                }
            ),
            Err(Error::Timeout)
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    #[ignore = "Synthetic child fixture invoked only by the process tests"]
    fn child_fixture() {
        match std::env::var("TESSEL_MODEL_PROBE_TEST").as_deref() {
            Ok("echo") => {
                let mut input = String::new();
                std::io::stdin().read_to_string(&mut input).unwrap();
                print!("{input}");
                eprint!("synthetic-stderr");
            }
            Ok("environment") => {
                for name in [
                    "TESSEL_TEAM_MCP_KEY",
                    "TESSEL_TEAM_MCP_PANE",
                    "TESSEL_TEAM_MCP_GENERATION",
                    "CLAUDE_CODE_SESSION_ID",
                ] {
                    assert!(
                        std::env::var_os(name).is_none(),
                        "Parent capability reached the synthetic child"
                    );
                }
                assert!(std::env::var("OPENAI_API_KEY").as_deref() == Ok("synthetic-account"));
                assert!(std::env::var("CODEX_HOME").as_deref() == Ok("synthetic-profile"));
                assert!(std::env::var_os("PATH").is_some());
            }
            Ok("failure") => std::process::exit(7),
            Ok("flood") => {
                let mut out = std::io::stdout().lock();
                for _ in 0..2048 {
                    if out.write_all(&[b'x'; 8192]).is_err() {
                        break;
                    }
                }
            }
            Ok("sleep") => std::thread::sleep(Duration::from_secs(5)),
            _ => (),
        }
    }
}
