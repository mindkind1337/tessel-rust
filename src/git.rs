//! Bounded, on-demand Git operations. No automatic recursive status polling.
use crate::workspace::Result;
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub fn run(root: &Path, args: &[&str], input: Option<&[u8]>) -> Result<String> {
    run_bytes(root, args, input).map(|output| output.text())
}

/// Preserve NUL-delimited paths and expose truncation separately from stdout.
/// Structured callers must disable bulk actions when `truncated` is true.
pub fn run_bytes(root: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Capture> {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().ok_or("Missing Git input pipe")?;
    let input = input.unwrap_or_default().to_vec();
    thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let stdout = child.stdout.take().ok_or("Missing Git output pipe")?;
    let stderr = child.stderr.take().ok_or("Missing Git error pipe")?;
    let (out_tx, out) = mpsc::channel();
    let (err_tx, err) = mpsc::channel();
    thread::spawn(move || {
        let _ = out_tx.send(drain(stdout));
    });
    thread::spawn(move || {
        let _ = err_tx.send(drain(stderr));
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Ok(s),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("Git timed out after 20 seconds.".to_string());
            }
            Err(e) => break Err(e.to_string()),
        }
    };
    // Do not wait on pipe readers after a timeout: a hook may retain inherited handles.
    let status = status?;
    let output = out
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| "Git output timed out (a child may still hold the pipe)")?
        .map_err(|error| format!("Git output could not be read: {error}"))?;
    let errors = err
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| "Git error output timed out")?
        .map_err(|error| format!("Git error output could not be read: {error}"))?;
    if !status.success() {
        return Err(errors.text());
    }
    Ok(output)
}

const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
pub struct Capture {
    pub bytes: Vec<u8>,
    pub truncated: bool,
}
impl Capture {
    fn text(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.bytes).into_owned();
        if self.truncated {
            text.push_str("\n[Git output truncated at 2 MiB.]\n");
        }
        text
    }
}
fn drain(mut reader: impl Read) -> std::io::Result<Capture> {
    let mut capture = Capture {
        bytes: Vec::new(),
        truncated: false,
    };
    let mut buffer = [0; 8192];
    loop {
        let n = match reader.read(&mut buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if n == 0 {
            return Ok(capture);
        }
        let room = OUTPUT_LIMIT.saturating_sub(capture.bytes.len());
        capture.truncated |= n > room;
        capture.bytes.extend_from_slice(&buffer[..n.min(room)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_output_is_drained_but_never_presented_as_complete() {
        let data = vec![b'x'; OUTPUT_LIMIT + 37];
        let mut reader = std::io::Cursor::new(data);
        let capture = drain(&mut reader).unwrap();
        assert_eq!(capture.bytes.len(), OUTPUT_LIMIT);
        assert_eq!(reader.position(), (OUTPUT_LIMIT + 37) as u64);
        assert!(capture.truncated);
        assert!(
            capture
                .text()
                .ends_with("[Git output truncated at 2 MiB.]\n")
        );
        let exact = drain(std::io::Cursor::new(vec![b'x'; OUTPUT_LIMIT])).unwrap();
        assert!(!exact.truncated);
        assert_eq!(exact.text().len(), OUTPUT_LIMIT);
    }
    #[test]
    fn pipe_read_failure_is_not_silently_converted_to_success() {
        struct Failed;
        impl Read for Failed {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("synthetic pipe failure"))
            }
        }
        assert!(drain(Failed).is_err());
    }
    #[test]
    fn real_repo_status_and_diff() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"], None).unwrap();
        std::fs::write(dir.path().join("sample.txt"), "hello\n").unwrap();
        let status = run(
            dir.path(),
            &["status", "--short", "--untracked-files=normal"],
            None,
        )
        .unwrap();
        assert!(status.contains("sample.txt"));
        run(dir.path(), &["add", "--", "sample.txt"], None).unwrap();
        let diff = run(
            dir.path(),
            &["diff", "--cached", "--no-ext-diff", "--no-textconv"],
            None,
        )
        .unwrap();
        assert!(diff.contains("+hello"));
    }

    #[test]
    fn porcelain_output_preserves_nul_delimiters_and_literal_paths() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"], None).unwrap();
        let name = "untracked file é.txt";
        std::fs::write(dir.path().join(name), "synthetic\n").unwrap();
        let status = run_bytes(
            dir.path(),
            &[
                "status",
                "--porcelain=v2",
                "-z",
                "--branch",
                "--untracked-files=normal",
            ],
            None,
        )
        .unwrap();
        assert!(!status.truncated);
        let expected = format!("? {name}");
        assert!(
            status
                .bytes
                .split(|b| *b == 0)
                .any(|record| record == expected.as_bytes())
        );
        assert_eq!(status.bytes.last(), Some(&0));
        run(dir.path(), &["add", "--", name], None).unwrap();
        let staged = run_bytes(dir.path(), &["status", "--porcelain=v2", "-z"], None).unwrap();
        assert!(!staged.truncated);
        assert!(staged.bytes.starts_with(b"1 A. "));
        assert!(staged.bytes.ends_with(format!(" {name}\0").as_bytes()));
    }
}
