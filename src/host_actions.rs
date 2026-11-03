//! Host metadata mutations only. Never reads credentials or opens a connection.
use crate::workspace::{Host, Result, Store, Workspace, validate_host};

pub const MAX_HOSTS: usize = 1000;

pub fn persist(
    state: &mut Workspace,
    store: &Store,
    change: impl FnOnce(&mut Vec<Host>) -> Result<bool>,
) -> Result<bool> {
    let mut next = state.clone();
    let changed = change(&mut next.hosts)?;
    if changed {
        // Persist before publishing UI state; failed writes leave the original intact.
        store.save(&next)?;
        state.hosts = next.hosts;
    }
    Ok(changed)
}

fn validate(host: &Host) -> Result<()> {
    validate_host(&host.alias)?;
    if host.folder.len() > 8192
        || host.folder.chars().any(char::is_control)
        || host.folder.split(['/', '\\']).any(|part| part == "..")
    {
        return Err(
            "Remote folder is invalid (length, control characters or parent traversal).".into(),
        );
    }
    Ok(())
}

/// The expected snapshot prevents a stale editor from changing another row.
pub fn save(
    hosts: &mut Vec<Host>,
    original: Option<(usize, &Host)>,
    replacement: Host,
) -> Result<bool> {
    validate(&replacement)?;
    let index = if let Some((index, expected)) = original {
        if hosts.get(index) != Some(expected) {
            return Err("This SSH host changed; reopen its settings.".into());
        }
        if expected == &replacement {
            return Ok(false);
        }
        Some(index)
    } else {
        if hosts.len() >= MAX_HOSTS {
            return Err("SSH host list is full.".into());
        }
        None
    };
    if hosts
        .iter()
        .enumerate()
        .any(|(i, host)| Some(i) != index && host == &replacement)
    {
        return Err("This SSH host and folder already exist.".into());
    }
    if let Some(index) = index {
        hosts[index] = replacement;
    } else {
        hosts.push(replacement);
    }
    Ok(true)
}

/// Removing saved metadata does not close or modify already running sessions.
pub fn remove(hosts: &mut Vec<Host>, index: usize, expected: &Host) -> Result<()> {
    if hosts.get(index) != Some(expected) {
        return Err("This SSH host changed; reopen its settings.".into());
    }
    hosts.remove(index);
    Ok(())
}

/// Explicit import preserves existing manual targets and never partially applies a failed batch.
pub fn import_aliases(hosts: &mut Vec<Host>, aliases: &[String]) -> Result<usize> {
    let mut next = hosts.clone();
    let mut added = 0;
    for alias in aliases {
        validate_host(alias)?;
        if next.iter().any(|host| host.alias == *alias) {
            continue;
        }
        save(
            &mut next,
            None,
            Host {
                alias: alias.clone(),
                folder: String::new(),
            },
        )?;
        added += 1;
    }
    *hosts = next;
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn host(alias: &str, folder: &str) -> Host {
        Host {
            alias: alias.into(),
            folder: folder.into(),
        }
    }

    #[test]
    fn config_import_keeps_manual_folders_and_rolls_back_a_failed_batch() {
        let mut hosts = vec![host("existing", "/srv/custom")];
        assert_eq!(
            import_aliases(&mut hosts, &["existing".into(), "new".into(), "new".into()]).unwrap(),
            1
        );
        assert_eq!(hosts, [host("existing", "/srv/custom"), host("new", "")]);
        let before = hosts.clone();
        assert!(import_aliases(&mut hosts, &["valid".into(), "-bad".into()]).is_err());
        assert_eq!(hosts, before);
        hosts = (0..MAX_HOSTS - 1)
            .map(|i| host(&format!("host{i}"), ""))
            .collect();
        let before = hosts.clone();
        assert!(import_aliases(&mut hosts, &["one".into(), "two".into()]).is_err());
        assert_eq!(hosts, before);
    }

    #[test]
    fn create_edit_remove_are_snapshot_checked_and_do_not_duplicate_metadata() {
        let mut hosts = Vec::new();
        let first = host("fixture", "/srv/app");
        assert!(save(&mut hosts, None, first.clone()).unwrap());
        assert!(save(&mut hosts, None, first.clone()).is_err());
        assert!(!save(&mut hosts, Some((0, &first)), first.clone()).unwrap());
        let second = host("fixture", "/srv/other");
        assert!(save(&mut hosts, None, second.clone()).unwrap());
        assert!(save(&mut hosts, Some((0, &first)), second.clone()).is_err());
        assert!(save(&mut hosts, Some((1, &first)), host("wrong", "")).is_err());
        assert!(remove(&mut hosts, 1, &first).is_err());
        assert!(save(&mut hosts, Some((0, &first)), host("edited", "/srv/app")).unwrap());
        assert!(remove(&mut hosts, 0, &first).is_err());
        remove(&mut hosts, 1, &second).unwrap();
        assert_eq!(hosts, vec![host("edited", "/srv/app")]);
        assert!(remove(&mut hosts, usize::MAX, &second).is_err());
    }

    #[test]
    fn invalid_or_full_updates_never_mutate_the_list() {
        let mut hosts = vec![host("original", "")];
        let original = hosts.clone();
        for replacement in [
            host("-oProxyCommand=bad", ""),
            host("fixture", "bad\npath"),
            host("fixture", "/srv/../private"),
            host("fixture", r"folder\..\private"),
            host("fixture", &"x".repeat(8193)),
        ] {
            assert!(save(&mut hosts, None, replacement).is_err());
            assert_eq!(hosts, original);
        }
        hosts = (0..MAX_HOSTS)
            .map(|i| host(&format!("host{i}"), ""))
            .collect();
        assert!(save(&mut hosts, None, host("extra", "")).is_err());
        let expected = hosts[0].clone();
        assert!(
            save(
                &mut hosts,
                Some((0, &expected)),
                host("edited", "path with spaces")
            )
            .unwrap()
        );
        assert_eq!(hosts.len(), MAX_HOSTS);
    }
    #[test]
    fn failed_persistence_does_not_publish_a_changed_host_list() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        std::fs::create_dir(dir.path().join("workspace.json")).unwrap();
        assert!(
            persist(&mut state, &store, |hosts| save(
                hosts,
                None,
                host("fixture", "")
            ))
            .is_err()
        );
        assert!(state.hosts.is_empty());
        std::fs::remove_dir(dir.path().join("workspace.json")).unwrap();
        assert!(
            persist(&mut state, &store, |hosts| save(
                hosts,
                None,
                host("fixture", "")
            ))
            .unwrap()
        );
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.hosts, state.hosts);
    }
}
