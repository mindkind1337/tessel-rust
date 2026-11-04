//! Terminal leases kept across restarts of the desktop, encrypted for this Windows user (DPAPI,
//! through `crate::vault::protect`). A lease is the right to type into a terminal on a host:
//! it lives here only, never in the workspace, a backup, Live Share or a log. The directory is
//! the user's local application data, the same whatever `--data-dir` or project is open.
//!
//! Each call is one transaction under a file lock: read (bounded), decrypt, check every entry,
//! change, encrypt, replace atomically. A file that does not read back exactly is `Corrupt`
//! and is never written over: only the explicit `reset_keeping_original` sets it aside.
//! Call from a worker thread, never an egui frame (DPAPI and the disk can take time).
//!
//! When to call (terminal worker): `put` after `create`, `take_lease`, or an attachment that
//! returns a lease; `get` before `attach_fresh` at start; `remove` only once an ending is
//! confirmed (`Exit` received, `close` answered, or the terminal reported gone for another
//! epoch), never on a mere disconnection or an ambiguous failure, so the lease can still be
//! used to recover. Discovering another epoch alone must not call `retain_epoch`:
//! another daemon version may coexist with the original one.
//! Windows only, like `crate::vault`.
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tessel_host_protocol::{DedupKey, Epoch, LeaseKey, TerminalId};
use zeroize::{Zeroize, Zeroizing};

#[path = "terminal/host_create_window.rs"]
mod create_window;

/// Most leases kept.
pub const MAX_ENTRIES: usize = 256;
/// Largest encrypted file read.
const MAX_FILE: u64 = 256 * 1024;
/// Largest plaintext accepted after decryption, and produced before encryption.
const MAX_PLAIN: usize = 128 * 1024;
/// Archives `reset_keeping_original` may create (`host-leases.corrupt-1.dat` …).
const MAX_ARCHIVES: u32 = 16;
/// How long a transaction waits for another instance's lock.
const LOCK_WAIT: Duration = Duration::from_secs(2);
const VERSION: u32 = 1;

/// Where a lease applies. Holds no secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseSlot {
    /// The validated OpenSSH alias of the host.
    pub alias: String,
    pub epoch: Epoch,
    pub terminal: TerminalId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseStoreError {
    InvalidSlot,
    Locked,
    Corrupt,
    Full,
    TooManyArchives,
    Io,
    Protect,
    Random,
    MissingPending,
}

impl LeaseStoreError {
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidSlot => "Invalid SSH host for a saved terminal",
            Self::Locked => "Saved terminal access is in use by another Tessel window",
            Self::Corrupt => {
                "Saved terminal access cannot be read; it was kept as is and nothing was changed"
            }
            Self::Full => "Too many saved terminals; close some before keeping more",
            Self::TooManyArchives => {
                "Too many unreadable copies of saved terminal access are already kept"
            }
            Self::Io => "Cannot read or write saved terminal access",
            Self::Protect => "Windows could not protect saved terminal access",
            Self::Random => "Cannot create a private terminal request identifier",
            Self::MissingPending => "The pending terminal request no longer exists",
        }
    }
}

impl std::fmt::Display for LeaseStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for LeaseStoreError {}

/// A lease as text, wiped when dropped. Its own type, so a lease parsed before a later field
/// makes the entry fail (a duplicate, an unknown field) is wiped too: serde drops the fields it
/// already built, and a struct-level `Drop` would never run for an entry never built.
struct Secret(String);

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl Serialize for Secret {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Secret)
    }
}

/// One stored lease.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    alias: String,
    epoch: String,
    terminal: String,
    lease: Secret,
}

/// Collects serialized bytes into a buffer allocated once at `limit`, and refuses a write that
/// would go beyond it before extending: the buffer never grows, so no copy of a lease is left
/// in a freed smaller one.
struct Bounded<'a> {
    buffer: &'a mut Vec<u8>,
    limit: usize,
}

impl Write for Bounded<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.buffer.len() + bytes.len() > self.limit {
            return Err(std::io::Error::other("beyond the limit"));
        }
        self.buffer.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Stored {
    fn matches(&self, slot: &LeaseSlot) -> bool {
        self.alias == slot.alias
            && self.epoch == slot.epoch.as_str()
            && self.terminal == slot.terminal.as_str()
    }

    fn valid(&self) -> bool {
        crate::workspace::validate_host(&self.alias).is_ok()
            && Epoch::parse(&self.epoch).is_some()
            && TerminalId::parse(&self.terminal).is_some()
            && LeaseKey::parse(&self.lease.0).is_some()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredFile {
    version: u32,
    entries: Vec<Stored>,
    #[serde(default, deserialize_with = "present_pending")]
    pending: Option<Vec<StoredPending>>,
}

fn present_pending<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<StoredPending>>, D::Error> {
    Vec::deserialize(deserializer).map(Some)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPending {
    id: String,
    alias: String,
    epoch: String,
    dedup: Secret,
}

#[derive(Default)]
struct Records {
    entries: Vec<Stored>,
    pending: Vec<StoredPending>,
}

/// Metadata only: listing after a restart never exposes a reusable creation key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingInfo {
    pub id: String,
    pub alias: String,
    pub epoch: Epoch,
}

/// One bounded read for the terminal picker. These identities grant no access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseIndex {
    pub saved: Vec<TerminalId>,
    pub pending: Vec<PendingInfo>,
}

/// Exists only in the process that reserved the request. No Deserialize, Clone or Debug.
pub struct CreateToken {
    info: PendingInfo,
    dedup: Secret,
    attempt: Option<create_window::Attempt>,
}
impl CreateToken {
    pub fn info(&self) -> &PendingInfo {
        &self.info
    }
    pub fn key_for_attempt(
        &mut self,
        epoch: &Epoch,
        now: Instant,
        request_timeout: Duration,
    ) -> Result<DedupKey, &'static str> {
        let attempt = self
            .attempt
            .get_or_insert_with(|| create_window::Attempt::begin(self.info.epoch.clone(), now));
        attempt.check(epoch, now, request_timeout).map_err(
            |_| "Creation retry is no longer safe; inspect the host's existing terminals",
        )?;
        DedupKey::parse(&self.dedup.0).ok_or("Invalid private terminal request identifier")
    }
}

/// A handle on the store; it keeps nothing between calls.
pub struct LeaseStore {
    dir: PathBuf,
}

impl LeaseStore {
    /// The user's local application data (`…\AppData\Local\Tessel\TesselRust\data\host-leases`),
    /// whatever data directory or project the window uses.
    pub fn open_default() -> Result<Self, LeaseStoreError> {
        let dirs = directories::ProjectDirs::from("com", "Tessel", "TesselRust")
            .ok_or(LeaseStoreError::Io)?;
        Self::at(&dirs.data_local_dir().join("host-leases"))
    }

    pub(crate) fn at(dir: &Path) -> Result<Self, LeaseStoreError> {
        std::fs::create_dir_all(dir).map_err(|_| LeaseStoreError::Io)?;
        Ok(Self {
            dir: dir.to_owned(),
        })
    }

    fn data(&self) -> PathBuf {
        self.dir.join("host-leases.dat")
    }

    fn archive(&self, n: u32) -> PathBuf {
        self.dir.join(format!("host-leases.corrupt-{n}.dat"))
    }

    /// Persist before sending create. A pending request reserves a lease slot.
    pub fn reserve_pending(
        &self,
        alias: &str,
        epoch: &Epoch,
    ) -> Result<CreateToken, LeaseStoreError> {
        crate::workspace::validate_host(alias).map_err(|_| LeaseStoreError::InvalidSlot)?;
        let mut bytes = Zeroizing::new([0u8; 32]);
        getrandom::fill(bytes.as_mut()).map_err(|_| LeaseStoreError::Random)?;
        let id = TerminalId::from_bytes(bytes[..16].try_into().unwrap())
            .as_str()
            .to_owned();
        let dedup = Secret(
            DedupKey::from_bytes(bytes[16..].try_into().unwrap())
                .as_str()
                .to_owned(),
        );
        self.transaction(|records| {
            if records.entries.len() + records.pending.len() >= MAX_ENTRIES {
                return Err(LeaseStoreError::Full);
            }
            if records
                .pending
                .iter()
                .any(|p| p.id == id || p.dedup.0 == dedup.0)
            {
                return Err(LeaseStoreError::Random);
            }
            records.pending.push(StoredPending {
                id: id.clone(),
                alias: alias.to_owned(),
                epoch: epoch.as_str().to_owned(),
                dedup: Secret(dedup.0.clone()),
            });
            Ok((
                CreateToken {
                    info: PendingInfo {
                        id,
                        alias: alias.to_owned(),
                        epoch: epoch.clone(),
                    },
                    dedup,
                    attempt: None,
                },
                true,
            ))
        })
    }

    pub fn list_pending(&self, alias: &str) -> Result<Vec<PendingInfo>, LeaseStoreError> {
        crate::workspace::validate_host(alias).map_err(|_| LeaseStoreError::InvalidSlot)?;
        self.transaction(|records| {
            Ok((
                records
                    .pending
                    .iter()
                    .filter(|p| p.alias == alias)
                    .map(|p| PendingInfo {
                        id: p.id.clone(),
                        alias: p.alias.clone(),
                        epoch: Epoch::parse(&p.epoch).expect("validated epoch"),
                    })
                    .collect(),
                false,
            ))
        })
    }

    /// Lists saved terminal identities for this exact alias/epoch and pending
    /// metadata for this alias across epochs. One decrypt, no write, no secrets.
    pub fn index(&self, alias: &str, epoch: &Epoch) -> Result<LeaseIndex, LeaseStoreError> {
        crate::workspace::validate_host(alias).map_err(|_| LeaseStoreError::InvalidSlot)?;
        self.transaction(|records| {
            Ok((
                LeaseIndex {
                    saved: records
                        .entries
                        .iter()
                        .filter(|entry| entry.alias == alias && entry.epoch == epoch.as_str())
                        .map(|entry| {
                            TerminalId::parse(&entry.terminal).expect("validated terminal")
                        })
                        .collect(),
                    pending: records
                        .pending
                        .iter()
                        .filter(|entry| entry.alias == alias)
                        .map(|entry| PendingInfo {
                            id: entry.id.clone(),
                            alias: entry.alias.clone(),
                            epoch: Epoch::parse(&entry.epoch).expect("validated epoch"),
                        })
                        .collect(),
                },
                false,
            ))
        })
    }

    /// One atomic write replaces the pending capability with the terminal lease.
    /// Disk failure leaves the pending record intact for same-run recovery.
    /// After a desktop restart, use only for a terminal found independently by
    /// listing and explicit attachment/control; never replay the saved create key.
    pub fn complete_pending(
        &self,
        id: &str,
        slot: &LeaseSlot,
        lease: &LeaseKey,
    ) -> Result<(), LeaseStoreError> {
        check(slot)?;
        self.transaction(|records| {
            let position = records
                .pending
                .iter()
                .position(|p| p.id == id && p.alias == slot.alias && p.epoch == slot.epoch.as_str())
                .ok_or(LeaseStoreError::MissingPending)?;
            records.pending.remove(position);
            // A delayed creation result must not replace a lease saved by a
            // later takeover of this same terminal. Retire the intent atomically.
            if records.entries.iter().any(|entry| entry.matches(slot)) {
                return Ok(((), true));
            }
            let stored = Stored {
                alias: slot.alias.clone(),
                epoch: slot.epoch.as_str().to_owned(),
                terminal: slot.terminal.as_str().to_owned(),
                lease: Secret(lease.as_str().to_owned()),
            };
            records.entries.push(stored);
            Ok(((), true))
        })
    }

    /// Only before the first send, after confirmed non-creation, or explicit
    /// forgetting. A timeout/disconnection is not evidence of non-creation.
    /// A confirmed daemon epoch change also proves the old program cannot survive.
    pub fn remove_pending_confirmed(&self, id: &str) -> Result<(), LeaseStoreError> {
        self.transaction(|records| {
            let before = records.pending.len();
            records.pending.retain(|p| p.id != id);
            Ok(((), before != records.pending.len()))
        })
    }

    pub fn get(&self, slot: &LeaseSlot) -> Result<Option<LeaseKey>, LeaseStoreError> {
        check(slot)?;
        self.transaction(|records| {
            let entries = &mut records.entries;
            let lease = entries
                .iter()
                .find(|e| e.matches(slot))
                .and_then(|e| LeaseKey::parse(&e.lease.0));
            Ok((lease, false))
        })
    }

    /// Keeps `lease` for `slot`, replacing the one it had. A new slot beyond `MAX_ENTRIES` is
    /// `Full`.
    pub fn put(&self, slot: &LeaseSlot, lease: &LeaseKey) -> Result<(), LeaseStoreError> {
        check(slot)?;
        self.transaction(|records| {
            let entries = &mut records.entries;
            let stored = Stored {
                alias: slot.alias.clone(),
                epoch: slot.epoch.as_str().to_owned(),
                terminal: slot.terminal.as_str().to_owned(),
                lease: Secret(lease.as_str().to_owned()),
            };
            let full = entries.len() + records.pending.len() >= MAX_ENTRIES;
            match entries.iter_mut().find(|e| e.matches(slot)) {
                Some(entry) if entry.lease.0 == stored.lease.0 => Ok(((), false)),
                // The replaced entry is dropped, which wipes its lease.
                Some(entry) => {
                    *entry = stored;
                    Ok(((), true))
                }
                None if full => Err(LeaseStoreError::Full),
                None => {
                    entries.push(stored);
                    Ok(((), true))
                }
            }
        })
    }

    /// Forgets the lease of `slot`, if any. Call only once the terminal's end is confirmed.
    pub fn remove(&self, slot: &LeaseSlot) -> Result<(), LeaseStoreError> {
        check(slot)?;
        self.transaction(|records| {
            let entries = &mut records.entries;
            let before = entries.len();
            entries.retain(|e| !e.matches(slot));
            Ok(((), entries.len() != before))
        })
    }

    /// Explicit bulk cleanup only after independently establishing that all old
    /// epochs belong to the same physical host and daemon version and have ended.
    /// A newly discovered epoch is insufficient: versioned daemons can coexist,
    /// and an SSH alias can be repointed. No production caller currently uses this.
    /// Returns how many leases were forgotten.
    pub fn retain_epoch(&self, alias: &str, current: &Epoch) -> Result<usize, LeaseStoreError> {
        crate::workspace::validate_host(alias).map_err(|_| LeaseStoreError::InvalidSlot)?;
        self.transaction(|records| {
            let entries = &mut records.entries;
            let before = entries.len();
            entries.retain(|e| e.alias != alias || e.epoch == current.as_str());
            let old_pending = records.pending.len();
            records
                .pending
                .retain(|p| p.alias != alias || p.epoch == current.as_str());
            let removed = before - entries.len() + old_pending - records.pending.len();
            Ok((removed, removed > 0))
        })
    }

    /// Only on the person's explicit request after `Corrupt`: moves the unreadable file aside
    /// as `host-leases.corrupt-<n>.dat` (never over an existing one) and starts empty.
    pub fn reset_keeping_original(&self) -> Result<(), LeaseStoreError> {
        let _lock = self.lock()?;
        let data = self.data();
        if !data.exists() {
            return Ok(());
        }
        let archive = (1..=MAX_ARCHIVES)
            .map(|n| self.archive(n))
            .find(|path| !path.exists())
            .ok_or(LeaseStoreError::TooManyArchives)?;
        // Under the lock nothing else writes here, so the free name stays free.
        std::fs::rename(&data, &archive).map_err(|_| LeaseStoreError::Io)
    }

    fn lock(&self) -> Result<File, LeaseStoreError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join("host-leases.lock"))
            .map_err(|_| LeaseStoreError::Io)?;
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            if fs2::FileExt::try_lock_exclusive(&file).is_ok() {
                return Ok(file);
            }
            if Instant::now() >= deadline {
                return Err(LeaseStoreError::Locked);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Lock, read, change, and write back only when `change` says so.
    fn transaction<T>(
        &self,
        change: impl FnOnce(&mut Records) -> Result<(T, bool), LeaseStoreError>,
    ) -> Result<T, LeaseStoreError> {
        let _lock = self.lock()?;
        let mut entries = self.load()?;
        let (value, write) = change(&mut entries)?;
        if write {
            self.save(&entries)?;
        }
        Ok(value)
    }

    fn load(&self) -> Result<Records, LeaseStoreError> {
        let file = match File::open(self.data()) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Records::default()),
            Err(_) => return Err(LeaseStoreError::Io),
        };
        let mut encrypted = Zeroizing::new(Vec::new());
        file.take(MAX_FILE + 1)
            .read_to_end(&mut encrypted)
            .map_err(|_| LeaseStoreError::Io)?;
        if encrypted.len() as u64 > MAX_FILE {
            return Err(LeaseStoreError::Corrupt);
        }
        // Whatever fails from here, the file is kept as it is.
        let plain =
            crate::vault::protect(&encrypted, false).map_err(|_| LeaseStoreError::Corrupt)?;
        if plain.len() > MAX_PLAIN {
            return Err(LeaseStoreError::Corrupt);
        }
        // A parse error is never shown: it could quote the file.
        let parsed: StoredFile =
            serde_json::from_slice(&plain).map_err(|_| LeaseStoreError::Corrupt)?;
        if !matches!(
            (parsed.version, parsed.pending.is_some()),
            (1, false) | (2, true)
        ) || parsed.entries.len() + parsed.pending.as_ref().map_or(0, Vec::len) > MAX_ENTRIES
            || !parsed.entries.iter().all(Stored::valid)
        {
            return Err(LeaseStoreError::Corrupt);
        }
        let entries = parsed.entries;
        let duplicate = entries.iter().enumerate().any(|(i, a)| {
            entries[..i]
                .iter()
                .any(|b| a.alias == b.alias && a.epoch == b.epoch && a.terminal == b.terminal)
        });
        if duplicate {
            return Err(LeaseStoreError::Corrupt);
        }
        let pending = parsed.pending.unwrap_or_default();
        for (i, item) in pending.iter().enumerate() {
            if crate::workspace::validate_host(&item.alias).is_err()
                || Epoch::parse(&item.epoch).is_none()
                || TerminalId::parse(&item.id).is_none()
                || DedupKey::parse(&item.dedup.0).is_none()
                || pending[..i].iter().any(|prior| {
                    prior.id == item.id
                        || (prior.alias == item.alias
                            && prior.epoch == item.epoch
                            && prior.dedup.0 == item.dedup.0)
                })
            {
                return Err(LeaseStoreError::Corrupt);
            }
        }
        Ok(Records { entries, pending })
    }

    fn save(&self, records: &Records) -> Result<(), LeaseStoreError> {
        let plain = serialize_records(&records.entries, &records.pending, MAX_PLAIN)?;
        let encrypted =
            crate::vault::protect(&plain, true).map_err(|_| LeaseStoreError::Protect)?;
        let mut temp =
            tempfile::NamedTempFile::new_in(&self.dir).map_err(|_| LeaseStoreError::Io)?;
        temp.write_all(&encrypted)
            .and_then(|()| temp.as_file().sync_all())
            .map_err(|_| LeaseStoreError::Io)?;
        temp.persist(self.data()).map_err(|_| LeaseStoreError::Io)?;
        Ok(())
    }
}

/// The file's plaintext, in a buffer allocated once at `limit` that never grows (`Bounded`):
/// beyond `limit` it is `Full`, and nothing was written to disk.
#[cfg(test)]
fn serialize(entries: &[Stored], limit: usize) -> Result<Zeroizing<Vec<u8>>, LeaseStoreError> {
    serialize_records(entries, &[], limit)
}

fn serialize_records(
    entries: &[Stored],
    pending: &[StoredPending],
    limit: usize,
) -> Result<Zeroizing<Vec<u8>>, LeaseStoreError> {
    #[derive(Serialize)]
    struct Out<'a> {
        version: u32,
        entries: &'a [Stored],
        #[serde(skip_serializing_if = "Option::is_none")]
        pending: Option<&'a [StoredPending]>,
    }
    let mut plain = Zeroizing::new(Vec::with_capacity(limit));
    serde_json::to_writer(
        Bounded {
            buffer: &mut plain,
            limit,
        },
        &Out {
            version: if pending.is_empty() { VERSION } else { 2 },
            entries,
            pending: (!pending.is_empty()).then_some(pending),
        },
    )
    .map_err(|_| LeaseStoreError::Full)?;
    Ok(plain)
}

fn check(slot: &LeaseSlot) -> Result<(), LeaseStoreError> {
    crate::workspace::validate_host(&slot.alias).map_err(|_| LeaseStoreError::InvalidSlot)
}

#[cfg(all(test, windows))]
mod tests {
    //! Every test works in a temporary directory with synthetic leases; nothing here can reach
    //! the user's store (`open_default` is never called).
    use super::*;

    #[test]
    fn delayed_creation_promotion_preserves_an_already_saved_takeover() {
        let (_dir, store) = store();
        let here = slot("fixture", 1, 1);
        let token = store.reserve_pending("fixture", &here.epoch).unwrap();
        store.put(&here, &lease(2)).unwrap();
        store
            .complete_pending(&token.info().id, &here, &lease(1))
            .unwrap();
        assert_eq!(store.get(&here).unwrap(), Some(lease(2)));
        assert!(store.list_pending("fixture").unwrap().is_empty());
    }

    #[test]
    fn pending_promotes_atomically_and_restore_exposes_only_metadata() {
        let (_dir, store) = store();
        let here = slot("fixture", 1, 1);
        store.put(&here, &lease(1)).unwrap();
        let mut token = store.reserve_pending("fixture", &here.epoch).unwrap();
        let start = Instant::now();
        let key = token
            .key_for_attempt(&here.epoch, start, Duration::from_secs(15))
            .unwrap();
        let cipher = std::fs::read(store.data()).unwrap();
        assert!(
            !cipher
                .windows(key.as_str().len())
                .any(|w| w == key.as_str().as_bytes())
        );
        assert_eq!(store.get(&here).unwrap(), Some(lease(1)));
        let metadata = store.list_pending("fixture").unwrap();
        assert_eq!(metadata, vec![token.info().clone()]);
        assert!(
            token
                .key_for_attempt(
                    &here.epoch,
                    start + Duration::from_secs(480),
                    Duration::from_secs(15)
                )
                .is_err()
        );
        let restored = LeaseStore::at(&store.dir).unwrap();
        let target = slot("fixture", 1, 2);
        assert_eq!(
            restored.complete_pending(&metadata[0].id, &slot("other", 1, 2), &lease(2)),
            Err(LeaseStoreError::MissingPending)
        );
        assert_eq!(restored.list_pending("fixture").unwrap().len(), 1);
        restored
            .complete_pending(&metadata[0].id, &target, &lease(2))
            .unwrap();
        assert!(restored.list_pending("fixture").unwrap().is_empty());
        assert_eq!(restored.get(&target).unwrap(), Some(lease(2)));
        let plain = crate::vault::protect(&std::fs::read(store.data()).unwrap(), false).unwrap();
        assert!(
            std::str::from_utf8(&plain)
                .unwrap()
                .contains("\"version\":1")
        );
    }

    #[test]
    fn pending_and_leases_share_capacity_and_epoch_cleanup() {
        let (_dir, store) = store();
        let first = store
            .reserve_pending("fixture", &Epoch::from_bytes([1; 16]))
            .unwrap();
        store
            .transaction(|records| {
                for n in 0..MAX_ENTRIES - 1 {
                    records.entries.push(Stored {
                        alias: "fixture".into(),
                        epoch: Epoch::from_bytes([1; 16]).as_str().into(),
                        terminal: TerminalId::from_bytes((n as u128).to_le_bytes())
                            .as_str()
                            .into(),
                        lease: Secret(lease(1).as_str().into()),
                    });
                }
                Ok(((), true))
            })
            .unwrap();
        assert!(matches!(
            store.reserve_pending("fixture", &Epoch::from_bytes([1; 16])),
            Err(LeaseStoreError::Full)
        ));
        let target = slot("fixture", 1, 255);
        store
            .complete_pending(&first.info().id, &target, &lease(2))
            .unwrap();
        assert_eq!(store.get(&target).unwrap(), Some(lease(2)));
        assert_eq!(
            store
                .retain_epoch("fixture", &Epoch::from_bytes([2; 16]))
                .unwrap(),
            MAX_ENTRIES
        );
        let token = store
            .reserve_pending("fixture", &Epoch::from_bytes([2; 16]))
            .unwrap();
        assert_eq!(
            store
                .retain_epoch("fixture", &Epoch::from_bytes([3; 16]))
                .unwrap(),
            1
        );
        assert!(store.list_pending("fixture").unwrap().is_empty());
        store.remove_pending_confirmed(&token.info().id).unwrap();
    }

    #[test]
    fn failed_promotion_keeps_pending_and_can_be_retried_without_recreation() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_dir, store) = store();
        let here = slot("fixture", 1, 1);
        let token = store.reserve_pending("fixture", &here.epoch).unwrap();
        let before = std::fs::read(store.data()).unwrap();
        // Allow reads/writes, but deny deletion/atomic replacement while held.
        let held = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(store.data())
            .unwrap();
        assert_eq!(
            store.complete_pending(&token.info().id, &here, &lease(1)),
            Err(LeaseStoreError::Io)
        );
        assert_eq!(std::fs::read(store.data()).unwrap(), before);
        assert_eq!(store.get(&here).unwrap(), None);
        assert_eq!(
            store.list_pending("fixture").unwrap(),
            vec![token.info().clone()]
        );
        drop(held);
        store
            .complete_pending(&token.info().id, &here, &lease(1))
            .unwrap();
        assert_eq!(store.get(&here).unwrap(), Some(lease(1)));
        assert!(store.list_pending("fixture").unwrap().is_empty());
    }

    #[test]
    fn malformed_pending_keeps_original_ciphertext() {
        let (_dir, store) = store();
        let token = store
            .reserve_pending("fixture", &Epoch::from_bytes([1; 16]))
            .unwrap();
        let record = format!(
            r#"{{"id":"{}","alias":"fixture","epoch":"{}","dedup":"{}"}}"#,
            token.info().id,
            token.info().epoch.as_str(),
            "a".repeat(32)
        );
        for json in [
            r#"{"version":2,"entries":[],"pending":null}"#.to_owned(),
            r#"{"version":1,"entries":[],"pending":[]}"#.to_owned(),
            format!(r#"{{"version":2,"entries":[],"pending":[{record},{record}]}}"#),
            format!(
                r#"{{"version":2,"entries":[],"pending":[{}]}}"#,
                record.replace("fixture", "-bad")
            ),
        ] {
            write_plain(&store, &json);
            let before = std::fs::read(store.data()).unwrap();
            assert_eq!(store.list_pending("fixture"), Err(LeaseStoreError::Corrupt));
            assert_eq!(std::fs::read(store.data()).unwrap(), before);
        }
    }

    fn store() -> (tempfile::TempDir, LeaseStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = LeaseStore::at(dir.path()).unwrap();
        (dir, store)
    }

    fn slot(alias: &str, epoch: u8, terminal: u8) -> LeaseSlot {
        LeaseSlot {
            alias: alias.into(),
            epoch: Epoch::from_bytes([epoch; 16]),
            terminal: TerminalId::from_bytes([terminal; 16]),
        }
    }

    fn lease(n: u8) -> LeaseKey {
        LeaseKey::from_bytes([n; 16])
    }

    /// Writes `json` encrypted as the store's file.
    fn write_plain(store: &LeaseStore, json: &str) {
        let encrypted = crate::vault::protect(json.as_bytes(), true).unwrap();
        std::fs::write(store.data(), &*encrypted).unwrap();
    }

    #[test]
    fn picker_index_filters_alias_and_epoch_without_rewriting_or_exposing_secrets() {
        let (_dir, store) = store();
        store.put(&slot("fixture", 1, 1), &lease(0xab)).unwrap();
        store.put(&slot("fixture", 2, 2), &lease(0xcd)).unwrap();
        store.put(&slot("other", 1, 3), &lease(0xef)).unwrap();
        let current = store
            .reserve_pending("fixture", &Epoch::from_bytes([1; 16]))
            .unwrap();
        let old = store
            .reserve_pending("fixture", &Epoch::from_bytes([2; 16]))
            .unwrap();
        let _other = store
            .reserve_pending("other", &Epoch::from_bytes([1; 16]))
            .unwrap();
        let before = std::fs::read(store.data()).unwrap();
        let index = store.index("fixture", &Epoch::from_bytes([1; 16])).unwrap();
        assert_eq!(index.saved, vec![slot("fixture", 1, 1).terminal]);
        assert_eq!(
            index.pending,
            vec![current.info().clone(), old.info().clone()]
        );
        assert_eq!(std::fs::read(store.data()).unwrap(), before);
        let debug = format!("{index:?}");
        assert!(!debug.contains(lease(0xab).as_str()));
        assert!(!debug.contains(&current.dedup.0));
        assert_eq!(
            store.index("invalid alias", &Epoch::from_bytes([1; 16])),
            Err(LeaseStoreError::InvalidSlot)
        );
    }

    #[test]
    fn a_lease_kept_comes_back_and_is_never_in_the_clear_on_disk() {
        let (_dir, store) = store();
        let here = slot("owner@fixture", 1, 2);
        assert_eq!(store.get(&here).unwrap(), None);
        store.put(&here, &lease(0xab)).unwrap();
        assert_eq!(store.get(&here).unwrap(), Some(lease(0xab)));
        let bytes = std::fs::read(store.data()).unwrap();
        let secret = lease(0xab).as_str().to_owned();
        assert!(
            !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "the lease is encrypted on disk"
        );
        // Replacing keeps one entry.
        store.put(&here, &lease(0xcd)).unwrap();
        assert_eq!(store.get(&here).unwrap(), Some(lease(0xcd)));
    }

    #[test]
    fn removing_is_idempotent_and_only_touches_its_slot() {
        let (_dir, store) = store();
        let (one, two) = (slot("one", 1, 1), slot("two", 1, 1));
        store.put(&one, &lease(1)).unwrap();
        store.put(&two, &lease(2)).unwrap();
        store.remove(&one).unwrap();
        store.remove(&one).unwrap();
        assert_eq!(store.get(&one).unwrap(), None);
        assert_eq!(store.get(&two).unwrap(), Some(lease(2)));
    }

    #[test]
    fn failed_atomic_replacement_keeps_the_previous_encrypted_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_dir, store) = store();
        let here = slot("fixture", 1, 1);
        store.put(&here, &lease(1)).unwrap();
        let before = std::fs::read(store.data()).unwrap();
        // Allow loading the old file but deny deletion/replacement until released.
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2)
            .open(store.data())
            .unwrap();
        assert_eq!(store.put(&here, &lease(2)), Err(LeaseStoreError::Io));
        assert_eq!(std::fs::read(store.data()).unwrap(), before);
        drop(held);
        assert_eq!(store.get(&here).unwrap(), Some(lease(1)));
    }

    #[test]
    fn beyond_the_limit_a_new_slot_is_refused_but_a_kept_one_still_changes() {
        let (_dir, store) = store();
        let numbered = |n: usize| {
            let mut bytes = [9u8; 16];
            bytes[0] = (n % 256) as u8;
            bytes[1] = (n / 256) as u8;
            LeaseSlot {
                terminal: TerminalId::from_bytes(bytes),
                ..slot("fixture", 1, 0)
            }
        };
        for n in 0..MAX_ENTRIES {
            store.put(&numbered(n), &lease(1)).unwrap();
        }
        assert_eq!(
            store.put(&slot("fixture", 2, 2), &lease(1)),
            Err(LeaseStoreError::Full)
        );
        store.put(&numbered(0), &lease(2)).unwrap();
        assert_eq!(store.get(&numbered(0)).unwrap(), Some(lease(2)));
    }

    #[test]
    fn an_unreadable_file_is_kept_and_changes_nothing_until_set_aside() {
        let (_dir, store) = store();
        std::fs::write(store.data(), b"not protected data").unwrap();
        let here = slot("fixture", 1, 1);
        assert_eq!(store.get(&here), Err(LeaseStoreError::Corrupt));
        assert_eq!(store.put(&here, &lease(1)), Err(LeaseStoreError::Corrupt));
        assert_eq!(store.remove(&here), Err(LeaseStoreError::Corrupt));
        assert_eq!(std::fs::read(store.data()).unwrap(), b"not protected data");

        store.reset_keeping_original().unwrap();
        assert_eq!(
            std::fs::read(store.archive(1)).unwrap(),
            b"not protected data"
        );
        assert_eq!(store.get(&here).unwrap(), None);

        // A second one never replaces the first archive.
        std::fs::write(store.data(), b"another bad file").unwrap();
        store.reset_keeping_original().unwrap();
        assert_eq!(
            std::fs::read(store.archive(1)).unwrap(),
            b"not protected data"
        );
        assert_eq!(
            std::fs::read(store.archive(2)).unwrap(),
            b"another bad file"
        );
    }

    #[test]
    fn archives_are_bounded() {
        let (_dir, store) = store();
        for n in 1..=MAX_ARCHIVES {
            std::fs::write(store.archive(n), b"old").unwrap();
        }
        std::fs::write(store.data(), b"bad").unwrap();
        assert_eq!(
            store.reset_keeping_original(),
            Err(LeaseStoreError::TooManyArchives)
        );
        assert_eq!(std::fs::read(store.data()).unwrap(), b"bad");
    }

    #[test]
    fn decrypted_contents_that_do_not_read_back_exactly_are_corrupt() {
        let (_dir, store) = store();
        let here = slot("fixture", 1, 1);
        let good = format!(
            r#"{{"alias":"fixture","epoch":"{}","terminal":"{}","lease":"{}"}}"#,
            here.epoch.as_str(),
            here.terminal.as_str(),
            lease(1).as_str()
        );
        for json in [
            format!(r#"{{"version":2,"entries":[{good}]}}"#),
            format!(r#"{{"version":1,"entries":[{good}],"extra":1}}"#),
            format!(r#"{{"version":1,"entries":[{good},{good}]}}"#),
            format!(
                r#"{{"version":1,"entries":[{}]}}"#,
                good.replace("fixture", "-oProxyCommand=x")
            ),
            format!(
                r#"{{"version":1,"entries":[{}]}}"#,
                good.replace(lease(1).as_str(), "short")
            ),
            format!(
                r#"{{"version":1,"entries":[{}]}}"#,
                vec![good.clone(); MAX_ENTRIES + 1].join(",")
            ),
            format!(
                r#"{{"version":1,"entries":[],"pad":"{}"}}"#,
                "x".repeat(MAX_PLAIN)
            ),
        ] {
            write_plain(&store, &json);
            assert_eq!(
                store.get(&here),
                Err(LeaseStoreError::Corrupt),
                "{}",
                &json[..60.min(json.len())]
            );
        }
        write_plain(&store, &format!(r#"{{"version":1,"entries":[{good}]}}"#));
        assert_eq!(store.get(&here).unwrap(), Some(lease(1)));
    }

    #[test]
    fn an_invalid_alias_is_refused_before_touching_the_file() {
        let (_dir, store) = store();
        for alias in ["", "-oProxyCommand=x", "host name", "host;rm"] {
            assert_eq!(
                store.put(&slot(alias, 1, 1), &lease(1)),
                Err(LeaseStoreError::InvalidSlot)
            );
        }
        assert!(!store.data().exists());
    }

    #[test]
    fn a_new_epoch_forgets_only_that_hosts_older_leases() {
        let (_dir, store) = store();
        store.put(&slot("alpha", 1, 1), &lease(1)).unwrap();
        store.put(&slot("alpha", 2, 2), &lease(2)).unwrap();
        store.put(&slot("beta", 1, 3), &lease(3)).unwrap();
        assert_eq!(
            store
                .retain_epoch("alpha", &Epoch::from_bytes([2; 16]))
                .unwrap(),
            1
        );
        assert_eq!(store.get(&slot("alpha", 1, 1)).unwrap(), None);
        assert_eq!(store.get(&slot("alpha", 2, 2)).unwrap(), Some(lease(2)));
        assert_eq!(store.get(&slot("beta", 1, 3)).unwrap(), Some(lease(3)));
    }

    #[test]
    fn the_serializer_refuses_beyond_its_limit_without_growing_its_buffer() {
        let mut buffer = Vec::with_capacity(16);
        let (start, capacity) = (buffer.as_ptr(), buffer.capacity());
        let mut writer = Bounded {
            buffer: &mut buffer,
            limit: 16,
        };
        writer.write_all(b"0123456789").unwrap();
        assert!(writer.write_all(b"0123456789").is_err());
        assert_eq!(buffer, b"0123456789");
        assert_eq!((buffer.as_ptr(), buffer.capacity()), (start, capacity));

        let entries = vec![Stored {
            alias: "fixture".into(),
            epoch: Epoch::from_bytes([1; 16]).as_str().into(),
            terminal: TerminalId::from_bytes([1; 16]).as_str().into(),
            lease: Secret(lease(1).as_str().into()),
        }];
        assert!(matches!(
            serialize(&entries, 32),
            Err(LeaseStoreError::Full)
        ));
        assert!(serialize(&entries, MAX_PLAIN).is_ok());
    }

    #[test]
    fn every_slot_with_the_longest_alias_fits_the_limit() {
        let entries: Vec<Stored> = (0..MAX_ENTRIES)
            .map(|n| {
                let alias = format!("{n:03}{}", "a".repeat(250));
                assert!(crate::workspace::validate_host(&alias).is_ok());
                Stored {
                    alias,
                    epoch: Epoch::from_bytes([n as u8; 16]).as_str().into(),
                    terminal: TerminalId::from_bytes([n as u8; 16]).as_str().into(),
                    lease: Secret(lease(n as u8).as_str().into()),
                }
            })
            .collect();
        let plain = serialize(&entries, MAX_PLAIN).unwrap();
        assert!(plain.len() < MAX_PLAIN, "{} bytes", plain.len());
    }

    #[test]
    fn another_holder_of_the_lock_makes_it_wait_then_refuse() {
        let (_dir, store) = store();
        let held = store.lock().unwrap();
        let started = Instant::now();
        assert_eq!(
            store.put(&slot("fixture", 1, 1), &lease(1)),
            Err(LeaseStoreError::Locked)
        );
        assert!(started.elapsed() >= LOCK_WAIT);
        drop(held);
        store.put(&slot("fixture", 1, 1), &lease(1)).unwrap();
    }
}
