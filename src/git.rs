//! Bounded, on-demand Git operations. No automatic recursive status polling.
use crate::workspace::Result;
use std::{collections::HashMap, path::Path, time::Duration};
#[cfg(test)]
use std::{thread, time::Instant};

pub fn run(root: &Path, args: &[&str], input: Option<&[u8]>) -> Result<String> {
    run_bytes(root, args, input).map(|output| output.text())
}

/// Stage exactly one project-relative file, including an already tracked deletion.
/// Literal pathspecs prevent names such as `*.txt` from selecting other files.
#[cfg(test)]
pub fn stage_file(root: &Path, relative: &str) -> Result<String> {
    stage_files(root, &[relative.to_owned()])
}

pub fn stage_files(root: &Path, paths: &[String]) -> Result<String> {
    let input = selected_path_input(root, paths)?;
    run(
        root,
        &[
            "--literal-pathspecs",
            "add",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ],
        Some(&input),
    )?;
    Ok(format!("Staged {} selected file(s).", paths.len()))
}

pub fn unstage_files(root: &Path, paths: &[String]) -> Result<String> {
    let input = selected_path_input(root, paths)?;
    // Mixed reset changes only the index, including an unborn branch on supported Git.
    run(
        root,
        &[
            "--literal-pathspecs",
            "reset",
            "-q",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ],
        Some(&input),
    )?;
    Ok(format!("Unstaged {} selected file(s).", paths.len()))
}

pub fn commit_staged(root: &Path, message: &str) -> Result<String> {
    if message.len() > OUTPUT_LIMIT {
        return Err("Commit message exceeds the size limit.".into());
    }
    let cleaned: String = message
        .chars()
        .filter(|c| *c != '\r' && *c != '\0')
        .collect();
    let message = cleaned.trim();
    if message.is_empty() {
        return Err("Enter a commit message.".into());
    }
    let out = run_bytes_with_timeout(
        root,
        &["commit", "-F", "-"],
        Some(message.as_bytes()),
        Duration::from_secs(180),
    )?
    .text();
    let trimmed = out.trim();
    if trimmed.is_empty() {
        Ok("Committed staged changes.".into())
    } else {
        Ok(trimmed.to_owned())
    }
}

pub fn discard_worktree(
    root: &Path,
    tracked_paths: &[String],
    untracked_paths: &[String],
) -> Result<String> {
    discard_with_recycler(
        root,
        tracked_paths,
        untracked_paths,
        crate::file_actions::move_to_trash,
    )
}

fn discard_with_recycler(
    root: &Path,
    tracked_paths: &[String],
    untracked_paths: &[String],
    mut recycle: impl FnMut(&Path, &Path) -> Result<std::path::PathBuf>,
) -> Result<String> {
    let total = tracked_paths.len() + untracked_paths.len();
    if total == 0 || total > 5000 {
        return Err("Select between one and 5,000 files.".into());
    }
    for path in tracked_paths.iter().chain(untracked_paths.iter()) {
        validate_stage_file(root, path)?;
    }
    if !tracked_paths.is_empty() {
        let input = selected_path_input(root, tracked_paths)?;
        run(
            root,
            &[
                "--literal-pathspecs",
                "checkout",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
                "--",
            ],
            Some(&input),
        )?;
    }
    for (recycled, rel) in untracked_paths.iter().enumerate() {
        let target = root.join(rel);
        recycle(root, &target).map_err(|_| format!("Recycling failed or was cancelled; no permanent deletion was attempted. {recycled} of {} untracked files were recycled; {} tracked files were already restored.", untracked_paths.len(), tracked_paths.len()))?;
    }
    Ok(format!("Discarded {total} file(s)."))
}

pub fn remote_op(
    root: &Path,
    op: crate::scm_changes_panel::ScmRemoteOp,
    branch: Option<&str>,
) -> Result<String> {
    use crate::scm_changes_panel::ScmRemoteOp;
    match op {
        ScmRemoteOp::Fetch => {
            let out = run(root, &["fetch", "--prune"], None)?;
            Ok(if out.trim().is_empty() {
                "Fetched remote updates.".into()
            } else {
                out.trim().to_owned()
            })
        }
        ScmRemoteOp::Pull | ScmRemoteOp::FastForward => {
            let out = run(root, &["pull", "--ff-only"], None)?;
            Ok(if out.trim().is_empty() {
                "Pulled fast-forward updates.".into()
            } else {
                out.trim().to_owned()
            })
        }
        ScmRemoteOp::Push => {
            let out = run(root, &["push"], None)?;
            Ok(if out.trim().is_empty() {
                "Pushed commits to upstream.".into()
            } else {
                out.trim().to_owned()
            })
        }
        ScmRemoteOp::Sync => {
            let pulled = run(root, &["pull", "--ff-only"], None)?;
            let pushed = run(root, &["push"], None)?;
            let combined = format!("{}\n{}", pulled.trim(), pushed.trim());
            let trimmed = combined.trim();
            Ok(if trimmed.is_empty() {
                "Synchronized branch with upstream.".into()
            } else {
                trimmed.to_owned()
            })
        }
        ScmRemoteOp::Publish => {
            let branch = branch
                .map(str::trim)
                .filter(|b| {
                    !b.is_empty()
                        && !b.starts_with('-')
                        && !b.chars().any(|c| c.is_control() || c.is_whitespace())
                })
                .ok_or("Cannot publish a detached or invalid branch name.")?;
            let out = run(root, &["push", "-u", "origin", branch], None)?;
            Ok(if out.trim().is_empty() {
                format!("Published branch {branch} to origin.")
            } else {
                out.trim().to_owned()
            })
        }
    }
}

pub fn file_diff(
    root: &Path,
    relative: &str,
    old_relative: Option<&str>,
    staged: bool,
    untracked: bool,
) -> Result<String> {
    validate_stage_file(root, relative)?;
    if let Some(old) = old_relative {
        validate_stage_file(root, old)?;
    }
    if untracked {
        let canonical_root = root.canonicalize().map_err(|e| e.to_string())?;
        let path = crate::files::confined(&canonical_root, &root.join(relative))?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        const DIFF_PREVIEW_MAX: usize = 512 * 1024;
        if bytes.iter().take(8192).any(|&b| b == 0) {
            return Ok(format!(
                "diff --git a/{relative} b/{relative}\nnew file\nBinary files /dev/null and b/{relative} differ\n"
            ));
        }
        let truncated = bytes.len() > DIFF_PREVIEW_MAX;
        let text = String::from_utf8_lossy(&bytes[..bytes.len().min(DIFF_PREVIEW_MAX)]);
        let lines: Vec<&str> = text.lines().collect();
        let mut out = format!(
            "diff --git a/{relative} b/{relative}\nnew file mode 100644\n--- /dev/null\n+++ b/{relative}\n@@ -0,0 +1,{} @@\n",
            lines.len()
        );
        for line in lines {
            out.push('+');
            out.push_str(line);
            out.push('\n');
        }
        if truncated {
            out.push_str("\n[Untracked diff preview truncated at 512 KiB.]\n");
        }
        return Ok(out);
    }
    let mut args = vec!["--literal-pathspecs", "diff"];
    if staged {
        args.push("--cached");
    }
    args.extend(["--no-ext-diff", "--no-textconv", "--"]);
    if let Some(old) = old_relative {
        args.push(old);
    }
    args.push(relative);
    run(root, &args, None)
}

pub type NumstatMap = HashMap<String, (Option<u32>, Option<u32>)>;

pub fn parse_numstat_z(bytes: &[u8], project_prefix: &str) -> NumstatMap {
    let mut map = NumstatMap::new();
    let mut records = bytes.split(|&b| b == 0);
    while let Some(rec) = records.next() {
        if rec.is_empty() {
            continue;
        }
        let Ok(rec_str) = std::str::from_utf8(rec) else {
            continue;
        };
        let mut parts = rec_str.splitn(3, '\t');
        let Some(added_raw) = parts.next() else {
            continue;
        };
        let Some(removed_raw) = parts.next() else {
            continue;
        };
        let Some(path_part) = parts.next() else {
            continue;
        };
        let added = if added_raw == "-" {
            None
        } else {
            added_raw.parse::<u32>().ok()
        };
        let removed = if removed_raw == "-" {
            None
        } else {
            removed_raw.parse::<u32>().ok()
        };
        let repo_path = if path_part.is_empty() {
            // Rename/copy in `-z` mode: `<added>\t<removed>\t\0<old_path>\0<new_path>\0`
            let _old = records.next();
            let Some(new_bytes) = records.next() else {
                break;
            };
            let Ok(new_str) = std::str::from_utf8(new_bytes) else {
                continue;
            };
            new_str
        } else {
            path_part
        };
        if let Some(rel) = project_relative_path(project_prefix, repo_path) {
            map.insert(rel.to_owned(), (added, removed));
        }
    }
    map
}

pub fn scoped_numstat(root: &Path, project_prefix: &str, cached: bool) -> Result<NumstatMap> {
    let args: &[&str] = if cached {
        &[
            "diff",
            "--cached",
            "--numstat",
            "-z",
            "--no-ext-diff",
            "--no-textconv",
            "--",
            ".",
        ]
    } else {
        &[
            "diff",
            "--numstat",
            "-z",
            "--no-ext-diff",
            "--no-textconv",
            "--",
            ".",
        ]
    };
    let capture = run_bytes(root, args, None)?;
    if capture.truncated {
        return Ok(NumstatMap::new());
    }
    Ok(parse_numstat_z(&capture.bytes, project_prefix))
}

fn selected_path_input(root: &Path, paths: &[String]) -> Result<Vec<u8>> {
    if paths.is_empty() || paths.len() > 5000 {
        return Err("Select between one and 5,000 files.".into());
    }
    let mut input = Vec::new();
    for path in paths {
        validate_stage_file(root, path)?;
        if input.len() + path.len() + 1 > OUTPUT_LIMIT {
            return Err("Selected Git paths exceed the input limit.".into());
        }
        input.extend_from_slice(path.as_bytes());
        input.push(0);
    }
    Ok(input)
}

fn validate_stage_file(root: &Path, relative: &str) -> Result<()> {
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
    Ok(())
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

#[path = "git/process.rs"]
mod process;
#[cfg(test)]
use process::drain;
pub use process::{
    Capture, GuestRepository, run_bytes, run_guest_bytes, run_guest_check_ignore,
    run_guest_repository,
};
use process::{OUTPUT_LIMIT, run_bytes_with_timeout};

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    #[test]
    fn normal_git_exit_keeps_intentionally_detached_helpers() {
        let dir = tempfile::tempdir().unwrap();
        run_bytes_with_timeout(dir.path(),
            &["-c", "alias.cleanup-test=!f() { (sleep 1; printf survived > hook-survived) </dev/null >/dev/null 2>&1 & }; f", "cleanup-test"],
            None, Duration::from_secs(5)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while !dir.path().join("hook-survived").exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        assert!(dir.path().join("hook-survived").exists());
    }

    #[test]
    fn successful_git_with_inherited_pipe_keeps_status_and_marks_partial_output() {
        let dir = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let output = run_bytes_with_timeout(
            dir.path(),
            &[
                "-c",
                "alias.cleanup-test=!f() { printf original; (sleep 10; printf late) & }; f",
                "cleanup-test",
            ],
            None,
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(6));
        assert_eq!(output.bytes, b"original");
        assert!(output.truncated);
        // Let the finite fixture finish and release its inherited pipes.
        thread::sleep(Duration::from_secs(10));
    }

    #[test]
    fn timed_out_git_kills_inherited_hook_processes() {
        let dir = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let result = run_bytes_with_timeout(
            dir.path(),
            &[
                "-c",
                "alias.cleanup-test=!f() { printf started > hook-started; (sleep 2; printf escaped > hook-escaped) & sleep 10; }; f",
                "cleanup-test",
            ],
            None,
            Duration::from_secs(1),
        );
        assert!(result.err().unwrap().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(4));
        assert!(
            dir.path().join("hook-started").exists(),
            "fixture never ran"
        );
        thread::sleep(Duration::from_millis(1500));
        assert!(
            !dir.path().join("hook-escaped").exists(),
            "descendant survived cleanup"
        );
    }

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
                .ends_with("[Git output incomplete or truncated at 2 MiB.]\n")
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
    #[test]
    fn multi_file_stage_validates_every_path_before_mutating_the_index() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"], None).unwrap();
        std::fs::write(dir.path().join("[one].txt"), "one").unwrap();
        std::fs::write(dir.path().join("two space.txt"), "two").unwrap();
        std::fs::write(dir.path().join("o.txt"), "other").unwrap();
        assert!(stage_files(dir.path(), &["[one].txt".into(), "../outside".into()]).is_err());
        assert!(
            run(dir.path(), &["diff", "--cached", "--name-only"], None)
                .unwrap()
                .is_empty()
        );
        stage_files(dir.path(), &["[one].txt".into(), "two space.txt".into()]).unwrap();
        let staged = run(dir.path(), &["diff", "--cached", "--name-only"], None).unwrap();
        assert_eq!(
            staged.lines().collect::<Vec<_>>(),
            vec!["[one].txt", "two space.txt"]
        );
    }
    #[test]
    fn unstage_preserves_working_bytes_before_and_after_the_first_commit() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"], None).unwrap();
        std::fs::write(dir.path().join("[one].txt"), "one").unwrap();
        std::fs::write(dir.path().join("two.txt"), "base").unwrap();
        stage_files(dir.path(), &["[one].txt".into(), "two.txt".into()]).unwrap();
        unstage_files(dir.path(), &["[one].txt".into()]).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("[one].txt")).unwrap(),
            "one"
        );
        assert_eq!(
            run(dir.path(), &["diff", "--cached", "--name-only"], None)
                .unwrap()
                .trim(),
            "two.txt"
        );
        run(
            dir.path(),
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "base",
            ],
            None,
        )
        .unwrap();
        std::fs::write(dir.path().join("two.txt"), "edited").unwrap();
        stage_files(dir.path(), &["[one].txt".into(), "two.txt".into()]).unwrap();
        assert!(unstage_files(dir.path(), &["two.txt".into(), "../outside.txt".into()]).is_err());
        assert!(
            !run(dir.path(), &["diff", "--cached", "--name-only"], None)
                .unwrap()
                .is_empty()
        );
        unstage_files(dir.path(), &["[one].txt".into(), "two.txt".into()]).unwrap();
        assert!(
            run(dir.path(), &["diff", "--cached", "--name-only"], None)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("two.txt")).unwrap(),
            "edited"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("[one].txt")).unwrap(),
            "one"
        );
    }

    #[test]
    fn cancelled_recycling_preserves_untracked_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("keep.txt");
        std::fs::write(&path, "keep synthetic content").unwrap();
        let result = discard_with_recycler(temp.path(), &[], &["keep.txt".into()], |_, _| {
            Err("cancelled".into())
        });
        assert!(result.unwrap_err().contains("no permanent deletion"));
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "keep synthetic content"
        );
    }

    #[test]
    fn numstat_discard_commit_and_file_diff_round_trip() {
        let raw =
            b"12\t3\tsrc/app.rs\x00-\t-\tassets/logo.png\x004\t1\t\x00src/old.rs\x00src/new.rs\x00";
        let parsed = parse_numstat_z(raw, "src/");
        assert_eq!(parsed.get("app.rs"), Some(&(Some(12), Some(3))));
        assert_eq!(parsed.get("new.rs"), Some(&(Some(4), Some(1))));
        assert!(!parsed.contains_key("assets/logo.png"));

        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"], None).unwrap();
        run(dir.path(), &["config", "core.autocrlf", "false"], None).unwrap();
        run(dir.path(), &["config", "user.name", "Fixture"], None).unwrap();
        run(
            dir.path(),
            &["config", "user.email", "fixture@example.invalid"],
            None,
        )
        .unwrap();
        std::fs::write(dir.path().join("tracked.txt"), "line1\nline2\n").unwrap();
        stage_file(dir.path(), "tracked.txt").unwrap();
        assert!(commit_staged(dir.path(), "   ").is_err());
        commit_staged(dir.path(), "initial commit").unwrap();

        std::fs::write(
            dir.path().join("tracked.txt"),
            "line1\nchanged\nextra1\nextra2\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("scratch.txt"), "temp untracked\n").unwrap();
        let stats = scoped_numstat(dir.path(), "", false).unwrap();
        assert_eq!(stats.get("tracked.txt"), Some(&(Some(3), Some(1))));
        let wt_diff = file_diff(dir.path(), "tracked.txt", None, false, false).unwrap();
        assert!(wt_diff.contains("+changed"));
        let untracked_diff = file_diff(dir.path(), "scratch.txt", None, false, true).unwrap();
        assert!(untracked_diff.contains("+temp untracked"));

        discard_worktree(dir.path(), &["tracked.txt".into()], &["scratch.txt".into()]).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("tracked.txt")).unwrap(),
            "line1\nline2\n"
        );
        assert!(!dir.path().join("scratch.txt").exists());
    }
}
