//! Guest read I/O never holds the host authority mutex.
use super::*;
static ACTIVE_READS: AtomicUsize = AtomicUsize::new(0);
struct ReadSlot;
impl ReadSlot {
    fn acquire() -> Result<Self> {
        let mut active = ACTIVE_READS.load(Ordering::SeqCst);
        loop {
            if active >= 4 {
                return Err("Guest file reads are busy. Try again later.".into());
            }
            match ACTIVE_READS.compare_exchange_weak(
                active,
                active + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Ok(Self),
                Err(current) => active = current,
            }
        }
    }
}
impl Drop for ReadSlot {
    fn drop(&mut self) {
        ACTIVE_READS.fetch_sub(1, Ordering::SeqCst);
    }
}
struct Grant {
    ignored: Arc<ignored_paths::IgnoredPaths>,
    roots: Vec<PathBuf>,
    project: SharedProject,
    rights: ShareRights,
    hide: bool,
}
fn authorize(inner: &HostInner, guest: &str, project: Option<u64>) -> Result<Grant> {
    if inner.stopped.load(Ordering::SeqCst) {
        return Err("Sharing has stopped.".into());
    }
    let index = inner
        .guests
        .iter()
        .position(|s| s.guest.id == guest)
        .ok_or("Guest session ended.")?;
    let slot = &inner.guests[index];
    if !matches!(slot.decision, GuestDecision::Approved)
        || slot.guest.state != GuestConnectionState::Joined
    {
        return Err("Guest session is not active.".into());
    }
    Ok(Grant {
        ignored: inner.ignored_paths.clone(),
        roots: inner
            .shared_projects
            .iter()
            .map(|p| p.path.clone())
            .collect(),
        project: inner.find_shared_project_for_guest(index, project)?,
        rights: slot.guest.rights,
        hide: inner.settings.hide_ignored,
    })
}
fn run_authorized(
    inner: &Arc<Mutex<HostInner>>,
    guest: &str,
    req: &Value,
    read: impl FnOnce(&Grant) -> Result<Value>,
) -> Result<Value> {
    let grant = {
        let guard = inner.lock().unwrap_or_else(|e| e.into_inner());
        authorize(&guard, guest, req.get("projectId").and_then(Value::as_u64))?
    };
    let result = read(&grant);
    let guard = inner.lock().unwrap_or_else(|e| e.into_inner());
    let current = authorize(&guard, guest, Some(grant.project.id))?;
    if current.roots != grant.roots
        || current.project.path != grant.project.path
        || current.hide != grant.hide
        || current.rights != grant.rights
    {
        return Err("Shared file access changed; refresh the current project.".into());
    }
    result
        .map_err(|error| guest_text::task_title(&error, &grant.project.path.to_string_lossy(), 300))
}
fn check_path(grant: &Grant, relative: &str, target: &Path) -> Result<()> {
    let paths = [
        relative.replace('\\', "/"),
        target
            .strip_prefix(&grant.project.path)
            .map_err(|_| "Invalid project scope")?
            .to_string_lossy()
            .replace('\\', "/"),
    ];
    if grant.hide
        && grant
            .ignored
            .hidden(&grant.project.path, &grant.roots, &paths)?
            .into_iter()
            .any(|v| v)
    {
        return Err("File unavailable".into());
    }
    Ok(())
}
fn read(grant: &Grant, req: &Value) -> Result<Value> {
    let rights = grant.rights;
    let hide_ignored = grant.hide;
    match req.get("op").and_then(Value::as_str).unwrap_or("") {
        "files.list" => {
            let proj = &grant.project;
            // This guard is independent of ignore visibility and Edit/Type permissions.
            super::guest_git_host::HostGit::new(&proj.path).repository()?;
            let relative = req.get("path").and_then(Value::as_str).unwrap_or("");
            let target = if relative.is_empty() {
                proj.path.clone()
            } else {
                share_path_checked(&proj.path, relative, hide_ignored)?
            };
            if !relative.is_empty() {
                check_path(grant, relative, &target)?;
            }
            let entries = list_dir_entries(&proj.path, &target)?;
            let filtered: Vec<Value> = entries
                .into_iter()
                .filter(|e| {
                    let rel = e.path.strip_prefix(&proj.path).unwrap_or(&e.path);
                    !private_path(rel) && !(hide_ignored && secret_name_path(rel))
                })
                .map(|e| {
                    let rel = e
                        .path
                        .strip_prefix(&proj.path)
                        .unwrap_or(&e.path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    json!({"name": e.name, "path": rel, "directory": e.directory})
                })
                .collect();
            let filtered = if grant.hide {
                let paths: Vec<String> = filtered
                    .iter()
                    .map(|e| e["path"].as_str().unwrap().to_string())
                    .collect();
                let hidden = grant.ignored.hidden(&proj.path, &grant.roots, &paths)?;
                filtered
                    .into_iter()
                    .zip(hidden)
                    .filter_map(|(entry, hide)| (!hide).then_some(entry))
                    .collect::<Vec<_>>()
            } else {
                filtered
            };
            Ok(json!({"entries": filtered, "readOnly": !rights.edit}))
        }
        "files.read" => {
            let proj = &grant.project;
            let relative = req.get("path").and_then(Value::as_str).unwrap_or("");
            let target = share_path_checked(&proj.path, relative, hide_ignored)?;
            check_path(grant, relative, &target)?;
            let hooks = hook_paths::discover(&proj.path, &grant.roots)?;
            let mut bytes = Vec::new();
            std::fs::File::open(&target)
                .map_err(|_| "File unavailable")?
                .take((MAX_FILE + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| "File read failed")?;
            if bytes.len() > MAX_FILE {
                return Err("Guest file limit is 8 MiB.".into());
            }
            Ok(json!({
                "name": target.file_name().unwrap_or_default().to_string_lossy(),
                "hash": file_version::hash(&bytes),
                "data": B64.encode(bytes),
                "readOnly": !rights.edit || ((hook_paths::protects(Path::new(relative), &hooks) || hook_paths::protects(target.strip_prefix(&proj.path).map_err(|_| "Invalid project scope")?, &hooks)) && !rights.r#type),
            }))
        }
        _ => Err("Unsupported guest file read.".into()),
    }
}
pub(super) fn dispatch(inner: &Arc<Mutex<HostInner>>, guest: &str, req: &Value) -> Value {
    let result = run_authorized(inner, guest, req, |grant| {
        let _slot = ReadSlot::acquire()?;
        read(grant, req)
    });
    match result {
        Ok(value) => json!({"result":value}),
        Err(error) => json!({"error":error}),
    }
}

fn legacy_grant(inner: &HostInner, token: &[u8]) -> Result<Grant> {
    if inner.stopped.load(Ordering::SeqCst) {
        return Err("Sharing stopped".into());
    }
    let invite = inner
        .invites
        .iter()
        .find(|inv| {
            token.len() == 32
                && bool::from(token.ct_eq(&inv.token_bytes))
                && inv.status(now_epoch_ms()) == InviteStatus::Live
        })
        .ok_or("Invalid invitation")?;
    let project = inner
        .shared_projects
        .iter()
        .find(|p| invite.projects.contains(&p.id))
        .ok_or("No shared project")?;
    Ok(Grant {
        project: project.clone(),
        rights: ShareRights::view_only(),
        hide: inner.settings.hide_ignored,
        roots: inner
            .shared_projects
            .iter()
            .map(|p| p.path.clone())
            .collect(),
        ignored: inner.ignored_paths.clone(),
    })
}
pub(super) fn dispatch_legacy(inner: &Arc<Mutex<HostInner>>, token: &[u8], req: &Value) -> Value {
    let result: Result<Value> = (|| {
        let grant = legacy_grant(&inner.lock().unwrap_or_else(|e| e.into_inner()), token)?;
        let _slot = ReadSlot::acquire()?;
        let op = match req.get("operation").and_then(Value::as_str) {
            Some("list") => "files.list",
            Some("read") => "files.read",
            _ => return Err("Unsupported operation".into()),
        };
        let result = read(
            &grant,
            &json!({"op":op,"path":req.get("path").and_then(Value::as_str).unwrap_or("")}),
        );
        let current = legacy_grant(&inner.lock().unwrap_or_else(|e| e.into_inner()), token)?;
        if current.project.id != grant.project.id
            || current.project.path != grant.project.path
            || current.roots != grant.roots
            || current.hide != grant.hide
        {
            return Err("Shared scope changed".into());
        }
        result
    })();
    match result {
        Ok(value) => json!({"result":value}),
        Err(_) => json!({"error":"Request unavailable or not allowed by this share."}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Host) {
        let dir = tempfile::Builder::new()
            .prefix("file-read-authority-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap();
        let host = Host::start(
            dir.path(),
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1".parse().unwrap(),
        )
        .unwrap();
        host.inner.lock().unwrap().guests.push(GuestSlot {
            decision: GuestDecision::Approved,
            guest: ConnectedGuest {
                id: "guest".into(),
                name: "Synthetic".into(),
                state: GuestConnectionState::Joined,
                code: String::new(),
                invite_id: None,
                invite_label: String::new(),
                projects: vec![1],
                rights: ShareRights::collaborate(),
                trusted: false,
                trust_id: None,
                identity_hex: None,
                since_ms: 0,
                watching_pane: None,
                typing_pane: None,
            },
        });
        (dir, host)
    }

    const IGNORE_PROBE: &str = "TESSEL_SYNTHETIC_IGNORE_PROBE";
    fn empty_repo(root: &Path) {
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        std::fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            root.join(".git/config"),
            "[core]\n repositoryformatversion = 0\n bare = false\n",
        )
        .unwrap();
    }
    #[test]
    fn isolated_ignore_probe() {
        if std::env::var_os(IGNORE_PROBE).is_none() {
            return;
        }
        let (dir, host) = fixture();
        empty_repo(dir.path());
        std::fs::write(dir.path().join(".gitignore"), "*.log\nhidden/\n").unwrap();
        std::fs::write(dir.path().join("trace.log"), "private").unwrap();
        std::fs::write(dir.path().join("private[1].log"), "bracket secret").unwrap();
        std::fs::write(dir.path().join("literal[1].txt"), "public").unwrap();
        std::fs::create_dir(dir.path().join("hidden")).unwrap();
        std::fs::write(dir.path().join("hidden/item.txt"), "private").unwrap();
        let rpc = |op: &str, path: &str| {
            dispatch(
                &host.inner,
                "guest",
                &json!({"op":op,"projectId":1,"path":path}),
            )
        };
        assert!(rpc("files.read", "literal[1].txt").get("result").is_some());
        assert!(rpc("files.read", "trace.log").get("error").is_some());
        assert!(rpc("files.read", "private[1].log").get("error").is_some());
        let listing = rpc("files.list", "");
        let entries = listing["result"]["entries"].as_array().unwrap();
        let listed: Vec<_> = entries
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(listed, vec![".gitignore", "literal[1].txt"], "{listing}");
        for name in [".editorconfig", ".env.example", "prod.env.sample"] {
            std::fs::write(dir.path().join(name), "example").unwrap();
            assert!(rpc("files.read", name).get("result").is_some(), "{name}");
        }
        std::fs::write(dir.path().join(".env.local"), "private").unwrap();
        assert!(rpc("files.read", ".env.local").get("error").is_some());
        let token = host.inner.lock().unwrap().invites[0].token_bytes;
        assert!(
            dispatch_legacy(
                &host.inner,
                &token,
                &json!({"operation":"read","path":"trace.log"})
            )
            .get("error")
            .is_some()
        );
        for request in [
            json!({"op":"files.write","path":"trace.log","data":B64.encode(b"changed")}),
            json!({"op":"files.create","path":"new.log"}),
            json!({"op":"files.rename","path":"hidden","to":"moved"}),
            json!({"op":"files.delete","path":"hidden"}),
        ] {
            let result =
                super::super::guest_file_mutations::dispatch(&host.inner, "guest", &request);
            assert!(result.get("error").is_some(), "{request}: {result}");
        }
        assert_eq!(
            std::fs::read(dir.path().join("trace.log")).unwrap(),
            b"private"
        );
        assert!(dir.path().join("hidden/item.txt").exists());
        std::fs::write(dir.path().join(".gitignore"), "").unwrap();
        assert!(rpc("files.read", "trace.log").get("error").is_some());
        assert!(rpc("files.read", "private[1].log").get("error").is_some());
        assert!(rpc("files.read", "hidden/item.txt").get("error").is_some());
        host.set_hide_ignored(false);
        assert!(rpc("files.read", "trace.log").get("result").is_some());
        host.set_hide_ignored(true);
        assert!(rpc("files.read", "trace.log").get("error").is_some());
        assert!(rpc("files.read", "private[1].log").get("error").is_some());
        let fresh = ignored_paths::IgnoredPaths::default();
        assert_eq!(
            fresh
                .hidden(dir.path(), &[dir.path().into()], &["trace.log".into()])
                .unwrap(),
            vec![false]
        );
        std::fs::write(dir.path().join(".gitignore"), "*.log\n").unwrap();
        std::fs::write(dir.path().join("never-seen[2].log"), "never listed secret").unwrap();
        // The host may force a previously hidden file into the index. Every SCM
        // surface must retain the earlier privacy decision for this share.
        crate::git::run_guest_bytes(
            dir.path(),
            &[
                "-c",
                "core.fsmonitor=false",
                "add",
                "-f",
                "--",
                "trace.log",
                "private[1].log",
                "never-seen[2].log",
            ],
            None,
            Duration::from_secs(10),
        )
        .unwrap();
        std::fs::write(dir.path().join("private[1].log"), "changed bracket secret").unwrap();
        let scm = |request: Value| {
            super::super::guest_git_dispatch::dispatch(&host.inner, "guest", &request)
        };
        let raw_status = crate::git::run_guest_bytes(
            dir.path(),
            &["-c", "core.fsmonitor=false", "status", "--porcelain=v1"],
            None,
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&raw_status.bytes).contains("never-seen[2].log"));
        let status = scm(json!({"op":"scm.status","projectId":1}));
        assert!(status.get("result").is_some(), "{status}");
        assert!(!status.to_string().contains("private[1].log"));
        assert!(!status.to_string().contains("trace.log"));
        assert!(status.to_string().contains("never-seen[2].log"));
        for request in [
            json!({"op":"scm.diff"}),
            json!({"op":"scm.diff","path":"private[1].log"}),
        ] {
            let diff = scm(request);
            assert_eq!(diff["result"]["diff"], "", "{diff}");
        }
        for op in ["scm.stage", "scm.unstage", "scm.discard"] {
            let result = scm(json!({"op":op,"path":"private[1].log"}));
            assert!(result.get("error").is_some(), "{op}: {result}");
        }
        // Leave ONLY the never-observed tracked file staged: rejection must come
        // from --no-index, not an older sticky decision or a missing Git identity.
        crate::git::run_guest_bytes(
            dir.path(),
            &["rm", "--cached", "-f", "--", "trace.log", "private[1].log"],
            None,
            Duration::from_secs(10),
        )
        .unwrap();
        let commit = scm(json!({"op":"scm.commit","message":"must not include hidden files"}));
        assert!(
            commit["error"]
                .as_str()
                .unwrap()
                .contains("Unstage changes outside"),
            "{commit}"
        );
        assert!(
            rpc("files.read", "never-seen[2].log")
                .get("result")
                .is_some()
        );
        assert!(commit.get("error").is_some(), "{commit}");
        assert!(!dir.path().join(".git/refs/heads/main").exists());
        std::fs::write(dir.path().join(".git/config"), "[broken").unwrap();
        assert!(rpc("files.read", "literal[1].txt").get("error").is_some());
        assert!(rpc("files.list", "").get("error").is_some());
        empty_repo(dir.path());
        assert!(rpc("files.read", "literal[1].txt").get("result").is_some());
        host.inner.lock().unwrap().invites[0].projects.clear();
        assert!(
            dispatch_legacy(
                &host.inner,
                &token,
                &json!({"operation":"read","path":"literal[1].txt"})
            )
            .get("error")
            .is_some()
        );
    }
    #[test]
    fn real_ignore_rules_stick_across_edits_and_cover_legacy_and_mutations() {
        run_isolated_probe("isolated_ignore_probe");
    }
    fn run_isolated_probe(probe: &str) {
        // Keep synthetic HOME under the isolated checkout, away from user data.
        let home = tempfile::Builder::new()
            .prefix("synthetic-share-home-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap();
        std::fs::write(home.path().join(".gitconfig"), "").unwrap();
        let test = format!("{}::{probe}", module_path!().split_once("::").unwrap().1);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child.env_clear();
        for name in [
            "PATH",
            "SystemRoot",
            "WINDIR",
            "COMSPEC",
            "PATHEXT",
            "TEMP",
            "TMP",
        ] {
            if let Some(value) = std::env::var_os(name) {
                child.env(name, value);
            }
        }
        child
            .args(["--exact", &test, "--nocapture"])
            .env(IGNORE_PROBE, "1")
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("XDG_CONFIG_HOME", home.path())
            .env("APPDATA", home.path());
        if probe == "repository_scope_probe" {
            child.env("LC_ALL", "fr_CA.UTF-8").env("LANGUAGE", "fr");
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            child.creation_flags(0x08000000);
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }

    #[test]
    fn real_home_hook_scope_is_expanded_by_git() {
        run_isolated_probe("home_hook_scope_probe");
    }
    #[test]
    fn real_repository_scope_refuses_bare_metadata_and_broken_repositories() {
        run_isolated_probe("repository_scope_probe");
    }
    #[test]
    fn repository_scope_probe() {
        if std::env::var_os(IGNORE_PROBE).is_none() {
            return;
        }
        assert_eq!(std::env::var("LC_ALL").unwrap(), "fr_CA.UTF-8");
        assert_eq!(std::env::var("LANGUAGE").unwrap(), "fr");
        let timeout = Duration::from_secs(10);
        // Outside the checkout: a genuine ordinary folder, not a subfolder of its repo.
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(
            crate::git::run_guest_repository(plain.path(), timeout).unwrap(),
            crate::git::GuestRepository::Plain
        );
        let (dir, host) = fixture();
        empty_repo(dir.path());
        assert!(matches!(
            crate::git::run_guest_repository(dir.path(), timeout).unwrap(),
            crate::git::GuestRepository::Worktree(_)
        ));
        assert!(crate::git::run_guest_repository(&dir.path().join(".git"), timeout).is_err());
        let git_dir = dunce::canonicalize(dir.path().join(".git")).unwrap();
        assert!(git_dir.starts_with(dunce::canonicalize(dir.path()).unwrap()));
        std::fs::remove_dir_all(&git_dir).unwrap();
        // A bare repository has no .git directory, so filename-based detection misses it.
        for name in ["objects", "refs", "hooks"] {
            std::fs::create_dir(dir.path().join(name)).unwrap();
        }
        std::fs::write(dir.path().join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            dir.path().join("config"),
            "[core]\n repositoryformatversion = 0\n bare = true\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("hooks/pre-receive"),
            "synthetic hook; never execute",
        )
        .unwrap();
        assert!(crate::git::run_guest_repository(dir.path(), timeout).is_err());
        for hide in [false, true] {
            {
                let mut inner = host.inner.lock().unwrap();
                inner.settings.hide_ignored = hide;
                inner.guests[0].guest.rights.r#type = true;
            }
            for request in [
                json!({"op":"files.list","path":""}),
                json!({"op":"files.read","path":"config"}),
                json!({"op":"files.read","path":"hooks/pre-receive"}),
            ] {
                assert!(
                    dispatch(&host.inner, "guest", &request)
                        .get("error")
                        .is_some(),
                    "{request}"
                );
            }
            for request in [
                json!({"op":"files.write","path":"config","expectHash":file_version::hash(&std::fs::read(dir.path().join("config")).unwrap()),"data":B64.encode(b"changed")}),
                json!({"op":"files.create","path":"hooks/post-receive"}),
                json!({"op":"files.rename","path":"hooks/pre-receive","to":"hooks/post-receive"}),
                json!({"op":"files.delete","path":"hooks/pre-receive"}),
            ] {
                assert!(
                    super::super::guest_file_mutations::dispatch(&host.inner, "guest", &request)
                        .get("error")
                        .is_some(),
                    "{request}"
                );
            }
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("hooks/pre-receive")).unwrap(),
            "synthetic hook; never execute"
        );
        assert!(!dir.path().join("hooks/post-receive").exists());
        assert!(host.snapshot().recent_actions.is_empty());
        let broken = tempfile::tempdir().unwrap();
        std::fs::write(broken.path().join(".git"), "gitdir: missing\n").unwrap();
        assert!(crate::git::run_guest_repository(broken.path(), timeout).is_err());
    }

    #[test]
    fn home_hook_scope_probe() {
        if std::env::var_os(IGNORE_PROBE).is_none() {
            return;
        }
        // HOME belongs exclusively to the parent test's temporary fixture.
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let project = tempfile::Builder::new()
            .prefix("hook-home-project-")
            .tempdir_in(&home)
            .unwrap();
        let root = project.path();
        empty_repo(root);
        let (_dir, host) = fixture();
        {
            let mut inner = host.inner.lock().unwrap();
            inner.shared_projects[0].path = root.canonicalize().unwrap();
            inner.guests[0].guest.rights.r#type = false;
        }
        std::fs::create_dir(root.join("hooks space")).unwrap();
        std::fs::create_dir_all(root.join("~/hooks space")).unwrap();
        for path in ["hooks space/run", "~/hooks space/run"] {
            std::fs::write(root.join(path), "synthetic").unwrap();
        }
        let config = root.join(".git/config");
        std::fs::write(
            &config,
            format!("[core]\n repositoryformatversion = 0\n bare = false\n hooksPath = ~/{}/hooks space\n", root.file_name().unwrap().to_str().unwrap()),
        )
        .unwrap();
        let read = |path| {
            dispatch(
                &host.inner,
                "guest",
                &json!({"op":"files.read","path":path}),
            )
        };
        let actual = read("hooks space/run");
        assert_eq!(actual["result"]["readOnly"], true, "{actual}");
        assert_eq!(read("~/hooks space/run")["result"]["readOnly"], false);
        let req = json!({"op":"files.write","path":"hooks space/run","expectHash":file_version::hash(b"synthetic"),"data":B64.encode(b"changed")});
        assert!(
            super::super::guest_file_mutations::dispatch(&host.inner, "guest", &req)
                .get("error")
                .is_some()
        );
        assert_eq!(
            std::fs::read(root.join("hooks space/run")).unwrap(),
            b"synthetic"
        );
        host.inner.lock().unwrap().guests[0].guest.rights.r#type = true;
        assert_eq!(
            super::super::guest_file_mutations::dispatch(&host.inner, "guest", &req)["result"]["ok"],
            true
        );
        assert!(host.snapshot().recent_actions.last().unwrap().flagged);
        // Invalid config must not silently make the protected file writable.
        std::fs::write(&config, "[invalid").unwrap();
        assert!(read("hooks space/run").get("error").is_some());
        // A broken .git indirection is not treated as an ordinary non-repository.
        let git_dir = dunce::canonicalize(root.join(".git")).unwrap();
        assert!(git_dir.starts_with(dunce::canonicalize(root).unwrap()));
        std::fs::remove_dir_all(&git_dir).unwrap();
        std::fs::write(root.join(".git"), "gitdir: missing-git-dir\n").unwrap();
        assert!(read("hooks space/run").get("error").is_some());
    }

    #[test]
    fn hook_scope_probe() {
        if std::env::var_os(IGNORE_PROBE).is_none() {
            return;
        }
        let (dir, host) = fixture();
        empty_repo(dir.path());
        let configure = |value: &str| {
            std::fs::write(
                dir.path().join(".git/config"),
                format!(
                    "[core]\n repositoryformatversion = 0\n bare = false\n hooksPath = {value}\n"
                ),
            )
            .unwrap()
        };
        std::fs::create_dir_all(dir.path().join("container/hooks")).unwrap();
        std::fs::create_dir(dir.path().join("container/hooks-other")).unwrap();
        for path in [
            "container/hooks/launch.sh",
            "container/hooks-other/plain",
            "post-commit",
            "ordinary.txt",
        ] {
            std::fs::write(dir.path().join(path), "original").unwrap();
        }
        configure("container/hooks");
        let read = |path: &str| {
            dispatch(
                &host.inner,
                "guest",
                &json!({"op":"files.read","path":path}),
            )
        };
        assert_eq!(
            read("container/hooks/launch.sh")["result"]["readOnly"],
            true
        );
        assert_eq!(read("post-commit")["result"]["readOnly"], false);
        assert_eq!(
            read("container/hooks-other/plain")["result"]["readOnly"],
            false
        );
        for request in [
            json!({"op":"files.write","path":"container/hooks/launch.sh","data":B64.encode(b"bad")}),
            json!({"op":"files.create","path":"container/hooks/new"}),
            json!({"op":"files.rename","path":"ordinary.txt","to":"container/hooks/new"}),
            json!({"op":"files.delete","path":"container"}),
        ] {
            assert!(
                super::super::guest_file_mutations::dispatch(&host.inner, "guest", &request)
                    .get("error")
                    .is_some(),
                "{request}"
            );
        }
        assert_eq!(
            std::fs::read(dir.path().join("container/hooks/launch.sh")).unwrap(),
            b"original"
        );
        configure(".");
        assert_eq!(read("post-commit")["result"]["readOnly"], true);
        assert_eq!(read("ordinary.txt")["result"]["readOnly"], false);
        configure("container/hooks");
        let changed =
            super::super::guest_file_mutations::check_hook_change_during_save(&host.inner, || {
                configure(".")
            });
        assert!(changed.unwrap_err().contains("hooks scope changed"));
        assert_eq!(
            std::fs::read(dir.path().join("post-commit")).unwrap(),
            b"original"
        );
        configure("container/hooks");
        host.inner.lock().unwrap().guests[0].guest.rights.r#type = true;
        let response = super::super::guest_file_mutations::dispatch(
            &host.inner,
            "guest",
            &json!({"op":"files.write","path":"container/hooks/launch.sh","expectHash":file_version::hash(&std::fs::read(dir.path().join("container/hooks/launch.sh")).unwrap()),"data":B64.encode(b"approved")}),
        );
        assert_eq!(response["result"]["ok"], true, "{response}");
        assert!(host.snapshot().recent_actions.last().unwrap().flagged);
    }
    #[test]
    fn real_hook_scope_gates_reads_mutations_and_root_hook_names() {
        run_isolated_probe("hook_scope_probe");
    }

    #[test]
    fn file_reads_release_authority_and_hide_results_after_revocation() {
        let (_dir, host) = fixture();
        let result = run_authorized(&host.inner, "guest", &json!({"projectId":1}), |_| {
            let mut guard = host
                .inner
                .try_lock()
                .expect("Read must not hold authority mutex");
            guard.guests[0].guest.projects.clear();
            Ok(json!({"data":"must not be disclosed"}))
        });
        assert!(result.is_err());
    }
    #[test]
    fn revoked_reads_do_not_release_filesystem_errors_either() {
        let (_dir, host) = fixture();
        let result = run_authorized(&host.inner, "guest", &json!({"projectId":1}), |_| {
            host.inner.lock().unwrap().guests[0].guest.projects.clear();
            Err("old filesystem details".into())
        });
        assert!(!result.unwrap_err().contains("filesystem details"));
    }
    #[test]
    fn file_reads_revalidate_root_masking_rights_and_host_lifetime() {
        for change in 0..4 {
            let (_dir, host) = fixture();
            let result = run_authorized(&host.inner, "guest", &json!({"projectId":1}), |_| {
                let mut guard = host.inner.lock().unwrap();
                match change {
                    0 => guard.shared_projects[0].path.push("changed"),
                    1 => guard.settings.hide_ignored = !guard.settings.hide_ignored,
                    2 => guard.guests[0].guest.rights.edit = false,
                    _ => guard.stopped.store(true, Ordering::SeqCst),
                }
                Ok(json!({"data":"not released"}))
            });
            assert!(result.is_err(), "change {change}");
        }
    }
    #[test]
    fn stopped_or_pending_guests_never_start_file_io() {
        let (_dir, host) = fixture();
        host.inner.lock().unwrap().guests[0].decision = GuestDecision::Pending;
        assert!(
            run_authorized(&host.inner, "guest", &json!({"projectId":1}), |_| panic!(
                "pending guest read"
            ))
            .is_err()
        );
        let inner = host.inner.clone();
        drop(host);
        assert!(
            run_authorized(&inner, "guest", &json!({"projectId":1}), |_| panic!(
                "stopped host read"
            ))
            .is_err()
        );
    }
    #[test]
    fn secret_name_families_are_hidden_from_listing_and_direct_reads() {
        let (dir, host) = fixture();
        let names = [
            "private.p8",
            "putty.ppk",
            "java.jks",
            "android.keystore",
            "production.tfvars",
            "terraform.tfstate.backup",
            "id_ecdsa",
            "id_dsa",
            "id_rsa_backup",
            "id_ed25519_old",
            "service-account-prod.json",
            "SERVICE-ACCOUNT.JSON",
        ];
        for name in names {
            std::fs::write(dir.path().join(name), "synthetic secret").unwrap();
            let response = dispatch(
                &host.inner,
                "guest",
                &json!({"op":"files.read","projectId":1,"path":name}),
            );
            assert!(response.get("error").is_some(), "{name}: {response}");
        }
        std::fs::write(dir.path().join("service-account-guide.md"), "public").unwrap();
        let response = dispatch(
            &host.inner,
            "guest",
            &json!({"op":"files.list","projectId":1}),
        );
        let entries = response["result"]["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "{response}");
        assert_eq!(entries[0]["name"], "service-account-guide.md");
        for name in names {
            assert!(secret_name_path(&Path::new("nested").join(name)));
        }
    }

    #[test]
    fn extracted_file_reads_preserve_contents_protection_and_limits() {
        let (dir, host) = fixture();
        std::fs::write(dir.path().join("visible.txt"), "synthetic").unwrap();
        std::fs::write(dir.path().join(".env"), "private").unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "instructions").unwrap();
        host.inner.lock().unwrap().guests[0].guest.rights.r#type = false;
        let response = dispatch(
            &host.inner,
            "guest",
            &json!({"op":"files.read","projectId":1,"path":"visible.txt"}),
        );
        assert_eq!(response["result"]["data"], B64.encode(b"synthetic"));
        let response = dispatch(
            &host.inner,
            "guest",
            &json!({"op":"files.list","projectId":1}),
        );
        assert!(
            response["result"]["entries"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["name"] != ".env")
        );
        let response = dispatch(
            &host.inner,
            "guest",
            &json!({"op":"files.read","projectId":1,"path":"AGENTS.md"}),
        );
        assert_eq!(response["result"]["readOnly"], true);
        for path in [".env", "../outside.txt"] {
            let response = dispatch(
                &host.inner,
                "guest",
                &json!({"op":"files.read","projectId":1,"path":path}),
            );
            assert!(response.get("error").is_some());
        }
        std::fs::File::create(dir.path().join("large.txt"))
            .unwrap()
            .set_len((MAX_FILE + 1) as u64)
            .unwrap();
        let response = dispatch(
            &host.inner,
            "guest",
            &json!({"op":"files.read","projectId":1,"path":"large.txt"}),
        );
        assert!(response.get("error").is_some());
    }
}
