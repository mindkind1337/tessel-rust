use super::TesselGpuiRoot;
use crate::guest_file_editor::Document;
use gpui::{App, AppContext, Context, Entity, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};

struct Binding {
    document: Document,
    state: Entity<EditableTextState>,
    _changed: Subscription,
}

#[derive(Default)]
pub(super) struct Input {
    binding: Option<Binding>,
}

impl Input {
    pub(super) fn state(&self) -> Option<&Entity<EditableTextState>> {
        self.binding.as_ref().map(|b| &b.state)
    }

    pub(super) fn focused(&self, window: &Window, cx: &App) -> bool {
        self.state()
            .is_some_and(|s| s.read(cx).focus_handle(cx).is_focused(window))
    }

    pub(super) fn sync(&mut self, document: Option<Document>, cx: &mut Context<TesselGpuiRoot>) {
        let Some(doc) = document else {
            self.binding = None;
            return;
        };
        if self.binding.as_ref().is_some_and(|b| {
            b.document.key == doc.key && std::sync::Arc::ptr_eq(&b.document.text, &doc.text)
        }) {
            return;
        }
        let state = cx.new(|cx| EditableTextState::new(StringStorage::from(doc.text.as_str()), cx));
        let changed = cx.subscribe(&state, |root, state, _: &TextChanged, cx| {
            if root.editor_input.guest.state() == Some(&state) {
                flush(root, cx);
            }
        });
        self.binding = Some(Binding {
            document: doc,
            state,
            _changed: changed,
        });
    }
}

pub(super) fn flush(root: &mut TesselGpuiRoot, cx: &mut Context<TesselGpuiRoot>) {
    let Some(binding) = root.editor_input.guest.binding.as_mut() else {
        return;
    };
    let text = binding.state.read(cx).as_str();
    if text == binding.document.text.as_str() {
        return;
    }
    if root
        .app
        .gpui_edit_guest_document(&binding.document, text.to_owned())
    {
        if let Some(doc) = root.app.gpui_guest_editor() {
            binding.document = doc;
        }
    } else {
        root.editor_input.guest.binding = None;
    }
    cx.notify();
}
