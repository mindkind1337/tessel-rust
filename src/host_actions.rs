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
        // Remember removals and renamed endpoints, so passive discovery cannot resurrect them.
        for old in &state.hosts {
            if !next.hosts.iter().any(|host| host.alias == old.alias) {
                next.removed_ssh_aliases.retain(|alias| alias != &old.alias);
                next.removed_ssh_aliases.push(old.alias.clone());
            }
        }
        next.removed_ssh_aliases
            .retain(|alias| !next.hosts.iter().any(|host| &host.alias == alias));
        let excess = next.removed_ssh_aliases.len().saturating_sub(200);
        next.removed_ssh_aliases.drain(..excess);
        // Persist before publishing UI state; failed writes leave the original intact.
        store.save(&next)?;
        state.hosts = next.hosts;
        state.removed_ssh_aliases = next.removed_ssh_aliases;
    }
    Ok(changed)
}

/// Passive sync respects removals; explicit Import re-adopts them only after a successful save.
pub fn sync_config(
    state: &mut Workspace,
    store: &Store,
    aliases: &[String],
    re_adopt: bool,
) -> Result<usize> {
    let mut next = state.clone();
    if re_adopt {
        next.removed_ssh_aliases.clear();
    }
    let aliases: Vec<_> = aliases
        .iter()
        .filter(|alias| !next.removed_ssh_aliases.contains(alias))
        .cloned()
        .collect();
    let added = import_aliases(&mut next.hosts, &aliases)?;
    if added != 0 || next.removed_ssh_aliases != state.removed_ssh_aliases {
        store.save(&next)?;
        state.hosts = next.hosts;
        state.removed_ssh_aliases = next.removed_ssh_aliases;
    }
    Ok(added)
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
    mut replacement: Host,
) -> Result<bool> {
    validate(&replacement)?;
    let index = if let Some((index, expected)) = original {
        if hosts.get(index) != Some(expected) {
            return Err("This SSH host changed; reopen its settings.".into());
        }
        replacement.target_id.clone_from(&expected.target_id);
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
    if hosts.iter().enumerate().any(|(i, host)| {
        Some(i) != index && host.alias == replacement.alias && host.folder == replacement.folder
    }) {
        return Err("This SSH host and folder already exist.".into());
    }
    if let Some(index) = index {
        let old = &hosts[index];
        replacement.target_id = Some(
            if old.target_id.is_none()
                && hosts
                    .iter()
                    .enumerate()
                    .any(|(i, h)| i != index && h.same_target(old))
            {
                crate::claude::uuid()?
            } else {
                old.identity().into_owned()
            },
        );
        hosts[index] = replacement;
    } else {
        replacement.target_id = Some(crate::claude::uuid()?);
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
                ..Default::default()
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
            ..Default::default()
        }
    }

    #[test]
    fn config_import_keeps_manual_folders_and_rolls_back_a_failed_batch() {
        let mut hosts = vec![host("existing", "/srv/custom")];
        assert_eq!(
            import_aliases(&mut hosts, &["existing".into(), "new".into(), "new".into()]).unwrap(),
            1
        );
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0], host("existing", "/srv/custom"));
        assert_eq!(hosts[1].alias, "new");
        assert!(hosts[1].folder.is_empty());
        assert!(hosts[1].target_id.is_some());
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
    fn ssh_target_ownership_survives_edits_restore_and_reuse_of_the_old_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let original = host("lab", "/before");
        let launch = crate::launch::Launch::ssh(original.clone());
        state.hosts.push(original.clone());
        persist(&mut state, &store, |list| {
            save(list, Some((0, &original)), host("renamed", "/after"))
        })
        .unwrap();
        persist(&mut state, &store, |list| {
            save(list, None, original.clone())
        })
        .unwrap();
        drop(store);
        let (store, reopened) = Store::open(dir.path()).unwrap();
        assert!(launch.belongs_to_ssh_target(&reopened.hosts[0]));
        assert!(!launch.belongs_to_ssh_target(&reopened.hosts[1]));
        let first_new = reopened.hosts[1].clone();
        let new_launch = crate::launch::Launch::ssh(first_new.clone());
        persist(&mut state, &store, |list| {
            save(list, Some((1, &first_new)), host("fresh", "/updated"))
        })
        .unwrap();
        assert!(new_launch.belongs_to_ssh_target(&state.hosts[1]));
        assert!(!launch.belongs_to_ssh_target(&state.hosts[1]));
    }

    #[test]
    fn create_edit_remove_are_snapshot_checked_and_do_not_duplicate_metadata() {
        let mut hosts = Vec::new();
        let first = host("fixture", "/srv/app");
        assert!(save(&mut hosts, None, first.clone()).unwrap());
        let first = hosts[0].clone();
        assert!(save(&mut hosts, None, first.clone()).is_err());
        assert!(!save(&mut hosts, Some((0, &first)), first.clone()).unwrap());
        let second = host("fixture", "/srv/other");
        assert!(save(&mut hosts, None, second.clone()).unwrap());
        let second = hosts[1].clone();
        assert!(save(&mut hosts, Some((0, &first)), second.clone()).is_err());
        assert!(save(&mut hosts, Some((1, &first)), host("wrong", "")).is_err());
        assert!(remove(&mut hosts, 1, &first).is_err());
        assert!(save(&mut hosts, Some((0, &first)), host("edited", "/srv/app")).unwrap());
        assert!(remove(&mut hosts, 0, &first).is_err());
        remove(&mut hosts, 1, &second).unwrap();
        assert_eq!(hosts.len(), 1);
        assert!(hosts[0].same_target(&first));
        assert_eq!(
            (&*hosts[0].alias, &*hosts[0].folder),
            ("edited", "/srv/app")
        );
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
    fn passive_sync_remembers_removed_and_renamed_aliases_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let aliases = vec!["lab".into(), "office".into()];
        assert_eq!(sync_config(&mut state, &store, &aliases, false).unwrap(), 2);
        let lab = state.hosts[0].clone();
        persist(&mut state, &store, |hosts| {
            remove(hosts, 0, &lab)?;
            Ok(true)
        })
        .unwrap();
        let office = state.hosts[0].clone();
        persist(&mut state, &store, |hosts| {
            save(hosts, Some((0, &office)), host("edited", "/manual"))
        })
        .unwrap();
        assert_eq!(state.removed_ssh_aliases, ["lab", "office"]);
        drop(store);
        let (store, mut restored) = Store::open(dir.path()).unwrap();
        assert_eq!(
            sync_config(&mut restored, &store, &aliases, false).unwrap(),
            0
        );
        assert_eq!(restored.hosts.len(), 1);
        assert_eq!(restored.hosts[0].folder, "/manual");
        assert_eq!(
            sync_config(&mut restored, &store, &aliases, true).unwrap(),
            2
        );
        assert!(restored.removed_ssh_aliases.is_empty());
        assert_eq!(restored.hosts[0].folder, "/manual");
    }

    #[test]
    fn failed_readoption_retains_removals_and_manual_add_can_restore_an_alias() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.removed_ssh_aliases = vec!["lab".into()];
        std::fs::create_dir(dir.path().join("workspace.json")).unwrap();
        assert!(sync_config(&mut state, &store, &["lab".into()], true).is_err());
        assert!(state.hosts.is_empty());
        assert_eq!(state.removed_ssh_aliases, ["lab"]);
        std::fs::remove_dir(dir.path().join("workspace.json")).unwrap();
        persist(&mut state, &store, |hosts| {
            save(hosts, None, host("lab", "/manual"))
        })
        .unwrap();
        assert!(state.removed_ssh_aliases.is_empty());
        assert_eq!(
            sync_config(&mut state, &store, &["lab".into()], false).unwrap(),
            0
        );
        assert_eq!(state.hosts[0].folder, "/manual");
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
