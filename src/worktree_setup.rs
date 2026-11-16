//! Explicit owner-only preparation of a newly created worktree. No saved receipt
//! or external issue can grant execution authority. Output is never retained.
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Missing,
    Completed,
    Failed { ran: bool, error: &'static str },
}
const CHECK: &str = "Could not check the worktree setup script.";
const REGULAR: &str = "The worktree setup script must be a regular tracked file.";
const UNSAFE: &str = "The worktree setup script could not be started safely.";
const FAILED: &str = "The worktree setup script failed or exceeded its execution limit.";

pub(crate) fn run(root: &Path, base_commit: &str) -> Outcome {
    run_with(root, base_commit, |root, hook| {
        let mut command = Command::new(if cfg!(windows) {
            "powershell.exe"
        } else {
            "pwsh"
        });
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ]);
        command.arg(hook).current_dir(root);
        execute(command, Duration::from_secs(60), 256 * 1024)
    })
}

fn run_with(root: &Path, base_commit: &str, execute: impl FnOnce(&Path, &Path) -> bool) -> Outcome {
    match checked_hook(root, base_commit) {
        Ok(None) => Outcome::Missing,
        Ok(Some((root, hook))) => {
            if execute(&root, &hook) {
                Outcome::Completed
            } else {
                Outcome::Failed {
                    ran: true,
                    error: FAILED,
                }
            }
        }
        Err(error) => Outcome::Failed { ran: false, error },
    }
}

fn checked_hook(
    root: &Path,
    base_commit: &str,
) -> Result<Option<(PathBuf, PathBuf)>, &'static str> {
    if !matches!(base_commit.len(), 40 | 64) || !base_commit.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(CHECK);
    }
    let entry = crate::git::run_bytes_with_timeout(
        root,
        &["ls-tree", "-z", base_commit, "--", ".tessel/setup.ps1"],
        None,
        Duration::from_secs(10),
    )
    .map_err(|_| CHECK)?;
    if entry.truncated || entry.bytes.len() > 16 * 1024 {
        return Err(CHECK);
    }
    if entry.bytes.is_empty() {
        return Ok(None);
    }
    let entry = std::str::from_utf8(&entry.bytes).map_err(|_| REGULAR)?;
    let object = entry
        .strip_prefix("100644 blob ")
        .or_else(|| entry.strip_prefix("100755 blob "))
        .and_then(|s| s.strip_suffix("\t.tessel/setup.ps1\0"))
        .ok_or(REGULAR)?;
    if !matches!(object.len(), 40 | 64) || !object.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(REGULAR);
    }
    for (path, directory) in [
        (root.to_owned(), true),
        (root.join(".tessel"), true),
        (root.join(".tessel/setup.ps1"), false),
    ] {
        let meta = fs::symlink_metadata(&path).map_err(|_| UNSAFE)?;
        if linked(&meta) || (directory && !meta.is_dir()) || (!directory && !meta.is_file()) {
            return Err(UNSAFE);
        }
    }
    let root = fs::canonicalize(root).map_err(|_| UNSAFE)?;
    let hook = fs::canonicalize(root.join(".tessel/setup.ps1")).map_err(|_| UNSAFE)?;
    if !hook.starts_with(&root) {
        return Err(UNSAFE);
    }
    Ok(Some((root, hook)))
}
fn linked(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

/// Drain both pipes into fixed buffers. Only byte counts and completion survive.
fn drain(reader: impl Read + Send + 'static, total: Arc<AtomicUsize>, done: mpsc::Sender<bool>) {
    thread::spawn(move || {
        let mut reader = reader;
        let mut buffer = [0u8; 8192];
        let ok = loop {
            match reader.read(&mut buffer) {
                Ok(0) => break true,
                Ok(n) => {
                    let _ = total.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                        Some(v.saturating_add(n))
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break false,
            }
        };
        let _ = done.send(ok);
    });
}

fn execute(mut command: Command, timeout: Duration, max_output: usize) -> bool {
    crate::agent_env::clean_command(&mut command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00000004); // hidden, suspended until job attachment
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let mut cleanup = match crate::git::GitProcessCleanup::attach(&child) {
        Ok(cleanup) => cleanup,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }
    };
    let total = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = mpsc::channel();
    drain(child.stdout.take().unwrap(), total.clone(), tx.clone());
    drain(child.stderr.take().unwrap(), total.clone(), tx);
    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut pipes = 0;
    let success = loop {
        let mut io_failed = false;
        for ok in rx.try_iter() {
            pipes += 1;
            io_failed |= !ok;
        }
        if io_failed || total.load(Ordering::Relaxed) > max_output || Instant::now() >= deadline {
            break false;
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(s) => status = s,
                Err(_) => break false,
            }
        }
        if let Some(status) = status {
            if !status.success() {
                break false;
            }
            if pipes == 2 {
                break true;
            }
        }
        thread::sleep(Duration::from_millis(10));
    };
    if success && cleanup.disarm().is_ok() {
        return true;
    }
    drop(cleanup); // Kill descendants before reaping the leader on failure.
    let _ = child.kill();
    let _ = child.wait();
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| crate::git::run(dir.path(), args, None).unwrap();
        run(&["init", "-q"]);
        run(&["config", "user.name", "Synthetic"]);
        run(&["config", "user.email", "fixture@example.invalid"]);
        run(&["config", "commit.gpgsign", "false"]);
        let hooks = dir.path().join("empty-hooks");
        fs::create_dir(&hooks).unwrap();
        run(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
        run(&["commit", "--allow-empty", "-m", "initial"]);
        dir
    }
    fn commit(root: &Path) -> String {
        crate::git::run(root, &["add", "--", ".tessel/setup.ps1"], None).unwrap();
        crate::git::run(root, &["commit", "-m", "setup"], None).unwrap();
        crate::git::run(root, &["rev-parse", "HEAD"], None)
            .unwrap()
            .trim()
            .into()
    }
    #[test]
    fn worktree_setup_missing_and_untracked_never_execute() {
        let dir = repo();
        let head = crate::git::run(dir.path(), &["rev-parse", "HEAD"], None).unwrap();
        assert_eq!(
            run_with(dir.path(), head.trim(), |_, _| panic!(
                "unexpected execution"
            )),
            Outcome::Missing
        );
        fs::create_dir(dir.path().join(".tessel")).unwrap();
        fs::write(dir.path().join(".tessel/setup.ps1"), "uncommitted fixture").unwrap();
        assert_eq!(
            run_with(dir.path(), head.trim(), |_, _| panic!(
                "unexpected execution"
            )),
            Outcome::Missing
        );
        assert!(matches!(
            run_with(dir.path(), "--all", |_, _| panic!("unexpected execution")),
            Outcome::Failed { ran: false, .. }
        ));
    }
    #[test]
    fn worktree_setup_selected_commit_and_failure_keep_the_copy() {
        let dir = repo();
        fs::create_dir(dir.path().join(".tessel")).unwrap();
        fs::write(dir.path().join(".tessel/setup.ps1"), "selected fixture").unwrap();
        let base = commit(dir.path());
        fs::write(dir.path().join(".tessel/setup.ps1"), "newer source fixture").unwrap();
        commit(dir.path());
        let parent = tempfile::tempdir().unwrap();
        let copy = parent.path().join("copy");
        crate::git::run(
            dir.path(),
            &["worktree", "add", "--detach", copy.to_str().unwrap(), &base],
            None,
        )
        .unwrap();
        assert_eq!(
            run_with(&copy, &base, |cwd, hook| {
                assert_eq!(cwd, fs::canonicalize(&copy).unwrap());
                assert_eq!(fs::read_to_string(hook).unwrap(), "selected fixture");
                false
            }),
            Outcome::Failed {
                ran: true,
                error: FAILED
            }
        );
        assert!(copy.join(".tessel/setup.ps1").is_file());
        assert_eq!(run_with(&copy, &base, |_, _| true), Outcome::Completed);
    }
    #[test]
    fn worktree_setup_git_symlink_mode_rejected_even_with_regular_checkout() {
        let dir = repo();
        fs::create_dir(dir.path().join(".tessel")).unwrap();
        fs::write(dir.path().join(".tessel/setup.ps1"), "../../outside").unwrap();
        let oid = crate::git::run(
            dir.path(),
            &["hash-object", "-w", "--stdin"],
            Some(b"../../outside"),
        )
        .unwrap();
        crate::git::run(
            dir.path(),
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                "120000",
                oid.trim(),
                ".tessel/setup.ps1",
            ],
            None,
        )
        .unwrap();
        crate::git::run(dir.path(), &["commit", "-m", "link mode"], None).unwrap();
        let head = crate::git::run(dir.path(), &["rev-parse", "HEAD"], None).unwrap();
        assert_eq!(
            run_with(dir.path(), head.trim(), |_, _| panic!(
                "unexpected execution"
            )),
            Outcome::Failed {
                ran: false,
                error: REGULAR
            }
        );
    }
    #[test]
    fn worktree_setup_missing_or_non_regular_checkout_never_executes() {
        let dir = repo();
        fs::create_dir(dir.path().join(".tessel")).unwrap();
        let hook = dir.path().join(".tessel/setup.ps1");
        fs::write(&hook, "synthetic tracked script").unwrap();
        let head = commit(dir.path());
        fs::remove_file(&hook).unwrap();
        assert_eq!(
            run_with(dir.path(), &head, |_, _| panic!("missing script executed")),
            Outcome::Failed {
                ran: false,
                error: UNSAFE
            }
        );
        fs::create_dir(&hook).unwrap();
        assert_eq!(
            run_with(dir.path(), &head, |_, _| panic!("directory executed")),
            Outcome::Failed {
                ran: false,
                error: UNSAFE
            }
        );
    }
    fn shell(script: &str) -> Command {
        let mut command = Command::new(if cfg!(windows) {
            "powershell.exe"
        } else {
            "/bin/sh"
        });
        if cfg!(windows) {
            command.args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ]);
        } else {
            command.args(["-c", script]);
        }
        command
    }
    #[test]
    fn worktree_setup_runner_accepts_success_and_rejects_exit_output_and_timeout() {
        let deadline = Duration::from_secs(15);
        assert!(execute(shell("exit 0"), deadline, 1024));
        assert!(!execute(shell("exit 7"), deadline, 1024));
        let loud = if cfg!(windows) {
            "[Console]::Out.Write(('synthetic-secret' * 1000)); [Console]::Error.Write(('synthetic-secret' * 1000))"
        } else {
            "i=0; while [ $i -lt 2000 ]; do printf synthetic-secret; printf synthetic-secret >&2; i=$((i+1)); done"
        };
        assert!(!execute(shell(loud), deadline, 1024));
        let sleep = if cfg!(windows) {
            "Start-Sleep -Seconds 30"
        } else {
            "sleep 30"
        };
        let start = Instant::now();
        assert!(!execute(shell(sleep), Duration::from_millis(300), 1024));
        assert!(start.elapsed() < Duration::from_secs(10));
    }
    #[test]
    fn worktree_setup_timeout_kills_descendants_but_success_preserves_detached_helpers() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = Command::new("git");
        command.current_dir(dir.path()).args(["-c",
            "alias.setup-fixture=!f() { printf started > started; (sleep 2; printf escaped > escaped) & sleep 10; }; f",
            "setup-fixture"]);
        assert!(!execute(command, Duration::from_secs(1), 1024));
        assert!(dir.path().join("started").exists(), "fixture never ran");
        thread::sleep(Duration::from_millis(1500));
        assert!(
            !dir.path().join("escaped").exists(),
            "descendant survived timeout"
        );
        let mut command = Command::new("git");
        command.current_dir(dir.path()).args(["-c",
            "alias.setup-fixture=!f() { (sleep 1; printf survived > survived) </dev/null >/dev/null 2>&1 & }; f",
            "setup-fixture"]);
        assert!(execute(command, Duration::from_secs(5), 1024));
        let deadline = Instant::now() + Duration::from_secs(4);
        while !dir.path().join("survived").exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(dir.path().join("survived").exists());
    }
}
