//! One cancellable, owner-local Tree subscription for the current SFTP directory.
//! Network calls and subscription teardown stay on the worker; the UI receives
//! only the latest complete bounded listing, never an unbounded queue of deltas.
use crate::remote::Entry;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};
use tessel_host_client::{
    Client, ClientError, ConnectionState,
    tree::{Tree, TreeEvent},
};
use tessel_host_protocol::{
    Name, RelPath,
    tree::{Changed, EntryKind, Listing},
};

const MAX_ENTRIES: usize = 10_000;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Key {
    pub alias: String,
    pub root: String,
    pub path: String,
}

pub(crate) enum Update {
    Listing(Vec<Entry>),
    Fallback,
}
type Mailbox = Arc<Mutex<Option<Update>>>;

struct Running {
    key: Key,
    cancel: Arc<AtomicBool>,
    mailbox: Mailbox,
    thread: JoinHandle<()>,
    ended_reported: bool,
}

#[derive(Default)]
pub(crate) struct Watch {
    desired: Option<Key>,
    running: Option<Running>,
    attempted: bool,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.invalidate();
    }
}

impl Watch {
    pub fn invalidate(&mut self) {
        self.desired = None;
        self.attempted = false;
        if let Some(running) = &self.running {
            running.cancel.store(true, Ordering::Release);
            running.mailbox.lock().unwrap().take();
        }
    }

    pub fn has_update(&self) -> bool {
        self.running.as_ref().is_some_and(|running| {
            self.desired.as_ref() == Some(&running.key)
                && (running.mailbox.lock().unwrap().is_some()
                    || (running.thread.is_finished() && !running.ended_reported))
        })
    }

    pub fn poll(&mut self, key: Option<Key>, client: Option<Client>) -> Option<Update> {
        if self.desired != key {
            self.invalidate();
            self.desired = key;
        }
        if let Some(running) = &self.running
            && running.cancel.load(Ordering::Acquire)
        {
            if !running.thread.is_finished() {
                return None;
            }
            if let Some(running) = self.running.take() {
                let _ = running.thread.join();
            }
        }
        if self.running.is_none()
            && !self.attempted
            && let (Some(key), Some(client)) = (self.desired.clone(), client)
        {
            self.attempted = true;
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = cancel.clone();
            let mailbox = Arc::new(Mutex::new(None));
            let worker_mailbox = mailbox.clone();
            let scope = key.clone();
            match std::thread::Builder::new()
                .name("remote-tree".into())
                .spawn(move || {
                    run(client, &scope, &worker_cancel, &worker_mailbox);
                }) {
                Ok(thread) => {
                    self.running = Some(Running {
                        key,
                        cancel,
                        mailbox,
                        thread,
                        ended_reported: false,
                    })
                }
                Err(_) => return Some(Update::Fallback),
            }
        }
        let running = self
            .running
            .as_mut()
            .filter(|running| self.desired.as_ref() == Some(&running.key))?;
        if running.thread.is_finished() && !running.ended_reported {
            running.ended_reported = true;
            running.mailbox.lock().unwrap().take();
            return Some(Update::Fallback);
        }
        running.mailbox.lock().unwrap().take()
    }
}

#[derive(Default)]
struct Cache {
    path: RelPath,
    revision: u64,
    entries: Option<BTreeMap<Name, tessel_host_protocol::tree::Entry>>,
}
impl Cache {
    fn listing(&mut self, listing: Listing) -> bool {
        self.entries = None;
        if listing.path != self.path
            || !listing.watched
            || listing.excluded
            || listing.truncated
            || listing.entries.len() > MAX_ENTRIES
        {
            return false;
        }
        self.revision = listing.revision;
        self.entries = Some(
            listing
                .entries
                .into_iter()
                .map(|entry| (entry.name.clone(), entry))
                .collect(),
        );
        true
    }

    fn change(&mut self, delta: Changed) -> bool {
        if delta.path != self.path || delta.revision <= self.revision {
            return false;
        }
        let Some(entries) = &mut self.entries else {
            return false;
        };
        for name in delta.removed {
            entries.remove(&name);
        }
        for entry in delta.added.into_iter().chain(delta.updated) {
            entries.insert(entry.name.clone(), entry);
        }
        self.revision = delta.revision;
        if entries.len() > MAX_ENTRIES {
            self.entries = None;
        }
        true
    }

    fn snapshot(&self, path: &str) -> Update {
        let Some(entries) = &self.entries else {
            return Update::Fallback;
        };
        let mut result = Vec::with_capacity(entries.len());
        for entry in entries.values() {
            // Labels are display-only. Lossy conversion could target a different file.
            let Ok(name) = std::str::from_utf8(entry.name.as_bytes()) else {
                return Update::Fallback;
            };
            if name.chars().any(char::is_control)
                || !matches!(entry.kind, EntryKind::File | EntryKind::Dir)
            {
                continue;
            }
            result.push(Entry {
                name: name.into(),
                path: format!("{}/{name}", path.trim_end_matches('/')),
                directory: entry.kind == EntryKind::Dir,
                size: entry.size,
            });
        }
        result.sort_by(|a, b| {
            b.directory
                .cmp(&a.directory)
                .then_with(|| a.name.cmp(&b.name))
        });
        Update::Listing(result)
    }
}

fn publish(mailbox: &Mailbox, update: Update) {
    *mailbox.lock().unwrap() = Some(update);
}

fn relative(root: &str, path: &str) -> Option<RelPath> {
    if !root.starts_with('/')
        || !path.starts_with('/')
        || root.chars().chain(path.chars()).any(char::is_control)
    {
        return None;
    }
    let root = root.trim_end_matches('/');
    let suffix = if path.trim_end_matches('/') == root {
        ""
    } else {
        path.strip_prefix(root)?.strip_prefix('/')?
    };
    RelPath::from_components(if suffix.is_empty() {
        vec![]
    } else {
        suffix
            .split('/')
            .map(|part| part.as_bytes().to_vec())
            .collect()
    })
}

struct Expansion {
    reply: std::sync::mpsc::Receiver<Result<Listing, tessel_host_client::ClientError>>,
    thread: JoinHandle<()>,
    epoch: u64,
}

fn run(client: Client, key: &Key, cancel: &AtomicBool, mailbox: &Mailbox) {
    let Some(path) = relative(&key.root, &key.path) else {
        publish(mailbox, Update::Fallback);
        return;
    };
    // Reopening the stable SFTP root reuses the daemon root instead of spending
    // one of its finite root slots for every visited directory.
    let Ok((tree, events)) = Tree::open(&client, &key.root) else {
        publish(mailbox, Update::Fallback);
        return;
    };
    let tree = Arc::new(tree);
    let mut cache = Cache {
        path: path.clone(),
        ..Default::default()
    };
    let mut pending: Option<Expansion> = None;
    let mut refresh = true;
    let mut closed = false;
    let mut epoch = 0u64;
    let mut newest_revision = 0u64;
    let mut expand_attempts = 0usize;
    let mut last_expand: Option<std::time::Instant> = None;
    let mut dirty = false;
    let mut last_publish = std::time::Instant::now();
    loop {
        if (cancel.load(Ordering::Acquire) || closed) && pending.is_none() {
            break;
        }
        if dirty && pending.is_none() && last_publish.elapsed() >= Duration::from_millis(100) {
            publish(mailbox, cache.snapshot(&key.path));
            dirty = false;
            last_publish = std::time::Instant::now();
        }
        if let Some(result) = pending.as_ref().and_then(|job| match job.reply.try_recv() {
            Ok(result) => Some(Some(result)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(None),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
        }) {
            let job = pending.take().unwrap();
            // The RPC is already finished; this join never waits on network I/O.
            let _ = job.thread.join();
            if cancel.load(Ordering::Acquire) || closed {
                break;
            }
            if job.epoch != epoch {
                refresh = cache.entries.is_none() || cache.revision < newest_revision;
            } else if let Some(Ok(listing)) = result {
                if listing.revision < newest_revision {
                    // Changes may precede the listing. Ask again instead of
                    // keeping an unbounded backlog or applying an older base.
                    refresh = true;
                } else if cache.listing(listing) {
                    expand_attempts = 0;
                    publish(mailbox, cache.snapshot(&key.path));
                } else {
                    publish(mailbox, Update::Fallback);
                    break;
                }
            } else if matches!(
                result,
                Some(Err(ClientError::ConnectionLost | ClientError::Timeout))
            ) {
                // A slow reconnect must not permanently retire this directory's
                // subscription. Retry only when connected, within the same budget.
                refresh = true;
                publish(mailbox, Update::Fallback);
            } else {
                publish(mailbox, Update::Fallback);
                break;
            }
        }
        if refresh
            && pending.is_none()
            && !cancel.load(Ordering::Acquire)
            && !closed
            && matches!(
                client.state(),
                ConnectionState::Connected { .. } | ConnectionState::Restarted { .. }
            )
            && last_expand.is_none_or(|last| last.elapsed() >= Duration::from_millis(250))
        {
            if expand_attempts >= 3 {
                publish(mailbox, Update::Fallback);
                break;
            }
            expand_attempts += 1;
            last_expand = Some(std::time::Instant::now());
            refresh = false;
            cache.entries = None;
            dirty = false;
            publish(mailbox, Update::Fallback);
            let (sender, reply) = std::sync::mpsc::sync_channel(1);
            let rpc_tree = tree.clone();
            let rpc_path = path.clone();
            let Ok(thread) = std::thread::Builder::new()
                .name("remote-tree-expand".into())
                .spawn(move || {
                    let _ = sender.send(rpc_tree.expand(&rpc_path));
                })
            else {
                publish(mailbox, Update::Fallback);
                break;
            };
            pending = Some(Expansion {
                reply,
                thread,
                epoch,
            });
        }
        if closed {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        // Keep draining while expand is blocked. Otherwise a full Tree receiver
        // can block the shared connection's reader and unrelated PTY traffic.
        let event = match events.recv_timeout(Duration::from_millis(20)) {
            Ok(event) => event,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => {
                closed = true;
                publish(mailbox, Update::Fallback);
                continue;
            }
        };
        match event {
            TreeEvent::Changed(delta) if delta.path == path => {
                newest_revision = newest_revision.max(delta.revision);
                if pending.is_none() && cache.change(delta) {
                    dirty = true;
                }
            }
            TreeEvent::Relisted(listing) if listing.path == path => {
                dirty = false;
                epoch = epoch.wrapping_add(1);
                if listing.revision < newest_revision {
                    refresh = true;
                    cache.entries = None;
                    publish(mailbox, Update::Fallback);
                } else {
                    cache.listing(listing);
                    expand_attempts = 0;
                    publish(mailbox, cache.snapshot(&key.path));
                }
            }
            TreeEvent::Disconnected => {
                dirty = false;
                epoch = epoch.wrapping_add(1);
                newest_revision = 0;
                refresh = true;
                cache.entries = None;
                publish(mailbox, Update::Fallback);
            }
            TreeEvent::Stale(stale) if path.starts_with(&stale.path) => {
                dirty = false;
                epoch = epoch.wrapping_add(1);
                refresh = true;
                cache.entries = None;
                publish(mailbox, Update::Fallback);
            }
            TreeEvent::Resynced(_) => {
                dirty = false;
                epoch = epoch.wrapping_add(1);
                refresh = true;
                cache.entries = None;
                publish(mailbox, Update::Fallback);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
