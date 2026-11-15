use super::*;
use std::cell::RefCell;
use std::rc::Rc;

/// A git that never spawns: it answers canned bytes by the subcommand in the args, and
/// records every invocation (and whether a mutation ran) for ordering assertions.
struct FakeGit {
    rules: Vec<(&'static str, Vec<u8>)>,
    log: Rc<RefCell<Vec<String>>>,
    calls: Rc<RefCell<Vec<Vec<String>>>>,
    truncate_config: bool,
}
impl FakeGit {
    fn new(log: Rc<RefCell<Vec<String>>>) -> Self {
        Self {
            rules: Vec::new(),
            log,
            calls: Rc::new(RefCell::new(Vec::new())),
            truncate_config: false,
        }
    }
    fn on(mut self, needle: &'static str, out: &[u8]) -> Self {
        self.rules.push((needle, out.to_vec()));
        self
    }
    fn with_truncated_config(mut self) -> Self {
        self.truncate_config = true;
        self
    }
    fn subcommand(args: &[&str]) -> String {
        let mut it = args.iter();
        for a in it.by_ref() {
            if *a == "--no-optional-locks" {
                break;
            }
        }
        it.next().map(|s| s.to_string()).unwrap_or_default()
    }
}
impl GitRun for FakeGit {
    fn run(&mut self, args: &[&str], _input: Option<&[u8]>) -> Result<Capture> {
        self.calls
            .borrow_mut()
            .push(args.iter().map(|s| s.to_string()).collect());
        let joined = args.join(" ");
        let sub = Self::subcommand(args);
        if matches!(sub.as_str(), "add" | "reset" | "checkout" | "commit") {
            self.log.borrow_mut().push(format!("git:{sub}"));
        }
        if sub == "config" && self.truncate_config {
            return Ok(Capture {
                bytes: Vec::new(),
                truncated: true,
            });
        }
        if args.contains(&"--show-origin") {
            return Ok(Capture::default());
        }
        for (needle, out) in &self.rules {
            if joined.contains(needle) {
                return Ok(Capture {
                    bytes: out.clone(),
                    truncated: false,
                });
            }
        }
        Ok(Capture {
            bytes: Vec::new(),
            truncated: false,
        })
    }
}

fn no_revalidate() -> impl FnMut() -> Result<()> {
    || Ok(())
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), b"fn main(){}").unwrap();
    dir
}

#[test]
fn fixed_prefix_turns_off_hooks_signing_transport_and_helpers() {
    let hooks = tempfile::tempdir().unwrap();
    let prefix = hardening_prefix(&[], hooks.path()).join(" ");
    for needle in [
        "core.hooksPath=",
        "protocol.ext.allow=never",
        "core.fsmonitor=false",
        "log.showSignature=false",
        "commit.gpgSign=false",
        "tag.gpgSign=false",
        "credential.helper=",
        "core.sshCommand=ssh",
        "core.alternateRefsCommand=",
        "submodule.recurse=false",
        "--literal-pathspecs",
    ] {
        assert!(prefix.contains(needle), "missing {needle} in {prefix}");
    }
    // The empty hooks directory, not a path anyone on the drive could create.
    assert!(prefix.contains(&hooks.path().to_string_lossy().replace('\\', "/")));
}

#[test]
fn parse_risky_keeps_program_runners_and_drops_harmless_values() {
    let cfg = "core.fsmonitor\nfalse\0\
               filter.lfs.process\ngit-lfs filter-process\0\
               diff.odf.textconv\npandoc\0\
               merge.ours.driver\ntrue\0\
               remote.evil.url\next::sh -c id\0\
               remote.origin.url\nhttps://example.com/x.git\0\
               protocol.ext.allow\nnever\0\
               gpg.evil.program\n/tmp/x\0";
    let keys = parse_risky(cfg.as_bytes());
    assert!(keys.contains(&"filter.lfs.process".to_string()));
    assert!(keys.contains(&"diff.odf.textconv".to_string()));
    assert!(keys.contains(&"merge.ours.driver".to_string()));
    assert!(keys.contains(&"remote.evil.url".to_string()));
    assert!(keys.contains(&"gpg.evil.program".to_string()));
    // fsmonitor=false, a non-ext remote URL and protocol.ext.allow=never run nothing.
    assert!(!keys.contains(&"core.fsmonitor".to_string()));
    assert!(!keys.contains(&"remote.origin.url".to_string()));
    assert!(!keys.contains(&"protocol.ext.allow".to_string()));
}

#[test]
fn named_runners_are_neutralized_with_required_off() {
    let hooks = tempfile::tempdir().unwrap();
    let prefix = hardening_prefix(
        &[
            "filter.lfs.process".into(),
            "diff.odf.textconv".into(),
            "merge.ours.driver".into(),
        ],
        hooks.path(),
    )
    .join(" ");
    assert!(prefix.contains("filter.lfs.clean="));
    assert!(prefix.contains("filter.lfs.smudge="));
    assert!(prefix.contains("filter.lfs.process="));
    assert!(prefix.contains("filter.lfs.required=false"));
    assert!(prefix.contains("diff.odf.textconv="));
    assert!(prefix.contains("diff.odf.command="));
    assert!(prefix.contains("merge.ours.driver="));
}

#[test]
fn nested_metadata_is_filtered_from_scm_even_without_ignore_hiding() {
    let dir = project();
    let bare = dir.path().join("service");
    std::fs::create_dir_all(bare.join("objects")).unwrap();
    std::fs::create_dir_all(bare.join("refs")).unwrap();
    std::fs::write(bare.join("HEAD"), "synthetic").unwrap();
    std::fs::write(bare.join("config"), "synthetic private metadata").unwrap();
    let hooks = tempfile::tempdir().unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut git = FakeGit::new(log.clone())
        .on(
            "status -z",
            b" M service/config\0 D ordinary-deleted.txt\0 D service/deleted\0",
        )
        .on("--cached --no-renames --name-only", b"service/config\0");
    let out = dispatch_with(
        &mut git,
        dir.path(),
        true,
        false,
        "scm.status",
        &json!({}),
        hooks.path(),
        &mut no_revalidate(),
    )
    .unwrap();
    assert_eq!(out["entries"].as_array().unwrap().len(), 1);
    assert_eq!(out["entries"][0]["path"], "ordinary-deleted.txt");
    let err = dispatch_with(
        &mut git,
        dir.path(),
        true,
        false,
        "scm.commit",
        &json!({"message":"synthetic"}),
        hooks.path(),
        &mut no_revalidate(),
    )
    .unwrap_err();
    assert!(
        err.contains("Unstage changes outside this project"),
        "{err}"
    );
    assert!(log.borrow().is_empty());
}

#[test]
fn status_and_diff_hide_private_and_out_of_project_names() {
    let (rows, hidden) = parse_status(
        b" M a.txt\0 M .env\0 M src/main.rs\0R  new.txt\0.git/config\0",
        "",
        true,
    );
    let paths: Vec<&str> = rows.iter().map(|r| r["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["a.txt", "src/main.rs"]);
    // .env (secret) and the rename whose origin is .git/config are hidden, not named.
    assert_eq!(hidden, 2);

    let (visible, hidden) = filter_visible(&["a.txt".into(), ".env".into(), "id_rsa".into()], true);
    assert_eq!(visible, ["a.txt"]);
    assert_eq!(hidden, 2);
}

#[test]
fn stage_uses_literal_pathspecs_and_the_verified_relative_path() {
    let dir = project();
    let hooks = tempfile::tempdir().unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut git = FakeGit::new(log.clone()).on("check-attr", b"src/main.rs\0filter\0unspecified\0");
    let calls = git.calls.clone();
    let mut reval = no_revalidate();
    let out = dispatch_with(
        &mut git,
        dir.path(),
        true,
        true,
        "scm.stage",
        &json!({"path": "src/main.rs"}),
        hooks.path(),
        &mut reval,
    )
    .unwrap();
    assert_eq!(out["ok"], true);
    let add = calls
        .borrow()
        .iter()
        .find(|c| c.iter().any(|a| a == "add"))
        .cloned()
        .expect("an add ran");
    assert!(add.contains(&"--literal-pathspecs".to_string()));
    // The verified relative path, with '/', not the guest's raw string.
    assert_eq!(add.last().unwrap(), "src/main.rs");
}

#[test]
fn a_pathspec_escape_is_refused_before_any_git_runs() {
    let dir = project();
    let hooks = tempfile::tempdir().unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut git = FakeGit::new(log.clone());
    let mut reval = no_revalidate();
    let mut bad_paths = vec![":/", "../outside"];
    if cfg!(windows) {
        bad_paths.extend(["*", "src/*"]);
    }
    for bad in bad_paths {
        let err = dispatch_with(
            &mut git,
            dir.path(),
            true,
            true,
            "scm.stage",
            &json!({"path": bad}),
            hooks.path(),
            &mut reval,
        )
        .unwrap_err();
        assert_eq!(err, "This path is not shared.");
    }
    assert!(log.borrow().is_empty(), "no git mutation should have run");
}

#[test]
fn a_filtered_file_is_never_staged_or_restored() {
    let dir = project();
    let hooks = tempfile::tempdir().unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut git = FakeGit::new(log.clone()).on("check-attr", b"a.txt\0filter\0git-crypt\0");
    let mut reval = no_revalidate();
    let err = dispatch_with(
        &mut git,
        dir.path(),
        true,
        true,
        "scm.stage",
        &json!({"path": "a.txt"}),
        hooks.path(),
        &mut reval,
    )
    .unwrap_err();
    assert!(err.contains("git filter"));
    assert!(log.borrow().is_empty(), "filtered file must not be staged");
}

#[test]
fn commit_is_refused_when_anything_outside_the_project_is_staged() {
    let dir = project();
    let hooks = tempfile::tempdir().unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    // Project is a subfolder of the repo; a path above it is staged.
    let mut git = FakeGit::new(log.clone())
        .on("--show-prefix", b"proj/\n")
        .on(
            "--cached --no-renames --name-only",
            b"proj/a.txt\0other/secret\0",
        );
    let mut reval = no_revalidate();
    let err = dispatch_with(
        &mut git,
        dir.path(),
        true,
        true,
        "scm.commit",
        &json!({"message": "x"}),
        hooks.path(),
        &mut reval,
    )
    .unwrap_err();
    assert!(err.contains("outside this project"));
    assert!(!log.borrow().iter().any(|e| e == "git:commit"));
}

#[test]
fn commit_revalidates_immediately_before_committing() {
    let dir = project();
    let hooks = tempfile::tempdir().unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut git = FakeGit::new(log.clone())
        .on("--show-prefix", b"\n")
        .on("--cached --no-renames --name-only", b"a.txt\0");
    let reval_log = log.clone();
    let mut reval = move || {
        reval_log.borrow_mut().push("revalidate".to_string());
        Ok(())
    };
    let out = dispatch_with(
        &mut git,
        dir.path(),
        true,
        true,
        "scm.commit",
        &json!({"message": "ok"}),
        hooks.path(),
        &mut reval,
    )
    .unwrap();
    assert_eq!(out["ok"], true);
    let order = log.borrow().clone();
    let r = order.iter().position(|e| e == "revalidate").unwrap();
    let c = order.iter().position(|e| e == "git:commit").unwrap();
    assert!(r < c, "revalidate must precede the commit: {order:?}");
}

#[test]
fn a_revoked_right_stops_the_commit_even_after_the_scope_check() {
    let dir = project();
    let hooks = tempfile::tempdir().unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut git = FakeGit::new(log.clone())
        .on("--show-prefix", b"\n")
        .on("--cached --no-renames --name-only", b"a.txt\0");
    let mut reval = || Err("Access was revoked.".to_string());
    let err = dispatch_with(
        &mut git,
        dir.path(),
        true,
        true,
        "scm.commit",
        &json!({"message": "ok"}),
        hooks.path(),
        &mut reval,
    )
    .unwrap_err();
    assert_eq!(err, "Access was revoked.");
    assert!(!log.borrow().iter().any(|e| e == "git:commit"));
}

#[test]
fn git_errors_are_bounded_and_never_leak_the_host_path() {
    let dir = tempfile::tempdir().unwrap();
    // The status run fails; run_bytes-style errors arrive as the stderr text. Here the
    // fake cannot fail a specific op, so drive the bound directly.
    let secret_host = format!("fatal: {}/.ssh/id_rsa is unreadable", dir.path().display());
    let bounded = bounded_err(dir.path(), &secret_host);
    assert!(!bounded.contains(&dir.path().to_string_lossy().into_owned()));
    assert!(bounded.len() <= 300 + 8);
}

#[test]
fn a_mutation_needs_the_git_right() {
    let dir = project();
    let hooks = tempfile::tempdir().unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut git = FakeGit::new(log);
    let mut reval = no_revalidate();
    let err = dispatch_with(
        &mut git,
        dir.path(),
        false,
        true,
        "scm.commit",
        &json!({"message": "x"}),
        hooks.path(),
        &mut reval,
    )
    .unwrap_err();
    assert!(err.contains("Git permission"));
}

#[test]
fn status_strips_the_project_prefix_and_emits_a_compat_string() {
    let dir = project();
    let hooks = tempfile::tempdir().unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    // The project is "proj/" under the repository; .env is secret, other/x is outside it.
    let mut git = FakeGit::new(log).on("--show-prefix", b"proj/\n").on(
        "status -z --porcelain",
        b" M proj/a.txt\0 M proj/.env\0 M other/x\0",
    );
    let mut reval = no_revalidate();
    let out = dispatch_with(
        &mut git,
        dir.path(),
        false,
        true,
        "scm.status",
        &json!({}),
        hooks.path(),
        &mut reval,
    )
    .unwrap();
    let entries = out["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["path"], "a.txt");
    assert_eq!(out["hidden"], 2);
    let status = out["status"].as_str().unwrap();
    assert!(status.contains("a.txt"));
    assert!(!status.contains(".env") && !status.contains("other"));
}

#[test]
fn diff_honors_the_requested_file_relative_to_the_project() {
    let dir = project();
    let hooks = tempfile::tempdir().unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut git = FakeGit::new(log).on("diff --relative --no-renames --no-color", b"@@ a patch @@");
    let calls = git.calls.clone();
    let mut reval = no_revalidate();
    let out = dispatch_with(
        &mut git,
        dir.path(),
        false,
        true,
        "scm.diff",
        &json!({"path": "src/main.rs"}),
        hooks.path(),
        &mut reval,
    )
    .unwrap();
    assert!(out["diff"].as_str().unwrap().contains("a patch"));
    let diff = calls
        .borrow()
        .iter()
        .find(|c| c.iter().any(|a| a == "diff") && c.iter().any(|a| a == "--relative"))
        .cloned()
        .expect("a scoped diff ran");
    assert_eq!(diff.last().unwrap(), "src/main.rs");
    assert!(!diff.iter().any(|a| a == "."), "not a whole-tree diff");
}

#[test]
fn an_unreadable_or_truncated_config_fails_closed() {
    let dir = project();
    let hooks = tempfile::tempdir().unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut git = FakeGit::new(log.clone())
        .with_truncated_config()
        .on("--show-prefix", b"\n")
        .on("--cached --no-renames --name-only", b"a.txt\0");
    let mut reval = no_revalidate();
    let err = dispatch_with(
        &mut git,
        dir.path(),
        true,
        true,
        "scm.commit",
        &json!({"message": "x"}),
        hooks.path(),
        &mut reval,
    )
    .unwrap_err();
    assert!(err.contains("could not be read safely"));
    assert!(
        log.borrow().is_empty(),
        "nothing runs when config is unsafe"
    );
}

#[test]
fn push_pull_fetch_stay_with_the_host() {
    let dir = project();
    let hooks = tempfile::tempdir().unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut git = FakeGit::new(log);
    let mut reval = no_revalidate();
    for op in ["scm.push", "scm.pull", "scm.fetch"] {
        let err = dispatch_with(
            &mut git,
            dir.path(),
            true,
            true,
            op,
            &json!({}),
            hooks.path(),
            &mut reval,
        )
        .unwrap_err();
        assert!(err.contains("exclusively with the host"));
    }
}

#[test]
fn case_sensitive_driver_names_keep_their_exact_subsection() {
    let keys = parse_risky(b"filter.LFS.process\ncommand\0diff.MyDriver.textconv\ncommand\0");
    let hooks = tempfile::tempdir().unwrap();
    let args = hardening_prefix(&keys, hooks.path());
    assert!(args.contains(&"filter.LFS.process=".to_owned()));
    assert!(!args.contains(&"filter.lfs.process=".to_owned()));
    assert!(args.contains(&"diff.MyDriver.textconv=".to_owned()));
}

#[test]
fn status_omits_incomplete_records_and_private_worktree_rename_origins() {
    let (rows, _) = parse_status(b" M visible.txt\0?? private-key", "", false);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["path"], "visible.txt");
    let (rows, hidden) = parse_status(b" R public.txt\0.env\0", "", true);
    assert!(rows.is_empty());
    assert_eq!(hidden, 1);
    let (rows, _) = parse_status("€ bogus\0".as_bytes(), "", false);
    assert!(rows.is_empty());
}

#[test]
fn distinct_driver_subsection_case_is_never_deduplicated() {
    let keys = parse_risky(b"filter.LFS.process\nupper\0filter.lfs.process\nlower\0diff.MyDriver.textconv\nupper\0diff.mydriver.textconv\nlower\0");
    let hooks = tempfile::tempdir().unwrap();
    let args = hardening_prefix(&keys, hooks.path());
    for key in [
        "filter.LFS.process=",
        "filter.lfs.process=",
        "filter.LFS.required=false",
        "filter.lfs.required=false",
        "diff.MyDriver.textconv=",
        "diff.mydriver.textconv=",
    ] {
        assert!(args.iter().any(|arg| arg == key), "missing {key}");
    }
}
