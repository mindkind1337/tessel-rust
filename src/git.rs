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
    fn drain(mut reader: impl Read) -> Vec<u8> {
        let mut kept = Vec::new();
        let mut buffer = [0; 8192];
        while let Ok(n) = reader.read(&mut buffer) {
            if n == 0 {
                break;
            }
            let room = (2 * 1024 * 1024usize).saturating_sub(kept.len());
            kept.extend_from_slice(&buffer[..n.min(room)]);
        }
        kept
    }
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
        .map_err(|_| "Git output timed out (a child may still hold the pipe)")?;
    let errors = err
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| "Git error output timed out")?;
    if !status.success() {
        return Err(String::from_utf8_lossy(&errors).into_owned());
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
