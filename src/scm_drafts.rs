//! In-memory commit drafts belong to a project and path, never the visible panel alone.
use crate::git_result::Owner;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};
fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect("commit draft identity exhausted")
}
struct Draft {
    id: u64,
    revision: u64,
    text: String,
}
struct Submitted {
    owner: Owner,
    id: u64,
    revision: u64,
}
#[derive(Default)]
pub(crate) struct Drafts {
    entries: HashMap<Owner, Draft>,
    submitted: Option<Submitted>,
}
impl Drafts {
    pub fn pending(&self, owner: &Owner) -> bool {
        self.submitted.as_ref().is_some_and(|s| &s.owner == owner)
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&Owner) -> bool) {
        self.entries.retain(|owner, _| keep(owner));
        if self
            .submitted
            .as_ref()
            .is_some_and(|s| !self.entries.contains_key(&s.owner))
        {
            self.submitted = None;
        }
    }
    pub fn touch(&mut self, owner: &Owner, id: u64) {
        if let Some(d) = self.entries.get_mut(owner)
            && d.id == id
        {
            d.revision = d
                .revision
                .checked_add(1)
                .expect("commit draft revision exhausted");
        }
    }
    pub fn ids(&self) -> Vec<u64> {
        self.entries.values().map(|d| d.id).collect()
    }

    pub fn ensure(&mut self, owner: &Owner) {
        self.entries.entry(owner.clone()).or_insert_with(|| Draft {
            id: next_id(),
            revision: 0,
            text: String::new(),
        });
    }
    pub fn input(&self, owner: &Owner) -> Option<(u64, &str)> {
        self.entries.get(owner).map(|d| (d.id, d.text.as_str()))
    }
    pub fn set(&mut self, owner: &Owner, text: String) {
        self.ensure(owner);
        let draft = self.entries.get_mut(owner).unwrap();
        if draft.text != text {
            draft.text = text;
            draft.revision = draft
                .revision
                .checked_add(1)
                .expect("commit draft revision exhausted");
        }
    }
    /// Called only after a Git job was accepted for this owner.
    pub fn submitted(&mut self, owner: &Owner) {
        if let Some(d) = self.entries.get(owner) {
            self.submitted = Some(Submitted {
                owner: owner.clone(),
                id: d.id,
                revision: d.revision,
            });
        }
    }
    pub fn complete(&mut self, owner: &Owner, success: bool) {
        if !self.submitted.as_ref().is_some_and(|s| &s.owner == owner) {
            return;
        }
        let submitted = self.submitted.take().unwrap();
        if success
            && let Some(d) = self.entries.get_mut(owner)
            && d.id == submitted.id
            && d.revision == submitted.revision
        {
            d.text.clear();
            d.id = next_id();
            d.revision = 0;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn owner(project: u64, path: &str) -> Owner {
        Owner {
            project,
            root: path.into(),
        }
    }
    #[test]
    fn failure_keeps_draft_and_success_retires_only_submitted_owner() {
        let mut drafts = Drafts::default();
        let a = owner(1, "a");
        let b = owner(2, "b");
        drafts.set(&a, "first".into());
        drafts.set(&b, "second".into());
        let id = drafts.input(&a).unwrap().0;
        drafts.submitted(&a);
        drafts.complete(&a, false);
        assert_eq!(drafts.input(&a), Some((id, "first")));
        drafts.submitted(&a);
        drafts.complete(&b, true);
        assert_eq!(drafts.input(&a), Some((id, "first")));
        drafts.complete(&a, true);
        assert_ne!(drafts.input(&a).unwrap().0, id);
        assert_eq!(drafts.input(&a).unwrap().1, "");
        assert_eq!(drafts.input(&b).unwrap().1, "second");
    }
    #[test]
    fn typing_during_commit_survives_even_if_it_returns_to_same_text() {
        let mut drafts = Drafts::default();
        let a = owner(1, "a");
        drafts.set(&a, "same".into());
        let id = drafts.input(&a).unwrap().0;
        drafts.submitted(&a);
        drafts.set(&a, "changed".into());
        drafts.set(&a, "same".into());
        drafts.complete(&a, true);
        assert_eq!(drafts.input(&a), Some((id, "same")));
    }
    #[test]
    fn replacement_path_has_separate_draft_and_noop_edit_does_not_change_revision() {
        let mut drafts = Drafts::default();
        let old = owner(1, "old");
        let new = owner(1, "new");
        drafts.set(&old, "old draft".into());
        drafts.set(&new, "new draft".into());
        drafts.submitted(&old);
        drafts.set(&old, "old draft".into());
        drafts.complete(&old, true);
        assert_eq!(drafts.input(&old).unwrap().1, "");
        assert_eq!(drafts.input(&new).unwrap().1, "new draft");
    }
    #[test]
    fn removed_owner_and_unstored_native_draft_cannot_be_cleared_by_completion() {
        let mut drafts = Drafts::default();
        let a = owner(1, "a");
        drafts.set(&a, "old".into());
        let id = drafts.input(&a).unwrap().0;
        drafts.submitted(&a);
        drafts.touch(&a, id);
        drafts.complete(&a, true);
        assert_eq!(drafts.input(&a), Some((id, "old")));
        drafts.submitted(&a);
        drafts.retain(|_| false);
        drafts.set(&a, "reopened".into());
        drafts.complete(&a, true);
        assert_eq!(drafts.input(&a).unwrap().1, "reopened");
    }
}
