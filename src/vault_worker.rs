//! Single owner of the decrypted vault. Widgets receive only metadata snapshots.
use crate::{
    browser_passwords_model::VaultEntryMetadata,
    vault::{Account, Vault},
};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
};

#[derive(Clone)]
pub struct Snapshot {
    pub revision: u64,
    pub entries: Vec<VaultEntryMetadata>,
}

/// Never Debug/Serialize: a fill payload is consumed immediately by the UI-thread
/// WebView bridge, then Account::drop wipes its secret. It is never widget state.
pub enum Effect {
    Opened,
    Imported(usize),
    Removed,
    Fill(Account),
    Cancelled,
}
pub struct Reply {
    pub request: u64,
    pub snapshot: Option<Snapshot>,
    pub result: Result<Effect, String>,
}
pub enum Command {
    ImportCsv(PathBuf),
    Delete(String),
    Fill { id: String, origin: String },
}
struct Request {
    id: u64,
    revision: u64,
    command: Command,
}

pub struct Worker {
    commands: SyncSender<Request>,
    replies: Receiver<Reply>,
    cancel: Arc<AtomicBool>,
    next: u64,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

fn account_id(account: &Account) -> String {
    // Stable metadata identity without changing the existing encrypted file format.
    // The revision also changes on secret replacement, rejecting stale confirmations.
    let mut hash = Sha256::new();
    hash.update((account.origin.len() as u64).to_le_bytes());
    hash.update(account.origin.as_bytes());
    hash.update((account.username.len() as u64).to_le_bytes());
    hash.update(account.username.as_bytes());
    format!("{:x}", hash.finalize())
}
fn snapshot(vault: &Vault, revision: u64) -> Snapshot {
    Snapshot {
        revision,
        entries: vault
            .accounts
            .iter()
            .map(|account| VaultEntryMetadata {
                id: account_id(account),
                origin: account.origin.clone(),
                username: account.username.clone(),
                // Agent access is intentionally unavailable until that backend is integrated.
                allow_agents: false,
            })
            .collect(),
    }
}

impl Worker {
    pub fn open_default() -> Result<Self, String> {
        Self::start(None)
    }
    fn start(directory: Option<PathBuf>) -> Result<Self, String> {
        Self::spawn(directory, None)
    }
    #[cfg(feature = "visual-test")]
    pub fn fixture() -> Result<Self, String> {
        let directory = tempfile::tempdir().map_err(|_| "Cannot create vault fixture")?;
        let path = directory.path().join("fixture.csv");
        std::fs::write(&path, "url,username,password\nhttps://example.com,compte-demo,synthetic-only\nhttps://mail.example,autre-compte,synthetic-only\n")
            .map_err(|_| "Cannot write vault fixture")?;
        let mut vault = Vault::open(directory.path())?;
        vault.import_csv(&path)?;
        drop(vault);
        Self::spawn(Some(directory.path().to_owned()), Some(directory))
    }
    fn spawn(
        directory: Option<PathBuf>,
        retained: Option<tempfile::TempDir>,
    ) -> Result<Self, String> {
        let (commands, incoming) = mpsc::sync_channel::<Request>(1);
        let (outgoing, replies) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::Builder::new()
            .name("private-vault".into())
            .spawn(move || {
                let _retained = retained;
                if flag.load(Ordering::Acquire) {
                    return;
                }
                let opened = match directory {
                    Some(path) => Vault::open(&path),
                    None => Vault::open_default(),
                };
                let mut vault = match opened {
                    Ok(vault) => vault,
                    Err(error) => {
                        let _ = outgoing.send(Reply {
                            request: 0,
                            snapshot: None,
                            result: Err(error),
                        });
                        return;
                    }
                };
                let mut revision = 0;
                let mut identities = std::collections::HashSet::new();
                if vault
                    .accounts
                    .iter()
                    .any(|account| !identities.insert(account_id(account)))
                {
                    let _ = outgoing.send(Reply {
                        request: 0,
                        snapshot: None,
                        result: Err(
                            "Vault contains duplicate login identities; original retained".into(),
                        ),
                    });
                    return;
                }
                if flag.load(Ordering::Acquire) {
                    return;
                }
                if outgoing
                    .send(Reply {
                        request: 0,
                        snapshot: Some(snapshot(&vault, revision)),
                        result: Ok(Effect::Opened),
                    })
                    .is_err()
                {
                    return;
                }
                while let Ok(request) = incoming.recv() {
                    if flag.load(Ordering::Acquire) {
                        break;
                    }
                    let result = execute(
                        &mut vault,
                        &mut revision,
                        request.revision,
                        request.command,
                        &flag,
                    );
                    if outgoing
                        .send(Reply {
                            request: request.id,
                            snapshot: Some(snapshot(&vault, revision)),
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|_| "Cannot start private vault worker")?;
        Ok(Self {
            commands,
            replies,
            cancel,
            next: 1,
        })
    }
    pub fn submit(&mut self, revision: u64, command: Command) -> Result<u64, String> {
        let id = self.next;
        let next = id.checked_add(1).ok_or("Vault request counter exhausted")?;
        self.commands
            .try_send(Request {
                id,
                revision,
                command,
            })
            .map_err(|_| "Vault worker is busy or closed")?;
        self.next = next;
        Ok(id)
    }
    pub fn poll(&self) -> Result<Option<Reply>, String> {
        match self.replies.try_recv() {
            Ok(reply) => Ok(Some(reply)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err("Private vault worker stopped".into()),
        }
    }
}

fn execute(
    vault: &mut Vault,
    revision: &mut u64,
    expected: u64,
    command: Command,
    cancel: &AtomicBool,
) -> Result<Effect, String> {
    if *revision != expected {
        return Err("Vault changed; repeat the action using the refreshed list".into());
    }
    match command {
        Command::ImportCsv(path) => {
            if cancel.load(Ordering::Acquire) {
                return Ok(Effect::Cancelled);
            }
            if !path.to_str().is_some_and(|path| {
                crate::browser_passwords_model::is_allowed_local_import_file_path(
                    path,
                    cfg!(windows),
                )
            }) {
                return Err("Choose a local password CSV file".into());
            }
            let metadata = std::fs::symlink_metadata(&path).map_err(|_| "Cannot read CSV")?;
            if !metadata.is_file() || metadata.len() > 5 * 1024 * 1024 {
                return Err("Choose a regular CSV file no larger than 5 MiB".into());
            }
            let next = revision
                .checked_add(1)
                .ok_or("Vault revision counter exhausted")?;
            let count = vault.import_csv(&path)?;
            *revision = next;
            Ok(Effect::Imported(count))
        }
        Command::Delete(id) => {
            let index = vault
                .accounts
                .iter()
                .position(|account| account_id(account) == id)
                .ok_or("Unknown vault account")?;
            let next = revision
                .checked_add(1)
                .ok_or("Vault revision counter exhausted")?;
            vault.remove(index)?;
            *revision = next;
            Ok(Effect::Removed)
        }
        Command::Fill { id, origin } => {
            let account = vault
                .accounts
                .iter()
                .find(|account| account_id(account) == id)
                .ok_or("Unknown vault account")?;
            if crate::vault::origin(&origin)? != account.origin {
                return Err("The browser must be on this account's exact HTTPS site".into());
            }
            Ok(Effect::Fill(Account {
                origin: account.origin.clone(),
                username: account.username.clone(),
                password: account.password.clone(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn seeded() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.csv");
        std::fs::write(&path, "url,username,password\nhttps://one.example,u,synthetic-one\nhttps://two.example,v,synthetic-two\n").unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();
        vault.import_csv(&path).unwrap();
        drop(vault);
        dir
    }
    fn receive(worker: &Worker) -> Reply {
        worker.replies.recv_timeout(Duration::from_secs(5)).unwrap()
    }
    #[test]
    fn stale_actions_are_rejected_and_fill_is_origin_bound() {
        let dir = seeded();
        let mut worker = Worker::start(Some(dir.path().to_owned())).unwrap();
        let opened = receive(&worker);
        let snapshot = opened.snapshot.unwrap();
        assert_eq!(snapshot.revision, 0);
        let metadata = serde_json::to_string(&snapshot.entries).unwrap();
        assert!(!metadata.contains("synthetic-one"));
        assert!(!metadata.contains("synthetic-two"));
        assert!(snapshot.entries.iter().all(|entry| !entry.allow_agents));
        let first = snapshot.entries[0].id.clone();
        let second = snapshot.entries[1].id.clone();
        worker.submit(0, Command::Delete(first)).unwrap();
        let removed = receive(&worker);
        assert!(matches!(removed.result, Ok(Effect::Removed)));
        assert_eq!(removed.snapshot.unwrap().revision, 1);
        worker.submit(0, Command::Delete(second.clone())).unwrap();
        let stale = receive(&worker);
        assert!(stale.result.is_err());
        assert_eq!(stale.snapshot.unwrap().entries.len(), 1);
        worker
            .submit(
                1,
                Command::Fill {
                    id: second.clone(),
                    origin: "https://one.example".into(),
                },
            )
            .unwrap();
        assert!(receive(&worker).result.is_err());
        worker
            .submit(
                1,
                Command::Fill {
                    id: second,
                    origin: "https://two.example".into(),
                },
            )
            .unwrap();
        let Ok(Effect::Fill(account)) = receive(&worker).result else {
            panic!("expected private fill ticket")
        };
        assert_eq!(account.password, "synthetic-two");
    }

    #[test]
    fn failed_save_preserves_revision_metadata_and_encrypted_original() {
        let dir = seeded();
        let path = dir.path().join("vault.dat");
        let backup = dir.path().join("fixture-original.dat");
        let mut worker = Worker::start(Some(dir.path().to_owned())).unwrap();
        let snapshot = receive(&worker).snapshot.unwrap();
        std::fs::rename(&path, &backup).unwrap();
        std::fs::create_dir(&path).unwrap();
        worker
            .submit(0, Command::Delete(snapshot.entries[0].id.clone()))
            .unwrap();
        let reply = receive(&worker);
        assert!(reply.result.is_err());
        let after = reply.snapshot.unwrap();
        assert_eq!(after.revision, 0);
        assert_eq!(after.entries, snapshot.entries);
        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(&backup, &path).unwrap();
        drop(worker);
        let deadline = Instant::now() + Duration::from_secs(5);
        let reopened = loop {
            if let Ok(vault) = Vault::open(dir.path()) {
                break vault;
            }
            assert!(Instant::now() < deadline, "worker did not release vault");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(reopened.accounts.len(), 2);
    }

    #[test]
    fn replacing_a_secret_invalidates_old_actions_without_changing_metadata_identity() {
        let dir = seeded();
        let mut worker = Worker::start(Some(dir.path().to_owned())).unwrap();
        let before = receive(&worker).snapshot.unwrap();
        let path = dir.path().join("replacement.csv");
        std::fs::write(
            &path,
            "url,username,password\nhttps://one.example,u,synthetic-replacement\n",
        )
        .unwrap();
        worker.submit(0, Command::ImportCsv(path)).unwrap();
        let reply = receive(&worker);
        assert!(matches!(reply.result, Ok(Effect::Imported(1))));
        let after = reply.snapshot.unwrap();
        assert_eq!(after.entries, before.entries);
        assert_eq!(after.revision, 1);
        let id = before.entries[0].id.clone();
        worker
            .submit(
                0,
                Command::Fill {
                    id: id.clone(),
                    origin: "https://one.example".into(),
                },
            )
            .unwrap();
        assert!(receive(&worker).result.is_err());
        worker
            .submit(
                1,
                Command::Fill {
                    id,
                    origin: "https://one.example".into(),
                },
            )
            .unwrap();
        let Ok(Effect::Fill(account)) = receive(&worker).result else {
            panic!("expected current private ticket")
        };
        assert_eq!(account.password, "synthetic-replacement");
    }
}
