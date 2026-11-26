use super::{Host, clean, transport_args, within};
use crate::agent_model_process::{self, Limits};
use std::time::Duration;

#[derive(Clone, Debug)]
pub(crate) struct Commit {
    pub id: String,
    pub author: String,
    pub time: i64,
    pub message: String,
    paths: Vec<String>,
}
impl Commit {
    pub fn details(&self, fr: bool) -> String {
        format!(
            "{} Â· {} Â· {}\n{}",
            self.author,
            crate::regional_date::long_local(self.time, fr).unwrap_or_default(),
            &self.id[..7],
            self.message
        )
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Page {
    pub commits: Vec<Commit>,
    pub limit: usize,
    pub more: bool,
    pub uncommitted: bool,
}

fn preamble(root: &str, path: &str) -> Result<String, String> {
    if !root.starts_with('/')
        || !clean(root)
        || !clean(path)
        || !within(root, path)
        || root == path
        || path
            .split('/')
            .skip(1)
            .any(|part| part.is_empty() || part == ".")
    {
        return Err("Invalid remote history target.".into());
    }
    let (dir, name) = path
        .rsplit_once('/')
        .ok_or("Invalid remote history path.")?;
    let dir = if dir.is_empty() { "/" } else { dir };
    // The SFTP open supplies a canonical path. Recheck the physical directory
    // and leaf in the shell session before asking Git about that literal file.
    let dir = crate::launch::posix_quote(dir);
    let name = crate::launch::posix_quote(name);
    Ok(format!(
        r#"dir={dir}
file={name}
CDPATH=
export GIT_TERMINAL_PROMPT=0 GIT_OPTIONAL_LOCKS=0 GIT_NO_LAZY_FETCH=1
cd -P "$dir" && [ "$(pwd -P)" = "$dir" ] && [ -f "$file" ] && [ ! -L "$file" ] || exit 2
"#,
    ))
}

fn script(root: &str, path: &str, limit: usize) -> Result<String, String> {
    if !(1..=500).contains(&limit) {
        return Err("Invalid remote history limit.".into());
    }
    let setup = preamble(root, path)?;
    Ok(format!(
        r#"{setup}if state=$(git -c core.fsmonitor=false --literal-pathspecs status --porcelain=v1 --untracked-files=all -- "$file") && [ -n "$state" ]; then
    echo dirty
else
    echo no-marker
fi
exec git --no-pager --literal-pathspecs log --follow --name-status --no-relative --no-color --no-notes --no-ext-diff --no-textconv --no-show-signature -z -n{} --format=%H%n%aN%n%at%n%B -- "$file"
"#,
        limit + 1
    ))
}

fn parse(output: &str, limit: usize) -> Result<Page, String> {
    let invalid = "Incomplete remote Git history.";
    let (state, output) = output.split_once('\n').ok_or(invalid)?;
    let uncommitted = match state {
        "dirty" => true,
        "no-marker" => false,
        _ => return Err(invalid.into()),
    };
    if !output.is_empty() && !output.ends_with('\0') {
        return Err(invalid.into());
    }
    let mut commits = Vec::new();
    let mut records = output.split_terminator('\0');
    while let Some(record) = records.next() {
        let mut fields = record.splitn(4, '\n');
        let id = fields.next().unwrap_or_default();
        if !matches!(id.len(), 40 | 64)
            || !id.bytes().all(|c| c.is_ascii_hexdigit())
            || commits.len() > limit
        {
            return Err(invalid.into());
        }
        commits.push(Commit {
            id: id.into(),
            author: fields.next().ok_or(invalid)?.into(),
            time: fields
                .next()
                .and_then(|v| v.parse::<i64>().ok())
                .and_then(|v| v.checked_mul(1000))
                .ok_or(invalid)?,
            message: fields.next().ok_or(invalid)?.trim().into(),
            paths: {
                let status = records.next().ok_or(invalid)?.trim_start_matches('\n');
                let count = if status.starts_with(['R', 'C']) { 2 } else { 1 };
                let mut paths = Vec::with_capacity(count);
                for _ in 0..count {
                    paths.push(records.next().ok_or(invalid)?.to_owned());
                }
                paths
            },
        });
    }
    let more = commits.len() > limit && limit < 500;
    commits.truncate(limit);
    Ok(Page {
        commits,
        uncommitted,
        more,
        limit,
    })
}

pub(crate) fn load(host: Host, root: &str, path: &str, limit: usize) -> Result<Page, String> {
    let input = script(root, path, limit)?;
    parse(&run(host, &input)?, limit)
}

fn diff_script(root: &str, path: &str, commit: Option<&Commit>) -> Result<String, String> {
    let mut input = preamble(root, path)?;
    if let Some(commit) = commit {
        if !matches!(commit.id.len(), 40 | 64)
            || !commit.id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("Invalid Git revision.".into());
        }
        input.push_str(
            "cd -P \"$(git rev-parse --show-toplevel)\" || exit 2\ntop=$(pwd -P) || exit 2\n",
        );
        let root = crate::launch::posix_quote(root.trim_end_matches('/'));
        input.push_str(&format!("root={root}\nset --\n"));
        for historical in &commit.paths {
            if !clean(historical) || historical.split('/').any(|p| p.is_empty() || p == ".") {
                return Err("Invalid Git history path.".into());
            }
            let name = crate::launch::posix_quote(historical);
            input.push_str(&format!("p={name}\ncase \"${{top%/}}/$p\" in \"$root\"/*) set -- \"$@\" \"$p\";; *) exit 2;; esac\n"));
        }
        input.push_str(&format!("exec git --no-pager --literal-pathspecs show --format= --no-show-signature --no-color --no-ext-diff --no-textconv --find-renames {} -- \"$@\"\n", commit.id));
    } else {
        input.push_str(r#"if git --literal-pathspecs ls-files --error-unmatch -- "$file" >/dev/null 2>&1; then
    exec git --no-pager --literal-pathspecs diff --no-ext-diff --no-textconv --no-color HEAD -- "$file"
fi
git --no-pager diff --no-index --no-ext-diff --no-textconv --no-color -- /dev/null "./$file"
rc=$?
[ "$rc" -eq 1 ] || exit "$rc"
"#);
    }
    Ok(input)
}

pub(crate) fn diff(
    host: Host,
    root: &str,
    path: &str,
    commit: Option<&Commit>,
) -> Result<Vec<String>, String> {
    let text = run(host, &diff_script(root, path, commit)?)?;
    Ok(text.lines().map(str::to_owned).collect())
}

fn run(host: Host, input: &str) -> Result<String, String> {
    let mut ssh = tokio::process::Command::new("ssh");
    ssh.args(transport_args(&host, false)?);
    let output = agent_model_process::run(
        ssh,
        Some(input),
        Limits {
            timeout: Duration::from_secs(25),
            output: 2 * 1024 * 1024,
        },
    )
    .map_err(|e| format!("Remote history request failed ({e:?})."))?;
    if output.code != Some(0) {
        return Err("Could not read Git history for this remote file.".into());
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, process::Command};

    #[test]
    fn remote_history_checks_scope_and_rejects_partial_replies() {
        for path in [
            "/other/file",
            "/repo/a/../b",
            "/repo//file",
            "/repo/./file",
            "/repo",
        ] {
            assert!(script("/repo", path, 50).is_err());
        }
        assert!(script("/repo", "/repo/test", 501).is_err());
        assert!(parse("truncated", 1).is_err());
        assert!(parse("dirty\nbad\nname\n0\nmessage\0", 1).is_err());
        let args = transport_args(
            &Host {
                alias: "fixture".into(),
                ..Host::default()
            },
            false,
        )
        .unwrap();
        assert!(args.iter().any(|p| p == "StrictHostKeyChecking=yes"));
        assert_eq!(&args[args.len() - 3..], ["--", "fixture", "sh -s"]);
    }

    #[test]
    fn remote_history_script_pages_real_git_across_a_literal_rename() {
        let dir = tempfile::tempdir().unwrap();
        let sh = if cfg!(windows) {
            let paths = Command::new("where.exe").arg("git.exe").output().unwrap();
            String::from_utf8_lossy(&paths.stdout)
                .lines()
                .filter_map(|line| {
                    Some(
                        PathBuf::from(line.trim())
                            .parent()?
                            .parent()?
                            .join("bin/sh.exe"),
                    )
                })
                .find(|p| p.is_file())
                .expect("Git for Windows shell")
        } else {
            PathBuf::from("sh")
        };
        let root = Command::new(&sh)
            .current_dir(dir.path())
            .args(["-c", "pwd -P"])
            .output()
            .unwrap();
        let root = String::from_utf8(root.stdout)
            .unwrap()
            .trim_end()
            .to_owned();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .current_dir(dir.path())
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-q"]);
        std::fs::write(dir.path().join("old"), "one").unwrap();
        git(&["add", "old"]);
        git(&["commit", "-qm", "original"]);
        let file = "new's [file] $test";
        git(&["mv", "old", file]);
        git(&["commit", "-qm", "renamed"]);
        let run = |file: &str, limit| {
            let script = script(&root, &format!("{root}/{file}"), limit).unwrap();
            let output = Command::new(&sh)
                .current_dir(dir.path())
                .args(["-c", &script])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            parse(&String::from_utf8(output.stdout).unwrap(), limit).unwrap()
        };
        let patch = |file: &str, commit: Option<&Commit>| {
            let input = diff_script(&root, &format!("{root}/{file}"), commit).unwrap();
            let output = Command::new(&sh)
                .current_dir(dir.path())
                .args(["-c", &input])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap()
        };
        let page = run(file, 1);
        assert!(page.more);
        assert_eq!(page.commits[0].message, "renamed");
        let all = run(file, 50);
        assert!(patch(file, Some(&all.commits[1])).contains("+one"));
        assert!(patch(file, Some(&all.commits[0])).contains("rename from old"));
        assert!(!all.uncommitted);
        assert!(!all.more);
        assert_eq!(all.commits.len(), 2);
        assert_eq!(all.commits[1].message, "original");
        std::fs::write(dir.path().join("other"), "untracked").unwrap();
        assert!(!run(file, 50).uncommitted);
        assert!(patch("other", None).contains("+untracked"));
        let other = run("other", 50);
        assert!(other.uncommitted && other.commits.is_empty());
        std::fs::write(dir.path().join(file), "local change").unwrap();
        assert!(run(file, 50).uncommitted);
        assert!(patch(file, None).contains("+local change"));
        git(&["add", "--", file]);
        assert!(run(file, 50).uncommitted);
        assert!(patch(file, None).contains("+local change"));
        git(&["commit", "-qm", "working"]);
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        git(&["mv", file, "nested/current"]);
        git(&["add", "--", "other"]);
        git(&["commit", "-qm", "move into project"]);
        let moved = run("nested/current", 50);
        let first = &moved.commits[0];
        let diff = patch("nested/current", Some(first));
        assert!(diff.contains("rename to nested/current"));
        assert!(!diff.contains("+untracked"));
        let nested = format!("{root}/nested");
        let confined = diff_script(&nested, &format!("{nested}/current"), Some(first)).unwrap();
        let denied = Command::new(&sh).args(["-c", &confined]).output().unwrap();
        assert!(!denied.status.success());
        assert!(denied.stdout.is_empty());
    }
}
