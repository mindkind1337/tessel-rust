#![allow(dead_code)]
//! Per-host `tessel-host` opt-in gate, direct-exec command verifier, and remote `Files`/`Changes`
//! watch caches with instant SFTP / `git` fallback.
//!
//! Mirrors Tessel's `hostDaemon.js`, `hostFsCache.js`, and `hostGitCache.js`:
//! - `Host::host_daemon` defaults to `false`; Tessel never installs or starts `tessel-host` in
//!   the background unless the user explicitly enables it on that SSH host (or clicks Connect in
//!   the preview panel).
//! - Direct-exec `ssh` connector strictly enforces
//!   `^"\$HOME/\.tessel-server/host/bin/tessel-host-[a-z0-9._-]{1,32}-(?:x86_64|aarch64)" connect --stdio$`.
//! - `RemoteTreeCache` gates reads on `reset == true` (`ready`), honors a `4 s` post-write
//!   revision barrier (`POST_WRITE_WAIT_MS`) so file mutations never show stale directory views,
//!   records a `60 s` refusal cooldown (`REFUSAL_COOLDOWN_MS`), and falls back to SFTP whenever
//!   disconnected or unready.
//! - `RemoteGitLruCache` bounds watched remote repositories to `MAX_GITS = 4` per host and
//!   reconstructs `git status --porcelain=v2 --branch` output for Tessel's SCM parser.

use std::collections::{BTreeMap, VecDeque};

/// Maximum number of concurrently cached remote Git roots per SSH host (`hostGitCache.js`).
pub const MAX_GITS: usize = 4;

/// Post-write wait window (`4 000 ms`) during which `read_dir_source` bypasses `RemoteTreeCache`
/// until `fs.treeUpdate` advances past the revision observed before the write.
pub const POST_WRITE_WAIT_MS: u64 = 4_000;

/// Cooldown (`60 000 ms`) after a `fs.watch` refusal (e.g. repository exceeds entry cap) before
/// attempting to open another watch on that root.
pub const REFUSAL_COOLDOWN_MS: u64 = 60_000;

const CONNECT_PREFIX: &str = "\"$HOME/.tessel-server/host/bin/tessel-host-";
const CONNECT_SUFFIX: &str = "\" connect --stdio";

/// Validates the exact direct-exec remote command allowed on `native_host_transport::verified_args`:
/// `^"\$HOME/\.tessel-server/host/bin/tessel-host-[a-z0-9._-]{1,32}-(?:x86_64|aarch64)" connect --stdio$`.
#[must_use]
pub fn validate_direct_exec_connect_command(command: &str) -> bool {
    let Some(inner) = command
        .strip_prefix(CONNECT_PREFIX)
        .and_then(|rest| rest.strip_suffix(CONNECT_SUFFIX))
    else {
        return false;
    };
    let version = if let Some(v) = inner.strip_suffix("-x86_64") {
        v
    } else if let Some(v) = inner.strip_suffix("-aarch64") {
        v
    } else {
        return false;
    };
    (1..=32).contains(&version.len())
        && version
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
}

/// Returns `true` when `tessel-host` is permitted to run for a host.
///
/// Background `Files` / `Changes` watches require `host_daemon_enabled == true`; explicit user
/// clicks inside the Native Host preview panel pass `explicit_preview_action = true`.
#[must_use]
pub const fn should_use_host_daemon(
    host_daemon_enabled: bool,
    explicit_preview_action: bool,
) -> bool {
    host_daemon_enabled || explicit_preview_action
}

/// Cached directory entry from `tessel-host` `fs.treeUpdate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTreeEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Incremental or full-reset `fs.treeUpdate` payload from `tessel-host`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTreeUpdate {
    pub revision: u64,
    pub reset: bool,
    pub dirs: BTreeMap<String, Vec<RemoteTreeEntry>>,
    pub removed_dirs: Vec<String>,
}

/// Data source selected for a remote directory read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteTreeReadSource {
    HostCache,
    SftpFallback,
}

/// Per-root `fs.watch` tree cache with `reset` readiness gate, `4 s` post-write barrier, and
/// `60 s` refusal cooldown.
#[derive(Clone, Debug, Default)]
pub struct RemoteTreeCache {
    ready: bool,
    revision: u64,
    dirs: BTreeMap<String, Vec<RemoteTreeEntry>>,
    pending_write_revision: Option<(u64, u64)>,
    refused_until_ms: Option<u64>,
}

impl RemoteTreeCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Applies a `fs.treeUpdate` event. Incremental deltas arriving before an initial `reset`
    /// snapshot are ignored so partial trees can never masquerade as complete directories.
    pub fn apply_update(&mut self, update: RemoteTreeUpdate) {
        if update.reset {
            self.dirs = update.dirs;
            self.revision = update.revision;
            self.ready = true;
            self.refused_until_ms = None;
        } else if self.ready {
            for removed in &update.removed_dirs {
                self.dirs.remove(removed);
            }
            for (dir, entries) in update.dirs {
                self.dirs.insert(dir, entries);
            }
            self.revision = update.revision;
        }

        if let Some((write_rev, _)) = self.pending_write_revision
            && self.ready
            && self.revision > write_rev
        {
            self.pending_write_revision = None;
        }
    }

    /// Records a remote file/folder mutation (`create`, `rename`, `delete`, `write`) so reads
    /// bypass potentially stale cache state for up to [`POST_WRITE_WAIT_MS`] until `revision` advances.
    pub fn record_write(&mut self, now_ms: u64) {
        self.pending_write_revision =
            Some((self.revision, now_ms.saturating_add(POST_WRITE_WAIT_MS)));
    }

    /// Records a watch refusal from `tessel-host` and enters a [`REFUSAL_COOLDOWN_MS`] backoff.
    pub fn record_refusal(&mut self, now_ms: u64) {
        self.ready = false;
        self.dirs.clear();
        self.pending_write_revision = None;
        self.refused_until_ms = Some(now_ms.saturating_add(REFUSAL_COOLDOWN_MS));
    }

    /// Clears live readiness when the `tessel-host` connection drops or `host_daemon` is turned off.
    pub fn record_disconnect(&mut self) {
        self.ready = false;
        self.dirs.clear();
        self.pending_write_revision = None;
    }

    /// Returns `true` when a `fs.watch` subscription may be opened at `now_ms`.
    #[must_use]
    pub fn can_open_watch(&self, host_daemon_enabled: bool, connected: bool, now_ms: u64) -> bool {
        host_daemon_enabled
            && connected
            && self.refused_until_ms.is_none_or(|until| now_ms >= until)
    }

    /// Selects [`RemoteTreeReadSource::HostCache`] only when `host_daemon` is enabled, connected,
    /// initialized by a `reset` snapshot, not waiting on a post-write revision bump, and holding `dir`.
    #[must_use]
    pub fn read_dir_source(
        &self,
        host_daemon_enabled: bool,
        connected: bool,
        dir: &str,
        now_ms: u64,
    ) -> RemoteTreeReadSource {
        if !host_daemon_enabled || !connected || !self.ready {
            return RemoteTreeReadSource::SftpFallback;
        }
        if let Some((write_rev, deadline_ms)) = self.pending_write_revision
            && now_ms < deadline_ms
            && self.revision <= write_rev
        {
            return RemoteTreeReadSource::SftpFallback;
        }
        if self.dirs.contains_key(dir) {
            RemoteTreeReadSource::HostCache
        } else {
            RemoteTreeReadSource::SftpFallback
        }
    }

    #[must_use]
    pub fn cached_entries(&self, dir: &str) -> Option<&[RemoteTreeEntry]> {
        self.dirs.get(dir).map(Vec::as_slice)
    }
}

/// Single changed path reported by `tessel-host` `git.statusUpdate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteGitFileEntry {
    /// Two-character porcelain v2 XY status (e.g. `"M."`, `".M"`, `"??"`, `"!!"`).
    pub xy: [char; 2],
    pub path: String,
    pub orig_path: Option<String>,
}

/// Cached `git.watch` snapshot for a remote Git repository root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteGitSnapshot {
    pub oid: String,
    pub head: String,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub files: Vec<RemoteGitFileEntry>,
}

/// Reconstructs `git status --porcelain=v2 --branch` text from a [`RemoteGitSnapshot`].
#[must_use]
pub fn reconstruct_porcelain_v2(snapshot: &RemoteGitSnapshot) -> String {
    let mut out = String::new();
    let oid = if snapshot.oid.trim().is_empty() {
        "(initial)"
    } else {
        snapshot.oid.trim()
    };
    let head = if snapshot.head.trim().is_empty() {
        "(detached)"
    } else {
        snapshot.head.trim()
    };
    out.push_str("# branch.oid ");
    out.push_str(oid);
    out.push('\n');
    out.push_str("# branch.head ");
    out.push_str(head);
    out.push('\n');
    if let Some(up) = snapshot
        .upstream
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push_str("# branch.upstream ");
        out.push_str(up);
        out.push('\n');
        out.push_str(&format!(
            "# branch.ab +{} -{}\n",
            snapshot.ahead, snapshot.behind
        ));
    }
    for entry in &snapshot.files {
        match entry.xy {
            ['?', '?'] => {
                out.push_str("? ");
                out.push_str(&entry.path);
                out.push('\n');
            }
            ['!', '!'] => {
                out.push_str("! ");
                out.push_str(&entry.path);
                out.push('\n');
            }
            [x, y] => {
                if let Some(orig) = &entry.orig_path {
                    out.push_str(&format!(
                        "2 {x}{y} N... 100644 100644 100644 0000000 0000000 R100 {}\t{}\n",
                        entry.path, orig
                    ));
                } else {
                    out.push_str(&format!(
                        "1 {x}{y} N... 100644 100644 100644 0000000 0000000 {}\n",
                        entry.path
                    ));
                }
            }
        }
    }
    out
}

/// Bounded per-host LRU (`MAX_GITS = 4`) of active remote `git.watch` snapshots.
#[derive(Clone, Debug, Default)]
pub struct RemoteGitLruCache {
    order: VecDeque<String>,
    snapshots: BTreeMap<String, RemoteGitSnapshot>,
}

impl RemoteGitLruCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    #[must_use]
    pub fn contains_root(&self, root: &str) -> bool {
        self.snapshots.contains_key(root)
    }

    fn touch(&mut self, root: &str) {
        if let Some(pos) = self.order.iter().position(|r| r == root) {
            self.order.remove(pos);
        }
        self.order.push_back(root.to_owned());
    }

    /// Inserts or updates `root`, evicting and returning the least-recently-used root if `> MAX_GITS`.
    pub fn put(&mut self, root: impl Into<String>, snapshot: RemoteGitSnapshot) -> Option<String> {
        let key = root.into();
        self.snapshots.insert(key.clone(), snapshot);
        self.touch(&key);
        if self.snapshots.len() > MAX_GITS
            && let Some(evicted) = self.order.pop_front()
        {
            self.snapshots.remove(&evicted);
            return Some(evicted);
        }
        None
    }

    /// Reads `root` and promotes it to most-recently-used.
    pub fn get(&mut self, root: &str) -> Option<&RemoteGitSnapshot> {
        if self.snapshots.contains_key(root) {
            self.touch(root);
        }
        self.snapshots.get(root)
    }

    pub fn invalidate(&mut self, root: &str) {
        self.snapshots.remove(root);
        if let Some(pos) = self.order.iter().position(|r| r == root) {
            self.order.remove(pos);
        }
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.snapshots.clear();
    }
}
