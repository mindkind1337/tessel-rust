//! File mutations run outside the host authority lock and recheck before applying.
//! A filesystem operation already launched may finish after revocation; its response
//! is then withheld. Descriptor-relative access and version conflicts are separate work.
use super::*;
static ACTIVE_WRITES: AtomicUsize = AtomicUsize::new(0);
struct WriteSlot;
impl WriteSlot {
    fn acquire() -> Result<Self> {
        let mut active = ACTIVE_WRITES.load(Ordering::SeqCst);
        loop {
            if active >= 2 {
                return Err("Guest file changes are busy. Try again later.".into());
            }
            match ACTIVE_WRITES.compare_exchange_weak(
                active,
                active + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Ok(Self),
                Err(value) => active = value,
            }
        }
    }
}
impl Drop for WriteSlot {
    fn drop(&mut self) {
        ACTIVE_WRITES.fetch_sub(1, Ordering::SeqCst);
    }
}
struct Grant {
    ignored: Arc<ignored_paths::IgnoredPaths>,
    roots: Vec<PathBuf>,
    project: SharedProject,
    rights: ShareRights,
    hide: bool,
    name: String,
}
struct Action {
    en: String,
    fr: String,
    flagged: bool,
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
    if !slot.guest.rights.edit {
        return Err("Edit permission is required to change files.".into());
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
        name: guest_text::task_title(&slot.guest.name, "", 40),
    })
}
fn validate(inner: &HostInner, guest: &str, grant: &Grant) -> Result<()> {
    let current = authorize(inner, guest, Some(grant.project.id))?;
    if current.roots != grant.roots
        || current.project.path != grant.project.path
        || current.rights != grant.rights
        || current.hide != grant.hide
    {
        return Err("Shared file access changed; refresh the current project.".into());
    }
    Ok(())
}
fn run_authorized(
    inner: &Arc<Mutex<HostInner>>,
    guest: &str,
    req: &Value,
    apply: impl FnOnce(&Grant, &mut dyn FnMut() -> Result<()>) -> Result<Action>,
) -> Result<Value> {
    let grant = {
        let guard = inner.lock().unwrap_or_else(|e| e.into_inner());
        authorize(&guard, guest, req.get("projectId").and_then(Value::as_u64))?
    };
    let mut check = || {
        validate(
            &inner.lock().unwrap_or_else(|e| e.into_inner()),
            guest,
            &grant,
        )
    };
    let result = apply(&grant, &mut check);
    let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
    validate(&guard, guest, &grant)?;
    let action = result
        .map_err(|e| guest_text::task_title(&e, &grant.project.path.to_string_lossy(), 300))?;
    guard.record_action(
        &grant.name,
        grant.project.id,
        guest_text::task_title(&action.en, &grant.project.path.to_string_lossy(), 300),
        guest_text::task_title(&action.fr, &grant.project.path.to_string_lossy(), 300),
        action.flagged,
    );
    Ok(json!({"ok":true}))
}
#[cfg(windows)]
fn rename_new(from: &Path, to: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW},
        core::PCWSTR,
    };
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // Zero flags means the OS refuses an existing destination atomically.
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVE_FILE_FLAGS(0),
        )
    }
    .map_err(|_| "Cannot rename; destination may already exist or be in use".into())
}
#[cfg(not(windows))]
fn rename_new(_from: &Path, _to: &Path) -> Result<()> {
    Err("Atomic no-overwrite rename is currently supported on Windows only".into())
}

fn check_unlinked_entry(grant: &Grant, relative: &str) -> Result<()> {
    let mut path = grant.project.path.clone();
    for component in Path::new(relative).components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err("Invalid file entry.".into());
        }
        path.push(component);
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| "Cannot verify file entry")?;
        #[cfg(windows)]
        let linked = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let linked = metadata.file_type().is_symlink();
        if linked || metadata.file_type().is_symlink() {
            return Err("Linked entries cannot be moved or recycled by a guest.".into());
        }
    }
    Ok(())
}
// A directory operation affects its descendants too. Refuse an incomplete walk.
fn check_descendants(grant: &Grant, target: &Path, hooks: &[String]) -> Result<()> {
    let started = Instant::now();
    let mut pending = vec![target.to_path_buf()];
    let mut examined = 0usize;
    let mut paths = Vec::new();
    while let Some(path) = pending.pop() {
        examined += 1;
        if examined > 10_000 || started.elapsed() > Duration::from_secs(2) {
            return Err("Directory is too large to verify for a guest operation.".into());
        }
        let relative = path
            .strip_prefix(&grant.project.path)
            .map_err(|_| "Invalid project scope")?;
        if private_path(relative)
            || (grant.hide && secret_name_path(relative))
            || (!grant.rights.r#type && hook_paths::protects(relative, hooks))
        {
            return Err("Directory contains private or protected items.".into());
        }
        paths.push(relative.to_string_lossy().replace('\\', "/"));
        let metadata =
            std::fs::symlink_metadata(&path).map_err(|_| "Cannot verify directory contents")?;
        #[cfg(windows)]
        let linked = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let linked = metadata.file_type().is_symlink();
        if linked || metadata.file_type().is_symlink() {
            return Err("Linked items cannot be changed by a guest directory operation.".into());
        }
        if metadata.is_dir() {
            for entry in std::fs::read_dir(&path).map_err(|_| "Cannot verify directory contents")? {
                if examined + pending.len() >= 10_000 || started.elapsed() > Duration::from_secs(2)
                {
                    return Err("Directory is too large to verify for a guest operation.".into());
                }
                pending.push(entry.map_err(|_| "Cannot verify directory entry")?.path());
            }
        }
    }
    check_ignored(grant, &paths)?;
    Ok(())
}
fn check_ignored(grant: &Grant, paths: &[String]) -> Result<()> {
    if grant.hide
        && grant
            .ignored
            .hidden(&grant.project.path, &grant.roots, paths)?
            .into_iter()
            .any(|v| v)
    {
        return Err("File unavailable".into());
    }
    Ok(())
}
fn check_request_paths(grant: &Grant, req: &Value, hooks: &[String]) -> Result<bool> {
    if hook_paths::discover(&grant.project.path, &grant.roots)? != hooks {
        return Err("Git hooks scope changed; refresh before changing files.".into());
    }
    let mut flagged = false;
    let mut privacy_paths = Vec::new();
    for key in ["path", "to"] {
        if let Some(relative) = req.get(key).and_then(Value::as_str) {
            let resolved = if grant.project.path.join(relative).exists() {
                share_path_checked(&grant.project.path, relative, grant.hide)?
            } else {
                share_new_path_checked(&grant.project.path, relative, grant.hide)?
            };
            let real_rel = resolved
                .strip_prefix(&grant.project.path)
                .map_err(|_| "Invalid project scope")?;
            flagged |= hook_paths::protects(Path::new(relative), hooks)
                || hook_paths::protects(real_rel, hooks);
            if flagged && !grant.rights.r#type {
                return Err("Agent instructions require Type permission.".into());
            }
            privacy_paths.push(relative.replace('\\', "/"));
            privacy_paths.push(
                resolved
                    .strip_prefix(&grant.project.path)
                    .map_err(|_| "Invalid project scope")?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    check_ignored(grant, &privacy_paths)?;
    Ok(flagged)
}
fn apply(grant: &Grant, req: &Value, revalidate: &mut dyn FnMut() -> Result<()>) -> Result<Action> {
    apply_with_recycler(grant, req, revalidate, &mut |path| {
        crate::native_recycle::send_path_to_recycle_bin(path)
    })
}
fn apply_with_recycler(
    grant: &Grant,
    req: &Value,
    revalidate: &mut dyn FnMut() -> Result<()>,
    recycle: &mut dyn FnMut(&Path) -> Result<()>,
) -> Result<Action> {
    let hooks = hook_paths::discover(&grant.project.path, &grant.roots)?;
    let rights = grant.rights;
    let hide_ignored = grant.hide;
    let guest_name = &grant.name;
    match req.get("op").and_then(Value::as_str).unwrap_or("") {
        "files.write" => {
            if !rights.edit {
                return Err("Edit permission is required to save files.".into());
            }
            let proj = &grant.project;
            let relative = req.get("path").and_then(Value::as_str).unwrap_or("");
            let flagged = is_agent_instruction_or_protected_file(Path::new(relative));
            if flagged && !rights.r#type {
                return Err(
                    "Agent instructions and .gitignore require Type permission to edit.".into(),
                );
            }
            let target = if proj.path.join(relative).exists() {
                share_path_checked(&proj.path, relative, hide_ignored)?
            } else {
                share_new_path_checked(&proj.path, relative, hide_ignored)?
            };
            let data_b64 = req.get("data").and_then(Value::as_str).unwrap_or("");
            let bytes = B64.decode(data_b64).map_err(|_| "Invalid file payload")?;
            if bytes.len() > 2 * 1024 * 1024 {
                return Err("Live Share file write limit is 2 MiB.".into());
            }
            let parent = target.parent().ok_or("Missing parent folder")?;
            let flagged = check_request_paths(grant, req, &hooks)? || flagged;
            revalidate()?;
            let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
            tmp.write_all(&bytes).map_err(|e| e.to_string())?;
            tmp.as_file().sync_all().map_err(|e| e.to_string())?;
            let flagged = check_request_paths(grant, req, &hooks)? || flagged;
            revalidate()?;
            tmp.persist(&target).map_err(|e| e.to_string())?;
            Ok(Action {
                en: format!("{guest_name} saved {relative}"),
                fr: format!("{guest_name} a enregistré {relative}"),
                flagged,
            })
        }
        "files.create" => {
            if !rights.edit {
                return Err("Edit permission is required to create files.".into());
            }
            let proj = &grant.project;
            let relative = req.get("path").and_then(Value::as_str).unwrap_or("");
            let is_dir = req
                .get("directory")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let flagged = is_agent_instruction_or_protected_file(Path::new(relative));
            if flagged && !rights.r#type {
                return Err("Agent instructions require Type permission.".into());
            }
            let target = share_new_path_checked(&proj.path, relative, hide_ignored)?;
            if target.exists() {
                return Err("Path already exists.".into());
            }
            let flagged = check_request_paths(grant, req, &hooks)? || flagged;
            revalidate()?;
            if is_dir {
                std::fs::create_dir(&target).map_err(|e| e.to_string())?;
            } else {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .map_err(|e| e.to_string())?;
            }
            Ok(Action {
                en: format!("{guest_name} created {relative}"),
                fr: format!("{guest_name} a créé {relative}"),
                flagged,
            })
        }
        "files.rename" => {
            if !rights.edit {
                return Err("Edit permission is required to rename files.".into());
            }
            let proj = &grant.project;
            let from_rel = req.get("path").and_then(Value::as_str).unwrap_or("");
            let to_rel = req.get("to").and_then(Value::as_str).unwrap_or("");
            let flagged = is_agent_instruction_or_protected_file(Path::new(from_rel))
                || is_agent_instruction_or_protected_file(Path::new(to_rel));
            if flagged && !rights.r#type {
                return Err("Agent instructions require Type permission.".into());
            }
            let src = share_path_checked(&proj.path, from_rel, hide_ignored)?;
            check_unlinked_entry(grant, from_rel)?;
            check_descendants(grant, &src, &hooks)?;
            let dst = share_new_path_checked(&proj.path, to_rel, hide_ignored)?;
            if dst.exists() {
                return Err("Destination already exists.".into());
            }
            let flagged = check_request_paths(grant, req, &hooks)? || flagged;
            revalidate()?;
            rename_new(&src, &dst)?;
            Ok(Action {
                en: format!("{guest_name} renamed {from_rel} to {to_rel}"),
                fr: format!("{guest_name} a renommé {from_rel} en {to_rel}"),
                flagged,
            })
        }
        "files.delete" => {
            if !rights.edit {
                return Err("Edit permission is required to delete files.".into());
            }
            let proj = &grant.project;
            let relative = req.get("path").and_then(Value::as_str).unwrap_or("");
            let flagged = is_agent_instruction_or_protected_file(Path::new(relative));
            if flagged && !rights.r#type {
                return Err("Agent instructions require Type permission.".into());
            }
            let target = share_path_checked(&proj.path, relative, hide_ignored)?;
            check_unlinked_entry(grant, relative)?;
            check_descendants(grant, &target, &hooks)?;
            let flagged = check_request_paths(grant, req, &hooks)? || flagged;
            revalidate()?;
            recycle(&target)?;
            Ok(Action {
                en: format!("{guest_name} moved {relative} to the Recycle Bin"),
                fr: format!("{guest_name} a mis {relative} à la corbeille"),
                flagged,
            })
        }
        _ => Err("Unsupported guest file change.".into()),
    }
}
pub(super) fn dispatch(inner: &Arc<Mutex<HostInner>>, guest: &str, req: &Value) -> Value {
    let result = run_authorized(inner, guest, req, |grant, check| {
        let _slot = WriteSlot::acquire()?;
        apply(grant, req, check)
    });
    match result {
        Ok(value) => json!({"result":value}),
        Err(error) => json!({"error":error}),
    }
}

#[cfg(test)]
pub(super) fn check_hook_change_during_save(
    inner: &Arc<Mutex<HostInner>>,
    mut change: impl FnMut(),
) -> Result<Value> {
    let req =
        json!({"op":"files.write","path":"post-commit","data":B64.encode(b"must not persist")});
    let mut changed = false;
    run_authorized(inner, "guest", &req, |grant, check| {
        apply(grant, &req, &mut || {
            check()?;
            if !changed {
                change();
                changed = true;
            }
            Ok(())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Host) {
        let dir = tempfile::Builder::new()
            .prefix("file-write-authority-")
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

    #[test]
    fn empty_mutation_paths_never_target_the_shared_root_or_its_parent() {
        let (dir, host) = fixture();
        let grant = authorize(&host.inner.lock().unwrap(), "guest", Some(1)).unwrap();
        for path in ["", "   "] {
            for op in [
                "files.write",
                "files.create",
                "files.rename",
                "files.delete",
            ] {
                let req = json!({"op":op,"path":path,"to":"child","data":B64.encode(b"payload")});
                let result = apply(&grant, &req, &mut || {
                    panic!("invalid root path reached mutation boundary")
                });
                assert!(result.is_err(), "{op} {path:?}");
                assert!(dir.path().is_dir());
                assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
            }
        }
    }
    #[test]
    fn mutations_run_without_authority_mutex_and_recheck_before_commit() {
        let (_dir, host) = fixture();
        let result = run_authorized(&host.inner, "guest", &json!({"projectId":1}), |_, check| {
            host.inner
                .try_lock()
                .expect("No authority lock during file I/O")
                .guests[0]
                .guest
                .rights
                .edit = false;
            check()?;
            panic!("revoked mutation must not commit")
        });
        assert!(result.is_err());
        assert!(host.snapshot().recent_actions.is_empty());
    }
    #[test]
    fn every_mutation_checks_current_authority_before_touching_destination() {
        let (dir, host) = fixture();
        std::fs::write(dir.path().join("existing.txt"), "original").unwrap();
        let grant = authorize(&host.inner.lock().unwrap(), "guest", Some(1)).unwrap();
        for req in [
            json!({"op":"files.write","path":"existing.txt","data":B64.encode(b"replacement")}),
            json!({"op":"files.create","path":"new.txt"}),
            json!({"op":"files.rename","path":"existing.txt","to":"new.txt"}),
            json!({"op":"files.delete","path":"existing.txt"}),
        ] {
            let mut called = false;
            let result = apply(&grant, &req, &mut || {
                called = true;
                Err("revoked".into())
            });
            assert!(called);
            assert!(result.is_err());
            assert_eq!(
                std::fs::read_to_string(dir.path().join("existing.txt")).unwrap(),
                "original"
            );
            assert!(!dir.path().join("new.txt").exists());
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }
    #[test]
    fn completed_or_failed_mutations_withhold_response_after_scope_change() {
        for failure in [false, true] {
            let (_dir, host) = fixture();
            let result = run_authorized(&host.inner, "guest", &json!({"projectId":1}), |_, _| {
                host.inner.lock().unwrap().guests[0].guest.projects.clear();
                if failure {
                    Err("old filesystem details".into())
                } else {
                    Ok(Action {
                        en: "done".into(),
                        fr: "fini".into(),
                        flagged: false,
                    })
                }
            });
            assert!(!result.unwrap_err().contains("filesystem details"));
            assert!(host.snapshot().recent_actions.is_empty());
        }
    }
    #[cfg(windows)]
    #[test]
    fn real_file_mutations_preserve_atomic_save_create_rename_and_delete() {
        let (dir, host) = fixture();
        for req in [
            json!({"op":"files.create","path":"one.txt"}),
            json!({"op":"files.write","path":"one.txt","data":B64.encode(b"saved")}),
            json!({"op":"files.rename","path":"one.txt","to":"two.txt"}),
        ] {
            assert_eq!(dispatch(&host.inner, "guest", &req)["result"]["ok"], true);
        }
        assert!(!dir.path().join("one.txt").exists());
        assert_eq!(std::fs::read(dir.path().join("two.txt")).unwrap(), b"saved");
        assert_eq!(
            dispatch(
                &host.inner,
                "guest",
                &json!({"op":"files.delete","path":"two.txt"})
            )["result"]["ok"],
            true
        );
        assert!(!dir.path().join("two.txt").exists());
        let actions = host
            .snapshot()
            .recent_actions
            .into_iter()
            .map(|a| a.summary_fr)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(actions.contains("corbeille"));
        assert!(!actions.contains("supprimé"));
    }
    #[test]
    fn rename_refuses_a_destination_created_after_validation() {
        let (dir, host) = fixture();
        std::fs::write(dir.path().join("source.txt"), "source").unwrap();
        let grant = authorize(&host.inner.lock().unwrap(), "guest", Some(1)).unwrap();
        let result = apply(
            &grant,
            &json!({"op":"files.rename","path":"source.txt","to":"destination.txt"}),
            &mut || {
                std::fs::write(dir.path().join("destination.txt"), "keep").unwrap();
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("destination.txt")).unwrap(),
            "keep"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("source.txt")).unwrap(),
            "source"
        );
    }
    #[cfg(windows)]
    #[test]
    fn a_junction_entry_never_recycles_its_target() {
        use std::os::windows::process::CommandExt;
        let (dir, host) = fixture();
        let target = dir.path().join("target");
        let link = dir.path().join("linked");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("keep.txt"), "safe").unwrap();
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "synthetic junction creation failed"
        );
        let grant = authorize(&host.inner.lock().unwrap(), "guest", Some(1)).unwrap();
        let result = apply_with_recycler(
            &grant,
            &json!({"op":"files.delete","path":"linked"}),
            &mut || Ok(()),
            &mut |_| panic!("junction target must not reach recycler"),
        );
        std::fs::remove_dir(&link).unwrap();
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(target.join("keep.txt")).unwrap(),
            "safe"
        );
    }
    #[cfg(windows)]
    #[test]
    fn a_junction_parent_cannot_hide_a_secret_destination() {
        use std::os::windows::process::CommandExt;
        let (dir, host) = fixture();
        let target = dir.path().join(".ssh");
        let link = dir.path().join("alias");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("existing.txt"), "private").unwrap();
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(output.status.success());
        let root = host.inner.lock().unwrap().shared_projects[0].path.clone();
        let new_result = share_new_path_checked(&root, "alias/new.txt", true);
        let existing_result = share_new_path_checked(&root, "alias/existing.txt", true);
        std::fs::remove_dir(&link).unwrap();
        assert!(new_result.is_err());
        assert!(existing_result.is_err());
        assert!(!target.join("new.txt").exists());
        assert_eq!(
            std::fs::read(target.join("existing.txt")).unwrap(),
            b"private"
        );
    }
    #[cfg(windows)]
    #[test]
    fn protected_parent_alias_requires_type_and_keeps_audit_flag() {
        use std::os::windows::process::CommandExt;
        let (dir, host) = fixture();
        let target = dir.path().join(".github");
        let link = dir.path().join("alias");
        std::fs::create_dir_all(target.join("workflows")).unwrap();
        std::fs::write(target.join("workflows/existing.yml"), "owner config").unwrap();
        std::fs::write(dir.path().join("ordinary.txt"), "safe").unwrap();
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(output.status.success());
        host.inner.lock().unwrap().guests[0].guest.rights.r#type = false;
        for request in [
            json!({"op":"files.create","path":"alias/workflows/new.yml"}),
            json!({"op":"files.write","path":"alias/workflows/existing.yml","data":B64.encode(b"changed")}),
            json!({"op":"files.rename","path":"ordinary.txt","to":"alias/workflows/renamed.yml"}),
        ] {
            assert!(
                dispatch(&host.inner, "guest", &request)
                    .get("error")
                    .is_some(),
                "{request}"
            );
        }
        let read = super::super::guest_file_reads::dispatch(
            &host.inner,
            "guest",
            &json!({"op":"files.read","path":"alias/workflows/existing.yml"}),
        );
        assert_eq!(read["result"]["readOnly"], true, "{read}");
        assert_eq!(
            std::fs::read(target.join("workflows/existing.yml")).unwrap(),
            b"owner config"
        );
        assert!(dir.path().join("ordinary.txt").exists());
        assert!(!target.join("workflows/new.yml").exists());
        host.inner.lock().unwrap().guests[0].guest.rights.r#type = true;
        let written = dispatch(
            &host.inner,
            "guest",
            &json!({"op":"files.write","path":"alias/workflows/existing.yml","data":B64.encode(b"approved")}),
        );
        std::fs::remove_dir(&link).unwrap();
        assert_eq!(written["result"]["ok"], true, "{written}");
        assert!(host.snapshot().recent_actions.last().unwrap().flagged);
        assert_eq!(
            std::fs::read(target.join("workflows/existing.yml")).unwrap(),
            b"approved"
        );
    }
    #[test]
    fn ci_configuration_requires_type_even_when_dot_files_are_visible() {
        let (dir, host) = fixture();
        {
            let mut guard = host.inner.lock().unwrap();
            guard.settings.hide_ignored = false;
            guard.guests[0].guest.rights.r#type = false;
        }
        std::fs::create_dir_all(dir.path().join(".github/workflows")).unwrap();
        for path in [
            ".github/workflows/build.yml",
            ".codex/settings.json",
            ".agents/skills/example/SKILL.md",
            "opencode.json",
            ".clinerules",
            ".forgejo/workflows/check.yml",
            "Jenkinsfile.production",
            ".gitlab-ci.yml",
            "Jenkinsfile",
            "azure-pipelines.yml",
            "bitbucket-pipelines.yml",
            ".drone.yml",
        ] {
            std::fs::create_dir_all(dir.path().join(path).parent().unwrap()).unwrap();
            assert!(
                dispatch(
                    &host.inner,
                    "guest",
                    &json!({"op":"files.create","path":path})
                )
                .get("error")
                .is_some()
            );
            std::fs::write(dir.path().join(path), "owner config").unwrap();
            assert!(
                dispatch(
                    &host.inner,
                    "guest",
                    &json!({"op":"files.write","path":path,"data":B64.encode(b"changed")})
                )
                .get("error")
                .is_some()
            );
            assert_eq!(
                std::fs::read_to_string(dir.path().join(path)).unwrap(),
                "owner config"
            );
        }
        let grant = authorize(&host.inner.lock().unwrap(), "guest", Some(1)).unwrap();
        assert!(check_descendants(&grant, &dir.path().join(".github"), &[]).is_err());
    }
    #[test]
    fn recycler_failure_keeps_bytes_and_does_not_record_success() {
        let (dir, host) = fixture();
        let target = dir.path().join("keep.txt");
        std::fs::write(&target, "recoverable").unwrap();
        let req = json!({"op":"files.delete","path":"keep.txt"});
        let result = run_authorized(&host.inner, "guest", &req, |grant, check| {
            apply_with_recycler(grant, &req, check, &mut |_| {
                Err("Recycle operation cancelled".into())
            })
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "recoverable");
        assert!(host.snapshot().recent_actions.is_empty());
    }
    #[test]
    fn directory_recycling_checks_descendants_and_routes_the_whole_folder() {
        let (dir, host) = fixture();
        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("ordinary.txt"), "payload").unwrap();
        host.inner.lock().unwrap().guests[0].guest.rights.r#type = false;
        let grant = authorize(&host.inner.lock().unwrap(), "guest", Some(1)).unwrap();
        let req = json!({"op":"files.delete","path":"folder"});
        let mut requested = None;
        apply_with_recycler(&grant, &req, &mut || Ok(()), &mut |path| {
            requested = Some(path.to_path_buf());
            Ok(())
        })
        .unwrap();
        assert_eq!(requested.unwrap(), folder.canonicalize().unwrap());
        for name in ["AGENTS.md", ".env", "vault.dat"] {
            std::fs::write(folder.join(name), "keep").unwrap();
            let result = apply_with_recycler(&grant, &req, &mut || Ok(()), &mut |_| {
                panic!("private descendant reached recycling")
            });
            assert!(result.is_err());
            std::fs::remove_file(folder.join(name)).unwrap();
        }
    }
    #[cfg(windows)]
    #[test]
    fn ordinary_directory_rename_needs_edit_but_not_type() {
        let (dir, host) = fixture();
        std::fs::create_dir(dir.path().join("ordinary")).unwrap();
        std::fs::write(dir.path().join("ordinary/note.txt"), "safe").unwrap();
        host.inner.lock().unwrap().guests[0].guest.rights.r#type = false;
        let response = dispatch(
            &host.inner,
            "guest",
            &json!({"op":"files.rename","path":"ordinary","to":"renamed"}),
        );
        assert_eq!(response["result"]["ok"], true);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("renamed/note.txt")).unwrap(),
            "safe"
        );
    }
    #[test]
    fn protected_files_and_directory_moves_require_type_permission() {
        let (dir, host) = fixture();
        std::fs::create_dir(dir.path().join("folder")).unwrap();
        std::fs::write(dir.path().join("folder/AGENTS.md"), "instructions").unwrap();
        host.inner.lock().unwrap().guests[0].guest.rights.r#type = false;
        for req in [
            json!({"op":"files.rename","path":"folder","to":"renamed"}),
            json!({"op":"files.write","path":"folder/AGENTS.md","data":B64.encode(b"changed")}),
            json!({"op":"files.create","path":"AGENTS.md"}),
            json!({"op":"files.delete","path":"folder/AGENTS.md"}),
            json!({"op":"files.create","path":".git"}),
        ] {
            assert!(dispatch(&host.inner, "guest", &req).get("error").is_some());
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("folder/AGENTS.md")).unwrap(),
            "instructions"
        );
        assert!(!dir.path().join("renamed").exists());
    }
}
