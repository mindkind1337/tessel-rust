//! Keep slow Git I/O out of the authority mutex and revalidate at its boundaries.
use super::*;
static ACTIVE_GIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct GitSlot;
impl GitSlot {
    fn acquire() -> Result<Self> {
        let mut active = ACTIVE_GIT.load(Ordering::SeqCst);
        loop {
            if active >= 2 {
                return Err("Guest Git is busy. Try again after the current request.".into());
            }
            match ACTIVE_GIT.compare_exchange_weak(
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
impl Drop for GitSlot {
    fn drop(&mut self) {
        ACTIVE_GIT.fetch_sub(1, Ordering::SeqCst);
    }
}
struct Grant {
    history: Arc<ignored_paths::IgnoredPaths>,
    project: SharedProject,
    hide: bool,
    git: bool,
    name: String,
    roots: Vec<PathBuf>,
}
fn authorize(guard: &HostInner, guest: &str, project: Option<u64>, mutate: bool) -> Result<Grant> {
    if guard.stopped.load(Ordering::SeqCst) {
        return Err("Sharing has stopped.".into());
    }
    let index = guard
        .guests
        .iter()
        .position(|slot| slot.guest.id == guest)
        .ok_or("Guest session has ended.")?;
    let slot = &guard.guests[index];
    if !matches!(slot.decision, GuestDecision::Approved)
        || slot.guest.state != GuestConnectionState::Joined
    {
        return Err("Guest session is not active.".into());
    }
    if mutate && !slot.guest.rights.git {
        return Err("Git permission is required.".into());
    }
    Ok(Grant {
        history: guard.ignored_paths.clone(),
        project: guard.find_shared_project_for_guest(index, project)?,
        hide: guard.settings.hide_ignored,
        git: slot.guest.rights.git,
        name: guest_text::task_title(&slot.guest.name, "", 40),
        roots: guard
            .shared_projects
            .iter()
            .map(|p| p.path.clone())
            .collect(),
    })
}
fn run_authorized(
    inner: &Arc<Mutex<HostInner>>,
    guest: &str,
    req: &Value,
    run: impl FnOnce(&Grant, &mut dyn FnMut() -> Result<()>) -> Result<Value>,
) -> Result<Value> {
    let op = req.get("op").and_then(Value::as_str).unwrap_or("");
    let mutate = matches!(
        op,
        "scm.stage" | "scm.unstage" | "scm.discard" | "scm.commit"
    );
    let grant = {
        let guard = inner.lock().unwrap_or_else(|e| e.into_inner());
        authorize(
            &guard,
            guest,
            req.get("projectId").and_then(Value::as_u64),
            mutate,
        )?
    };
    let mut revalidate = || {
        let guard = inner.lock().unwrap_or_else(|e| e.into_inner());
        let current = authorize(&guard, guest, Some(grant.project.id), mutate)?;
        if current.project.path != grant.project.path
            || current.hide != grant.hide
            || current.roots != grant.roots
        {
            return Err("Shared project scope changed; retry from the current project.".into());
        }
        Ok(())
    };
    revalidate()?;
    let value = run(&grant, &mut revalidate)?;
    // Do not disclose a result after project access has been removed. An already
    // launched mutation may have completed; the guest retains an uncertain draft.
    revalidate()?;
    if mutate {
        let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
        let current = authorize(&guard, guest, Some(grant.project.id), true)?;
        if current.project.path != grant.project.path
            || current.hide != grant.hide
            || current.roots != grant.roots
        {
            return Err("Shared project scope changed.".into());
        }
        let action = match op {
            "scm.stage" => "stage",
            "scm.unstage" => "unstage",
            "scm.discard" => "discard",
            _ => "commit",
        };
        guard.record_action(
            &grant.name,
            grant.project.id,
            format!("{} completed Git {action}", grant.name),
            format!("{} a terminé l’action Git {action}", grant.name),
            false,
        );
    }
    Ok(value)
}
pub(super) fn private_hooks() -> Result<(tempfile::TempDir, PathBuf)> {
    let owner = tempfile::Builder::new()
        .prefix("tessel-empty-hooks-")
        .tempdir()
        .map_err(|_| "Cannot prepare safe Git hooks directory.")?;
    // Private-path checks still block guest writes when the system temp folder itself is shared.
    let path = owner.path().join(".tessel");
    std::fs::create_dir(&path).map_err(|_| "Cannot prepare private Git hooks directory.")?;
    Ok((owner, path))
}
pub(super) fn dispatch(inner: &Arc<Mutex<HostInner>>, guest: &str, req: &Value) -> Value {
    let result = run_authorized(inner, guest, req, |grant, revalidate| {
        let _slot = GitSlot::acquire()?;
        let (_owner, hooks) = private_hooks()?;
        guest_git_host::dispatch(
            &grant.project.path,
            &grant.roots,
            &grant.history,
            grant.git,
            grant.hide,
            req.get("op").and_then(Value::as_str).unwrap_or(""),
            req,
            &hooks,
            revalidate,
        )
    });
    match result {
        Ok(value) => json!({"result":value}),
        Err(error) => json!({"error":error}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Host) {
        let dir = tempfile::Builder::new()
            .prefix("git-authority-")
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
    fn guest_git_io_releases_authority_and_revocation_blocks_mutation() {
        let (_dir, host) = fixture();
        let result = run_authorized(
            &host.inner,
            "guest",
            &json!({"op":"scm.commit","projectId":1}),
            |_, check| {
                let mut guard = host
                    .inner
                    .try_lock()
                    .expect("Git I/O must not hold authority mutex");
                guard.guests[0].guest.rights.git = false;
                drop(guard);
                check()?;
                panic!("revoked mutation must not execute")
            },
        );
        assert!(result.is_err());
        assert!(host.snapshot().recent_actions.is_empty());
    }
    #[test]
    fn guest_git_does_not_disclose_reads_after_scope_changes() {
        let (_dir, host) = fixture();
        let result = run_authorized(
            &host.inner,
            "guest",
            &json!({"op":"scm.status","projectId":1}),
            |_, _| {
                host.inner.lock().unwrap().guests[0].guest.projects.clear();
                Ok(json!({"status":"hidden"}))
            },
        );
        assert!(result.is_err());
    }
    #[test]
    fn guest_git_revalidates_root_and_masking_before_execution() {
        for root in [false, true] {
            let (_dir, host) = fixture();
            let result = run_authorized(
                &host.inner,
                "guest",
                &json!({"op":"scm.stage","projectId":1}),
                |_, check| {
                    let mut guard = host.inner.lock().unwrap();
                    if root {
                        guard.shared_projects[0].path.push("replacement");
                    } else {
                        guard.settings.hide_ignored = !guard.settings.hide_ignored;
                    }
                    drop(guard);
                    check()?;
                    panic!("changed scope must not execute")
                },
            );
            assert!(result.is_err());
        }
    }
    #[test]
    fn guest_git_host_drop_invalidates_captured_authority() {
        let (_dir, host) = fixture();
        let inner = host.inner.clone();
        drop(host);
        assert!(
            run_authorized(
                &inner,
                "guest",
                &json!({"op":"scm.commit","projectId":1}),
                |_, _| panic!("stopped host must not execute")
            )
            .is_err()
        );
    }
    #[test]
    fn guest_git_hooks_stay_private_even_when_temp_parent_is_shared() {
        let (owner, hooks) = private_hooks().unwrap();
        assert!(std::fs::read_dir(&hooks).unwrap().next().is_none());
        assert!(share_new_path_checked(owner.path(), ".tessel/post-commit", false).is_err());
    }
}
