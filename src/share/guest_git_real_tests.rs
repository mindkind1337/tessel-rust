//! Real Git boundaries with an empty synthetic home/config and disposable repositories.
use super::*;
use std::{path::PathBuf, process::Command};
struct RealGit {
    root: PathBuf,
    home: PathBuf,
}
impl GitRun for RealGit {
    fn run(&mut self, args: &[&str], _input: Option<&[u8]>) -> Result<Capture> {
        let mut command = Command::new("git");
        command.env_clear();
        for key in ["PATH", "SystemRoot", "WINDIR", "COMSPEC", "PATHEXT"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("APPDATA", &self.home)
            .env("LOCALAPPDATA", &self.home)
            .env("TEMP", &self.home)
            .env("TMP", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join("empty-config"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let output = command
            .args(args)
            .current_dir(&self.root)
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        Ok(Capture {
            bytes: output.stdout,
            truncated: false,
        })
    }
}
struct Repo {
    _dir: tempfile::TempDir,
    git: RealGit,
    hooks: PathBuf,
}
impl Repo {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("guest-git-real-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap();
        let root = dir.path().join("repo");
        let home = dir.path().join("home");
        let hooks = dir.path().join("empty-hooks");
        for path in [&root, &home, &hooks] {
            std::fs::create_dir_all(path).unwrap();
        }
        std::fs::write(home.join("empty-config"), b"").unwrap();
        let mut git = RealGit { root, home };
        git.run(&["init", "-q", "--template", hooks.to_str().unwrap()], None)
            .unwrap();
        git.run(&["config", "user.name", "Synthetic Guest Test"], None)
            .unwrap();
        git.run(&["config", "user.email", "guest@example.invalid"], None)
            .unwrap();
        Self {
            _dir: dir,
            git,
            hooks,
        }
    }
    fn command(&mut self, args: &[&str]) -> Vec<u8> {
        self.git.run(args, None).unwrap().bytes
    }
    fn write(&self, path: &str, content: &str) {
        let path = self.git.root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    fn request(&mut self, op: &str, req: Value) -> Result<Value> {
        let root = self.git.root.clone();
        dispatch_with(
            &mut self.git,
            &root,
            true,
            true,
            op,
            &req,
            &self.hooks,
            &mut || Ok(()),
        )
    }
    fn baseline(&mut self) {
        self.command(&["add", "--all"]);
        self.command(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--no-gpg-sign",
            "-m",
            "baseline",
        ]);
    }
}
#[test]
fn real_guest_discard_never_recurses_into_host_submodules() {
    let mut source = Repo::new();
    source.write("note.txt", "committed");
    source.baseline();
    let mut parent = Repo::new();
    let source_path = source.git.root.to_string_lossy().into_owned();
    parent.command(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        "--",
        &source_path,
        "sub",
    ]);
    parent.baseline();
    parent.command(&["config", "submodule.recurse", "true"]);
    parent.write("sub/note.txt", "local edits");
    // Positive control: host configuration genuinely expands ordinary checkout.
    parent.command(&["checkout", "--", "sub"]);
    assert_eq!(
        std::fs::read_to_string(parent.git.root.join("sub/note.txt")).unwrap(),
        "committed"
    );
    parent.write("sub/note.txt", "local edits");
    let head_before = parent.command(&["-C", "sub", "rev-parse", "HEAD"]);
    parent
        .request("scm.discard", json!({"path":"sub"}))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(parent.git.root.join("sub/note.txt")).unwrap(),
        "local edits"
    );
    assert_eq!(
        parent.command(&["-C", "sub", "rev-parse", "HEAD"]),
        head_before
    );
}

#[test]
fn real_guest_git_never_runs_hooks_fsmonitor_or_signing_program() {
    let mut repo = Repo::new();
    repo.write("visible.txt", "before");
    repo.baseline();
    for hook in [
        "pre-commit",
        "prepare-commit-msg",
        "commit-msg",
        "post-commit",
        "post-index-change",
        "reference-transaction",
    ] {
        repo.write(
            &format!(".git/hooks/{hook}"),
            "#!/bin/sh\nprintf called > marker-hook\nexit 0\n",
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                repo.git.root.join(format!(".git/hooks/{hook}")),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
    }
    repo.write(
        ".git/helper",
        "#!/bin/sh\nprintf called > marker-helper\nexit 1\n",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            repo.git.root.join(".git/helper"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    repo.command(&["config", "core.fsmonitor", ".git/helper"]);
    repo.command(&["config", "gpg.program", ".git/helper"]);
    repo.command(&["config", "commit.gpgsign", "true"]);
    repo.write("visible.txt", "after");
    repo.request("scm.status", json!({})).unwrap();
    repo.request("scm.stage", json!({"path":"visible.txt"}))
        .unwrap();
    repo.request("scm.commit", json!({"message":"guest change"}))
        .unwrap();
    assert!(!repo.git.root.join("marker-hook").exists());
    assert!(!repo.git.root.join("marker-helper").exists());
    let commit = repo.command(&["cat-file", "commit", "HEAD"]);
    assert!(!String::from_utf8_lossy(&commit).contains("gpgsig"));
}
#[test]
fn real_guest_git_hides_private_diff_and_refuses_filtered_stage() {
    let mut repo = Repo::new();
    repo.write("visible.txt", "before");
    repo.write(".env", "PRIVATE_BEFORE");
    repo.baseline();
    repo.write("visible.txt", "after");
    repo.write(".env", "PRIVATE_AFTER");
    let status = repo.request("scm.status", json!({})).unwrap();
    assert!(status["status"].as_str().unwrap().contains("visible.txt"));
    assert!(!status.to_string().contains(".env"));
    let diff = repo.request("scm.diff", json!({})).unwrap();
    assert!(diff["diff"].as_str().unwrap().contains("after"));
    assert!(!diff.to_string().contains("PRIVATE_"));
    assert!(repo.request("scm.diff", json!({"path":".env"})).is_err());
    repo.write(".gitattributes", "*.dat filter=Crypt\n");
    repo.write("payload.dat", "plaintext");
    repo.command(&[
        "config",
        "filter.Crypt.clean",
        "sh -c 'printf called > marker-filter; cat'",
    ]);
    assert!(
        repo.request("scm.stage", json!({"path":"payload.dat"}))
            .is_err()
    );
    assert!(!repo.git.root.join("marker-filter").exists());
    assert!(
        repo.command(&["diff", "--cached", "--name-only"])
            .is_empty()
    );
}
#[test]
fn real_guest_git_subproject_status_diff_and_commit_stay_in_scope() {
    let mut repo = Repo::new();
    repo.write("project/visible.txt", "before");
    repo.write("outside.txt", "outside before");
    repo.baseline();
    repo.write("project/visible.txt", "after");
    repo.write("outside.txt", "outside after");
    repo.command(&["add", "outside.txt"]);
    let top = repo.git.root.clone();
    repo.git.root = top.join("project");
    let status = repo.request("scm.status", json!({})).unwrap();
    assert!(status["status"].as_str().unwrap().contains("visible.txt"));
    assert!(!status.to_string().contains("outside"));
    let diff = repo
        .request("scm.diff", json!({"path":"visible.txt"}))
        .unwrap();
    assert!(diff["diff"].as_str().unwrap().contains("after"));
    assert!(!diff.to_string().contains("outside"));
    repo.request("scm.stage", json!({"path":"visible.txt"}))
        .unwrap();
    assert!(
        repo.request(
            "scm.commit",
            json!({"message":"must refuse outside staged work"})
        )
        .is_err()
    );
    for magic in [":/", "*", "../outside.txt"] {
        assert!(repo.request("scm.stage", json!({"path":magic})).is_err());
    }
}
#[test]
fn real_git_config_origin_format_is_nul_paired() {
    let mut repo = Repo::new();
    repo.write(
        "included.conf",
        "[filter \"CaseSensitive\"]\n clean = cat\n",
    );
    repo.command(&["config", "include.path", "../included.conf"]);
    let config = repo.command(&["config", "--includes", "-z", "--show-origin", "--list"]);
    let records: Vec<_> = config
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(records.len() % 2, 0);
    assert!(
        records
            .as_chunks::<2>()
            .0
            .iter()
            .any(|pair| pair[0].starts_with(b"file:")
                && pair[1].starts_with(b"filter.CaseSensitive.clean\n"))
    );
}

#[test]
fn real_guest_git_refuses_empty_and_missing_mutable_includes() {
    let mut repo = Repo::new();
    repo.write("visible.txt", "data");
    repo.baseline();
    repo.command(&["config", "include.path", "../included.conf"]);
    for content in [
        None,
        Some(""),
        Some("[filter \"NewDriver\"]\n clean = cat\n"),
    ] {
        if let Some(content) = content {
            repo.write("included.conf", content);
        }
        assert!(repo.request("scm.status", json!({})).is_err());
    }
}
#[test]
fn real_guest_git_refuses_config_editable_in_another_shared_project() {
    let mut repo = Repo::new();
    let other = repo._dir.path().join("other-project");
    std::fs::create_dir(&other).unwrap();
    let include = other
        .join("future.conf")
        .to_string_lossy()
        .replace('\\', "/");
    repo.command(&["config", "include.path", &include]);
    let root = repo.git.root.clone();
    let roots = vec![root.clone(), other];
    let result = dispatch_with_roots(
        &mut repo.git,
        &root,
        &roots,
        None,
        true,
        true,
        "scm.status",
        &json!({}),
        &repo.hooks,
        &mut || Ok(()),
    );
    assert!(result.is_err());
}

#[test]
fn real_guest_file_history_is_scoped_paged_and_redacted() {
    let mut repo = Repo::new();
    repo.write("visible[1].txt", "first");
    repo.write("visible1.txt", "other");
    repo.write(".env", "private");
    repo.baseline();
    repo.write("visible1.txt", "unrelated");
    repo.command(&["commit", "-qam", "unrelated"]);
    repo.write("visible[1].txt", "second");
    repo.command(&[
        "commit",
        "-qam",
        "contact person@example.org or other%40example.org",
    ]);
    repo.write("visible[1].txt", "unsaved");
    let query = |limit| json!({"path":"visible[1].txt","limit":limit});
    let page = repo.request("scm.fileHistory", query(1)).unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["hasMore"], true);
    assert_eq!(page["uncommitted"], true);
    assert_eq!(page["rel"], "visible[1].txt");
    assert!(page["items"][0]["time"].as_i64().unwrap() > 1_000_000_000_000);
    assert!(!page.to_string().contains("example."));
    let all = repo.request("scm.fileHistory", query(50)).unwrap();
    assert_eq!(all["hasMore"], false);
    assert_eq!(all["items"].as_array().unwrap().len(), 2);
    assert_eq!(all["items"][1]["message"], "baseline");
    assert!(repo.request("scm.fileHistory", query(201)).is_err());
    repo.command(&["config", "log.follow", "true"]);
    repo.command(&["mv", "visible[1].txt", "renamed.txt"]);
    repo.command(&["commit", "-qm", "renamed"]);
    let renamed = repo
        .request("scm.fileHistory", json!({"path":"renamed.txt"}))
        .unwrap();
    assert_eq!(renamed["items"].as_array().unwrap().len(), 3);
    assert_eq!(renamed["items"][2]["message"], "baseline");
    assert_eq!(renamed["items"][0]["message"], "renamed");
    for path in [".env", ".git/config", "../outside", ""] {
        assert!(
            repo.request("scm.fileHistory", json!({"path":path}))
                .is_err()
        );
    }
}

#[test]
fn real_guest_file_history_stops_before_private_and_outside_renames() {
    let mut repo = Repo::new();
    repo.write("unshared.txt", "outside origin");
    repo.write(" project/.env", "private origin");
    repo.baseline();
    repo.command(&["mv", "unshared.txt", " project/from-outside"]);
    repo.command(&["mv", " project/.env", " project/from-private"]);
    repo.command(&["commit", "-qm", "hidden origin details"]);
    repo.write(" project/from-outside", "visible edit");
    repo.write(" project/from-private", "visible edit too");
    repo.command(&["commit", "-qam", "public change"]);
    repo.git.root = repo.git.root.join(" project");
    for path in ["from-outside", "from-private"] {
        let page = repo
            .request("scm.fileHistory", json!({"path":path,"limit":1}))
            .unwrap();
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        assert_eq!(page["items"][0]["message"], "public change");
        assert_eq!(page["hasMore"], false);
    }
}

#[test]
fn guest_history_sticky_rename_boundary_discards_the_rename_record() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = b"\0latest\0\nM\0folder/new.txt\0\0boundary\0\nR100\0folder/old.txt\0folder/new.txt\0\0older\0\nA\0folder/old.txt\0";
    let retained =
        visible_history_records(bytes, "folder/", "new.txt", dir.path(), true, &|names| {
            assert_eq!(names, ["old.txt"]);
            Ok(vec![true])
        })
        .unwrap();
    assert_eq!(retained, ["latest"]);
}
