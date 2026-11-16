//! Native private key field; secret text never enters a root snapshot or action.
use super::private_key_storage::{Handle, Storage};
use gpui::StatefulInteractiveElement;
use gpui::{
    App, AppContext, Entity, EntityInputHandler, Focusable, InteractiveElement, Styled, Window, px,
};
use gpui_ce_elements::editable_text::{EditableTextElement, EditableTextState, text_input};
use zeroize::Zeroizing;
gpui::actions!(tessel_private_key, [UndoKey, RedoKey, CopyKey, CutKey]);
pub(crate) struct Input {
    state: Entity<EditableTextState>,
    private: Handle,
}
impl Drop for Input {
    fn drop(&mut self) {
        self.private.close();
    }
}
impl Input {
    pub fn new(cx: &mut App) -> Self {
        let (storage, private) = Storage::pair();
        Self {
            state: cx.new(|cx| EditableTextState::new(storage, cx)),
            private,
        }
    }
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.state.read(cx).focus_handle(cx).focus(window, cx);
    }
    pub fn entity(&self) -> Entity<EditableTextState> {
        self.state.clone()
    }
    pub fn focus_handle(&self, cx: &App) -> gpui::FocusHandle {
        self.state.read(cx).focus_handle(cx)
    }
    pub fn rejected(&self) -> bool {
        self.private.rejected()
    }
    pub fn is_empty(&self, cx: &App) -> bool {
        self.state.read(cx).as_str().is_empty()
    }
    pub fn take(&self, window: &mut Window, cx: &mut App) -> Result<Zeroizing<String>, String> {
        if self.private.rejected() {
            return Err("Enter an ASCII key of at most 4096 characters".into());
        }
        if self.state.update(cx, |state, cx| {
            state.marked_text_range(window, cx).is_some()
        }) {
            return Err("Finish composing the key before connecting".into());
        }
        let secret = self.private.submission();
        self.state.update(cx, |state, cx| state.emplace("", cx));
        self.private.clear_history();
        Ok(secret)
    }
    pub fn render(
        &self,
        french: bool,
        tokens: &super::theme::GpuiThemeTokens,
    ) -> EditableTextElement {
        let undo = self.state.clone();
        let undo_private = self.private.clone();
        let redo = self.state.clone();
        let redo_private = self.private.clone();
        let cut = self.state.clone();
        text_input("private-linear-key")
            .state(self.state.downgrade())
            .key_context("EditableText TesselPrivateKey")
            .placeholder(if french {
                "Clé API personnelle Linear"
            } else {
                "Linear personal API key"
            })
            .aria_label(if french {
                "Clé API Linear, saisie masquée"
            } else {
                "Linear API key, hidden input"
            })
            .on_action(move |_: &UndoKey, w, cx| replay(&undo, &undo_private, false, w, cx))
            .on_action(move |_: &RedoKey, w, cx| replay(&redo, &redo_private, true, w, cx))
            .on_action(|_: &CopyKey, _, _| {})
            .on_action(move |_: &CutKey, w, cx| {
                cut.update(cx, |state, cx| {
                    if state
                        .selected_text_range(false, w, cx)
                        .is_some_and(|selection| !selection.range.is_empty())
                    {
                        state.replace_text_in_range(None, "", w, cx);
                    }
                });
            })
            .w_full()
            .h(px(30.))
            .px(px(8.))
            .py(px(4.))
            .text_size(px(12.))
            .text_color(tokens.text)
    }
}
fn replay(
    state: &Entity<EditableTextState>,
    private: &Handle,
    redo: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let selection = state.update(cx, |state, cx| {
        if state.marked_text_range(window, cx).is_some() {
            return None;
        }
        state.selected_text_range(false, window, cx)
    });
    let Some(selection) = selection else {
        return;
    };
    let range = if selection.reversed {
        selection.range.end..selection.range.start
    } else {
        selection.range
    };
    private.selection(range.into());
    let Some(target) = (if redo { private.redo() } else { private.undo() }) else {
        return;
    };
    state.update(cx, |state, cx| {
        state.emplace(&target.value, cx);
        state.set_selected_text_range(
            target.selection.anchor.index..target.selection.caret.index,
            window,
            cx,
        );
    });
    private.selection(target.selection);
}
pub fn register(cx: &mut App) {
    cx.bind_keys(
        gpui::ActionBindingCollection::default()
            .with::<UndoKey>("secondary-z")
            .with::<RedoKey>("secondary-shift-z")
            .with::<CopyKey>("secondary-c")
            .with::<CutKey>("secondary-x")
            .as_keybindings(Some("TesselPrivateKey")),
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, ParentElement, Render, TestAppContext, div};
    struct Fixture {
        input: Input,
    }
    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
            let _ = cx;
            div().w(px(300.)).child(self.input.render(
                false,
                &super::super::theme::GpuiThemeTokens::from_theme(
                    crate::theme_palettes::ThemeId::default(),
                ),
            ))
        }
    }
    #[gpui::test]
    fn private_key_native_queries_history_and_ime_never_expose_secret(cx: &mut TestAppContext) {
        let view = cx.add_window(|_, cx| Fixture {
            input: Input::new(cx),
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |fixture, window, cx| {
            fixture.input.state.update(cx, |state, cx| {
                state.replace_text_in_range(None, "synthetic-key", window, cx);
                assert_eq!(state.as_str(), "*************");
                assert!(state.history().is_none());
                let mut adjusted = None;
                assert_eq!(
                    state
                        .text_for_range(0..13, &mut adjusted, window, cx)
                        .unwrap(),
                    "*************"
                );
                let before = state.selected_text_range(false, window, cx).unwrap().range;
                state.replace_and_mark_text_in_range(None, "é", None, window, cx);
                assert!(state.marked_text_range(window, cx).is_none());
                assert_eq!(
                    state.selected_text_range(false, window, cx).unwrap().range,
                    before
                );
                state.set_selected_text_range(0..9, window, cx);
                state.replace_text_in_range(None, "fixture", window, cx);
            });
            replay(
                &fixture.input.state,
                &fixture.input.private,
                false,
                window,
                cx,
            );
            assert_eq!(fixture.input.private.submission().as_str(), "synthetic-key");
            replay(
                &fixture.input.state,
                &fixture.input.private,
                true,
                window,
                cx,
            );
            assert_eq!(fixture.input.private.submission().as_str(), "fixture-key");
            assert_eq!(
                fixture.input.take(window, cx).unwrap().as_str(),
                "fixture-key"
            );
            assert!(fixture.input.state.read(cx).as_str().is_empty());
            assert!(fixture.input.private.undo().is_none());
        })
        .unwrap();
    }
    #[gpui::test]
    fn private_key_closed_field_rejects_late_input_and_erases_history(cx: &mut TestAppContext) {
        let view = cx.add_window(|_, cx| Fixture {
            input: Input::new(cx),
        });
        view.update(cx, |fixture, window, cx| {
            fixture.input.state.update(cx, |state, cx| {
                state.replace_text_in_range(None, "synthetic-key", window, cx)
            });
            let state = fixture.input.state.clone();
            let handle = fixture.input.private.clone();
            let old = std::mem::replace(&mut fixture.input, Input::new(cx));
            drop(old);
            state.update(cx, |state, cx| {
                state.replace_text_in_range(None, "late", window, cx)
            });
            assert!(handle.submission().is_empty());
            assert!(handle.undo().is_none());
            assert!(handle.rejected());
        })
        .unwrap();
    }

    #[gpui::test]
    fn private_key_keyboard_copy_cut_and_history_keep_clipboard_private(cx: &mut TestAppContext) {
        cx.update(|cx| {
            use gpui_ce_elements::editable_text::actions::{
                DEFAULT_INPUT_CONTEXT, default_bindings,
            };
            cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
            register(cx);
            cx.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".into()));
        });
        let view = cx.add_window(|_, cx| Fixture {
            input: Input::new(cx),
        });
        view.update(cx, |fixture, window, cx| {
            fixture.input.focus(window, cx);
            fixture.input.state.update(cx, |state, cx| {
                state.replace_text_in_range(None, "synthetic-key", window, cx)
            });
        })
        .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(view.into(), "ctrl-a ctrl-c");
        assert_eq!(
            cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
                .as_deref(),
            Some("sentinel")
        );
        cx.simulate_keystrokes(view.into(), "ctrl-x");
        view.update(cx, |fixture, _, _| {
            assert!(fixture.input.private.submission().is_empty())
        })
        .unwrap();
        assert_eq!(
            cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
                .as_deref(),
            Some("sentinel")
        );
        cx.simulate_keystrokes(view.into(), "ctrl-z");
        view.update(cx, |fixture, _, _| {
            assert_eq!(fixture.input.private.submission().as_str(), "synthetic-key")
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-shift-z");
        view.update(cx, |fixture, _, _| {
            assert!(fixture.input.private.submission().is_empty())
        })
        .unwrap();
    }
}
