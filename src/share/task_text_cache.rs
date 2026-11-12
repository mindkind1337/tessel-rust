//! Host-local sanitized title cache. Never serialized or exposed to guests.
use super::{SharedTask, guest_text};
use std::path::{Path, PathBuf};

pub(super) struct CachedTaskText {
    project: u64,
    root: PathBuf,
    raw_title: String,
    raw_guest_by: Option<String>,
    pub title: String,
    pub guest_by: Option<String>,
}
impl CachedTaskText {
    pub(super) fn matches(&self, card: &SharedTask, root: &Path) -> bool {
        self.project == card.project_id
            && self.root == root
            && self.raw_title == card.title
            && self.raw_guest_by == card.guest_by
    }
    pub(super) fn new(card: &SharedTask, root: &Path) -> Self {
        let allowed_root = root.to_str().unwrap_or("");
        Self {
            project: card.project_id,
            root: root.to_owned(),
            raw_title: card.title.clone(),
            raw_guest_by: card.guest_by.clone(),
            title: guest_text::task_title(&card.title, allowed_root, 300),
            guest_by: card
                .guest_by
                .as_ref()
                .map(|name| guest_text::task_title(name, allowed_root, 40)),
        }
    }
}
