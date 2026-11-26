use super::{Host, clean, transport_args, within};
use crate::agent_model_process::{self, Limits};
use std::time::Duration;

#[derive(Clone, Debug)]
pub(crate) struct Commit {
    pub id: String,
    pub author: String,
    pub time: i64,
    pub message: String,
}
impl Commit {
    pub fn details(&self, fr: bool) -> String {
        format!(
            "{} · {} · {}\n{}",
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
}

fn script(root: &str, path: &str, limit: usize) -> Result<String, String> {
    if !root.starts_with('/')
        || !clean(root)
        || !clean(path)
        || !within(root, path)
        || root == path
        || path
            .split('/')
            .skip(1)
            .any(|part| part.is_empty() || part == ".")
        || !(1..=500).contains(&limit)
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
        "dir={dir}\nfile={name}\nCDPATH=\nexport GIT_TERMINAL_PROMPT=0 GIT_OPTIONAL_LOCKS=0 GIT_NO_LAZY_FETCH=1\ncd -P \"$dir\" && [ \"$(pwd -P)\" = \"$dir\" ] && [ -f \"$file\" ] && [ ! -L \"$file\" ] || exit 2\nexec git --no-pager --literal-pathspecs log --follow --no-patch --no-color --no-notes --no-ext-diff --no-textconv --no-show-signature -z -n{} --format=%H%n%aN%n%at%n%B -- \"$file\"\n",
        limit + 1
    ))
}

fn parse(output: &str, limit: usize) -> Result<Page, String> {
    let invalid = "Incomplete remote Git history.";
    if !output.is_empty() && !output.ends_with('\0') {
        return Err(invalid.into());
    }
    let mut commits = Vec::new();
    for record in output.split_terminator('\0') {
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
        });
    }
    let more = commits.len() > limit && limit < 500;
    commits.truncate(limit);
    Ok(Page {
        commits,
        more,
        limit,
    })
}

pub(crate) fn load(host: Host, root: &str, path: &str, limit: usize) -> Result<Page, String> {
    let input = script(root, path, limit)?;
    let mut ssh = tokio::process::Command::new("ssh");
    ssh.args(transport_args(&host, false)?);
    let output = agent_model_process::run(
        ssh,
        Some(&input),
        Limits {
            timeout: Duration::from_secs(25),
            output: 2 * 1024 * 1024,
        },
    )
    .map_err(|e| format!("Remote history request failed ({e:?})."))?;
    if output.code != Some(0) {
        return Err("Could not read Git history for this remote file.".into());
    }
    parse(&output.stdout, limit)
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
        assert!(parse("bad\nname\n0\nmessage\0", 1).is_err());
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
        let run = |limit| {
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
        let page = run(1);
        assert!(page.more);
        assert_eq!(page.commits[0].message, "renamed");
        let all = run(50);
        assert!(!all.more);
        assert_eq!(all.commits.len(), 2);
        assert_eq!(all.commits[1].message, "original");
    }
}
