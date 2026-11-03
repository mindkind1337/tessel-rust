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

/// Stage exactly one project-relative file, including an already tracked deletion.
/// Literal pathspecs prevent names such as `*.txt` from selecting other files.
pub fn stage_file(root: &Path, relative: &str) -> Result<String> {
    let relative =
        project_relative_path("", relative).ok_or("Choose a file inside this project.")?;
    let canonical_root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut candidate = canonical_root.clone();
    for component in Path::new(relative).components() {
        candidate.push(component);
        match std::fs::symlink_metadata(&candidate) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err("Staging through symbolic links is not supported.".into());
                }
                let resolved = candidate.canonicalize().map_err(|e| e.to_string())?;
                if !resolved.starts_with(&canonical_root) {
                    return Err("Path is outside this project.".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    if root.join(relative).is_dir() {
        return Err("Choose one file, not a directory.".into());
    }
    run(root, &["--literal-pathspecs", "add", "--", relative], None)
}

pub struct ScopedStatus {
    pub project_prefix: String,
    pub output: Capture,
}

/// Git porcelain paths are repository-relative even from a nested project.
/// Keep that prefix explicit; the consumer must map paths before opening files.
pub fn scoped_status(root: &Path) -> Result<ScopedStatus> {
    let prefix = run_bytes(root, &["rev-parse", "--show-prefix"], None)?;
    if prefix.truncated {
        return Err("Git project prefix exceeded the output limit.".into());
    }
    let project_prefix =
        String::from_utf8(prefix.bytes).map_err(|_| "Git project prefix is not valid UTF-8.")?;
    let project_prefix = project_prefix
        .strip_suffix('\n')
        .ok_or("Git project prefix is incomplete.")?
        .to_string();
    let output = run_bytes(
        root,
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=all",
            "--",
            ".",
        ],
        None,
    )?;
    Ok(ScopedStatus {
        project_prefix,
        output,
    })
}

/// Preserve literal names, rejecting traversal and paths outside the project.
pub fn project_relative_path<'a>(prefix: &str, repository_path: &'a str) -> Option<&'a str> {
    let relative = repository_path.strip_prefix(prefix)?;
    if relative.is_empty() || relative.as_bytes().contains(&0) {
        return None;
    }
    #[cfg(windows)]
    if relative.contains(['\\', ':']) {
        // Porcelain uses forward slashes on Windows. Reject alternate separators
        // and NTFS stream syntax rather than reinterpret a repository path.
        return None;
    }
    if !Path::new(relative)
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_)))
    {
        return None;
    }
    // Path::components normalizes internal dots; reject them explicitly too.
    if relative
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    Some(relative)
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

    #[test]
    fn nested_project_status_excludes_siblings_and_maps_literal_names() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"], None).unwrap();
        let nested = dir.path().join("inside");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("é space.txt"), "inside").unwrap();
        std::fs::write(dir.path().join("outside.txt"), "outside").unwrap();
        let status = scoped_status(&nested).unwrap();
        assert_eq!(status.project_prefix, "inside/");
        assert!(!status.output.truncated);
        assert!(
            status
                .output
                .bytes
                .split(|b| *b == 0)
                .any(|r| r == "? inside/é space.txt".as_bytes())
        );
        assert!(
            !status
                .output
                .bytes
                .windows(b"outside.txt".len())
                .any(|r| r == b"outside.txt")
        );
        assert_eq!(
            project_relative_path(&status.project_prefix, "inside/é space.txt"),
            Some("é space.txt")
        );
        for invalid in [
            "outside.txt",
            "inside-other/a",
            "inside/../outside.txt",
            "inside/./a",
            "inside//a",
            "inside/",
        ] {
            assert!(project_relative_path(&status.project_prefix, invalid).is_none());
        }
        assert!(project_relative_path("", "/absolute").is_none());
        assert_eq!(
            project_relative_path("", "..notes.txt"),
            Some("..notes.txt")
        );
        assert_eq!(project_relative_path("", "...config"), Some("...config"));
        #[cfg(windows)]
        for invalid in [
            "C:/outside.txt",
            "../outside",
            "..\\outside",
            "\\\\server\\share\\file",
            "file.txt:stream",
            "folder\\.\\file.txt",
        ] {
            assert!(project_relative_path("", invalid).is_none());
        }
        assert_eq!(scoped_status(dir.path()).unwrap().project_prefix, "");
    }
    #[test]
    fn stage_file_rejects_escape_and_preserves_literal_names_and_deletions() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"], None).unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(dir.path().join("outside.txt"), "outside").unwrap();
        // Reproduce the former UI behavior in this disposable repository only.
        run(
            &root,
            &["--literal-pathspecs", "add", "--", "../outside.txt"],
            None,
        )
        .unwrap();
        assert!(
            run(dir.path(), &["diff", "--cached", "--name-only"], None)
                .unwrap()
                .contains("outside.txt")
        );
        run(dir.path(), &["rm", "--cached", "--", "outside.txt"], None).unwrap();
        for path in [
            "../outside.txt",
            ".",
            "",
            "/outside.txt",
            "sub/../outside.txt",
        ] {
            assert!(stage_file(&root, path).is_err(), "{path}");
        }
        assert!(stage_file(&root, dir.path().join("outside.txt").to_str().unwrap()).is_err());
        for name in ["..notes.txt", "\u{e9} space.txt", "[file].txt", "f.txt"] {
            std::fs::write(root.join(name), name).unwrap();
        }
        std::fs::create_dir(root.join("folder")).unwrap();
        assert!(stage_file(&root, "folder").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path(), root.join("link")).unwrap();
            assert!(stage_file(&root, "link/outside.txt").is_err());
            std::fs::remove_file(root.join("link")).unwrap();
        }
        stage_file(&root, "[file].txt").unwrap();
        let staged = run(dir.path(), &["diff", "--cached", "--name-only"], None).unwrap();
        assert_eq!(staged.trim(), "project/[file].txt");
        stage_file(&root, "..notes.txt").unwrap();
        stage_file(&root, "\u{e9} space.txt").unwrap();
        run(
            dir.path(),
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
            None,
        )
        .unwrap();
        std::fs::remove_file(root.join("..notes.txt")).unwrap();
        stage_file(&root, "..notes.txt").unwrap();
        let staged = run(dir.path(), &["diff", "--cached", "--name-status"], None).unwrap();
        assert_eq!(staged.trim(), "D\tproject/..notes.txt");
        assert!(!staged.contains("outside.txt"));
    }
}
