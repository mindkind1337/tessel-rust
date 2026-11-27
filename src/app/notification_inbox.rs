use crate::dialogs_toolbar_model::{MAX_NOTIFICATIONS, NotificationCenter, NotificationKind};
use crate::workspace::{Store, atomic_write};
use std::io::Read;

// Like the Vue inbox, storage is best effort. A full disk must not prevent a
// pending question from appearing or being answered in the current window.
const MAX_BYTES: u64 = 5 * 1024 * 1024;
const FILE: &str = "notifications.json";

pub(super) fn load(store: Option<&Store>) -> NotificationCenter {
    let Some(store) = store else {
        return NotificationCenter::default();
    };
    let load_file = || {
        let mut bytes = Vec::new();
        std::fs::File::open(store.directory().join(FILE))
            .ok()?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_BYTES {
            return None;
        }
        let mut inbox: NotificationCenter = serde_json::from_slice(&bytes).ok()?;
        let excess = inbox.items.len().saturating_sub(MAX_NOTIFICATIONS);
        inbox.items.drain(..excess);
        inbox.next_id = inbox
            .items
            .iter()
            .map(|n| n.id)
            .max()
            .unwrap_or(0)
            .max(inbox.next_id);
        Some(inbox)
    };
    load_file().unwrap_or_default()
}

impl super::Tessel {
    pub(super) fn persist_notifications(&self) {
        if let Some(store) = &self.store
            && let Ok(bytes) = serde_json::to_vec(&self.notifications)
            && bytes.len() as u64 <= MAX_BYTES
        {
            let _ = atomic_write(&store.directory().join(FILE), &bytes);
        }
    }

    pub(super) fn push_notification(
        &mut self,
        kind: NotificationKind,
        title: impl Into<String>,
        detail: impl Into<String>,
        pane: Option<u64>,
    ) {
        self.notifications.push_coalesced(kind, title, detail, pane);
        self.persist_notifications();
    }

    pub(super) fn set_notification_unread(&mut self, id: u64, unread: bool) {
        if let Some(item) = self
            .notifications
            .items
            .iter_mut()
            .find(|item| item.id == id)
            && item.unread != unread
        {
            item.unread = unread;
            self.persist_notifications();
        }
    }

    pub(super) fn read_pane_notifications(&mut self, pane: u64) {
        let mut changed = false;
        for item in &mut self.notifications.items {
            if item.pane_id == Some(pane) && item.unread {
                item.unread = false;
                changed = true;
            }
        }
        if changed {
            self.persist_notifications();
        }
    }

    pub(super) fn clear_notifications(&mut self) {
        self.notifications.clear();
        self.persist_notifications();
    }
}
