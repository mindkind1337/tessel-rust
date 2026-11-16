//! Private ASCII API-key storage. GPUI only sees placeholders, including through
//! accessibility, selection copying and platform text queries. No Debug/Serialize.
use gpui::CaretSelection;
use gpui_ce_elements::editable_text::UnicodeTextStorage;
use std::{cell::RefCell, collections::VecDeque, ops::Range, rc::Rc};
use zeroize::Zeroizing;
const LIMIT: usize = 4096;
const HISTORY_LIMIT: usize = 32;
pub(crate) struct Snapshot {
    pub value: Zeroizing<String>,
    pub selection: CaretSelection,
}
struct Secret {
    active: bool,
    value: Zeroizing<String>,
    selection: CaretSelection,
    undo: VecDeque<Snapshot>,
    redo: VecDeque<Snapshot>,
    replaying: bool,
    rejected: bool,
}
impl Default for Secret {
    fn default() -> Self {
        Self {
            active: true,
            value: Zeroizing::new(String::new()),
            selection: CaretSelection::default(),
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            replaying: false,
            rejected: false,
        }
    }
}
#[derive(Clone)]
pub(crate) struct Handle(Rc<RefCell<Secret>>);
pub(crate) struct Storage {
    private: Rc<RefCell<Secret>>,
    masked: String,
    version: u16,
}
impl Storage {
    pub fn pair() -> (Self, Handle) {
        let private = Rc::new(RefCell::new(Secret::default()));
        (
            Self {
                private: private.clone(),
                masked: String::new(),
                version: 0,
            },
            Handle(private),
        )
    }
}
impl Handle {
    pub fn close(&self) {
        let mut state = self.0.borrow_mut();
        state.active = false;
        state.value = Zeroizing::new(String::new());
        state.undo.clear();
        state.redo.clear();
        state.replaying = false;
    }

    /// Copy only at submission; the returned owner zeroizes on drop.
    pub fn submission(&self) -> Zeroizing<String> {
        Zeroizing::new(self.0.borrow().value.to_string())
    }
    pub fn rejected(&self) -> bool {
        self.0.borrow().rejected
    }
    /// Call after clearing the native field, so undo cannot restore a submitted key.
    pub fn clear_history(&self) {
        let mut state = self.0.borrow_mut();
        state.undo.clear();
        state.redo.clear();
        state.rejected = false;
    }
    pub fn undo(&self) -> Option<Snapshot> {
        self.replay(false)
    }
    pub fn redo(&self) -> Option<Snapshot> {
        self.replay(true)
    }
    fn replay(&self, redo: bool) -> Option<Snapshot> {
        let mut state = self.0.borrow_mut();
        if !state.active {
            return None;
        }
        let target = if redo {
            state.redo.pop_back()
        } else {
            state.undo.pop_back()
        }?;
        let current = Snapshot {
            value: Zeroizing::new(state.value.to_string()),
            selection: state.selection,
        };
        let destination = if redo {
            &mut state.undo
        } else {
            &mut state.redo
        };
        destination.push_back(current);
        while destination.len() > HISTORY_LIMIT {
            destination.pop_front();
        }
        state.replaying = true;
        Some(target)
    }
    pub fn selection(&self, selection: CaretSelection) {
        self.0.borrow_mut().selection = selection;
    }
}
impl UnicodeTextStorage for Storage {
    fn supports_history(&self) -> bool {
        false
    }
    fn version(&self) -> u16 {
        self.version
    }
    fn content_utf8(&self) -> &str {
        &self.masked
    }
    fn len_utf16(&self) -> usize {
        self.masked.len()
    }
    fn allows_replacement(&self, range: &Range<usize>, text: &str) -> bool {
        let private = self.private.borrow();
        let allowed = private.active
            && private.value.len() == self.masked.len()
            && range.start <= range.end
            && range.end <= self.masked.len()
            && self.masked.len() - (range.end - range.start) + text.len() <= LIMIT
            && text.bytes().all(|byte| (0x20..=0x7e).contains(&byte));
        drop(private);
        self.private.borrow_mut().rejected = !allowed;
        allowed
    }
    fn before_edit(&mut self, selection: CaretSelection) {
        self.private.borrow_mut().selection = selection;
    }
    fn replace_range(&mut self, range: Range<usize>, text: &str) {
        if !self.allows_replacement(&range, text) {
            return;
        }
        let mut state = self.private.borrow_mut();
        if !state.replaying {
            let before = Snapshot {
                value: Zeroizing::new(state.value.to_string()),
                selection: state.selection,
            };
            state.undo.push_back(before);
            while state.undo.len() > HISTORY_LIMIT {
                state.undo.pop_front();
            }
            state.redo.clear();
        }
        state.replaying = false;
        // Allocate a fresh zeroizing buffer so replacement/deletion cannot leave
        // an old key suffix in spare String capacity after shrinking the value.
        let mut replacement = Zeroizing::new(String::with_capacity(
            state.value.len() - (range.end - range.start) + text.len(),
        ));
        replacement.push_str(&state.value[..range.start]);
        replacement.push_str(text);
        replacement.push_str(&state.value[range.end..]);
        state.value = replacement;
        state.selection = (range.start + text.len()..range.start + text.len()).into();
        self.masked = "*".repeat(state.value.len());
        self.version = self.version.wrapping_add(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_storage_surface_is_masked_and_edits_keep_the_private_value() {
        let (mut storage, handle) = Storage::pair();
        storage.replace_range(0..0, "synthetic-key");
        assert_eq!(storage.content_utf8(), "*************");
        assert_eq!(storage.len_utf16(), 13);
        assert_eq!(handle.submission().as_str(), "synthetic-key");
        storage.replace_range(0..9, "fixture");
        assert_eq!(handle.submission().as_str(), "fixture-key");
        assert!(!storage.supports_history());
    }
    #[test]
    fn invalid_insertions_preserve_key_and_private_history_round_trips() {
        let (mut storage, handle) = Storage::pair();
        storage.replace_range(0..0, "alpha-key");
        storage.before_edit((2..5).into());
        storage.replace_range(2..5, "XYZ");
        let changed = handle.submission();
        assert!(!storage.allows_replacement(&(0..0), "é"));
        assert!(handle.rejected());
        assert!(!storage.allows_replacement(&(0..0), &"x".repeat(LIMIT)));
        assert_eq!(handle.submission().as_str(), changed.as_str());
        let undo = handle.undo().unwrap();
        storage.replace_range(0..storage.content_utf8().len(), &undo.value);
        handle.selection(undo.selection);
        assert_eq!(handle.submission().as_str(), "alpha-key");
        let redo = handle.redo().unwrap();
        storage.replace_range(0..storage.content_utf8().len(), &redo.value);
        assert_eq!(handle.submission().as_str(), changed.as_str());
        storage.replace_range(0..storage.content_utf8().len(), "");
        handle.clear_history();
        assert!(handle.undo().is_none());
        assert!(handle.redo().is_none());
        assert!(handle.submission().is_empty());
    }
}
