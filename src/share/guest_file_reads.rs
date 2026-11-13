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
    if current.project.path != grant.project.path
        || current.hide != grant.hide
        || current.rights != grant.rights
    {
        return Err("Shared file access changed; refresh the current project.".into());
    }
    result
        .map_err(|error| guest_text::task_title(&error, &grant.project.path.to_string_lossy(), 300))
}
fn read(grant: &Grant, req: &Value) -> Result<Value> {
    let rights = grant.rights;
    let hide_ignored = grant.hide;
    match req.get("op").and_then(Value::as_str).unwrap_or("") {
        "files.list" => {
            let proj = &grant.project;
            let relative = req.get("path").and_then(Value::as_str).unwrap_or("");
            let target = if relative.is_empty() {
                proj.path.clone()
            } else {
                share_path_checked(&proj.path, relative, hide_ignored)?
            };
            let entries = list_dir_entries(&proj.path, &target)?;
            let filtered: Vec<Value> = entries
                .into_iter()
                .filter(|e| {
                    let rel = e.path.strip_prefix(&proj.path).unwrap_or(&e.path);
                    !private_path(rel) && !(hide_ignored && secret_or_dot_path(rel))
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
            Ok(json!({"entries": filtered, "readOnly": !rights.edit}))
        }
        "files.read" => {
            let proj = &grant.project;
            let relative = req.get("path").and_then(Value::as_str).unwrap_or("");
            let target = share_path_checked(&proj.path, relative, hide_ignored)?;
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
                "data": B64.encode(bytes),
                "readOnly": !rights.edit || (is_agent_instruction_or_protected_file(Path::new(relative)) && !rights.r#type),
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
            assert!(secret_or_dot_path(&Path::new("nested").join(name)));
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
