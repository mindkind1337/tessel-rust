//! Native drafts are scoped to a live provider instance, never just a pane number.
use super::{FocusInputTarget, TesselGpuiRoot};
use crate::app::gpui_bridge::{GpuiAction, GpuiWorkspaceSnapshot};
use gpui::{App, AppContext, Context, Entity, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
use std::collections::HashMap;

gpui::actions!(tessel_chat_composer, [Submit, Interrupt]);

pub(super) struct Transcript {
    pub handle: gpui::ScrollHandle,
    following: std::cell::Cell<bool>,
    previous: std::cell::Cell<(gpui::Pixels, gpui::Pixels)>,
}

impl Default for Transcript {
    fn default() -> Self {
        Self {
            handle: Default::default(),
            following: true.into(),
            previous: Default::default(),
        }
    }
}

impl Transcript {
    fn geometry(&self) -> (gpui::Pixels, gpui::Pixels) {
        (self.handle.max_offset().y, self.handle.offset().y)
    }
    pub fn jump(&self) {
        self.following.set(true);
        self.previous.set(self.geometry());
        self.handle.scroll_to_bottom();
    }
    pub fn align_reply(&self, index: usize) {
        self.following.set(false);
        self.previous.set(self.geometry());
        self.handle.scroll_to_top_of_item(index);
    }
    pub fn wheel(&self, delta: gpui::Pixels) {
        let current = self.geometry();
        self.previous.set(current);
        if delta > gpui::px(0.) && current.0 > gpui::px(0.) {
            self.following.set(false);
        }
    }
    pub fn prepare(&self) -> bool {
        let current = self.geometry();
        let prior = self.previous.replace(current);
        let distance = current.0 + current.1;
        // An offset change at the same content size comes from scrolling.
        // Extra streamed lines or a resize change the extent instead.
        if current.0 == prior.0 && current.1 != prior.1 {
            self.following
                .set(distance <= gpui::px(4.) && (self.following.get() || current.1 < prior.1));
        }
        if self.following.get() {
            self.handle.scroll_to_bottom();
        }
        !self.following.get() && distance > gpui::px(48.)
    }
}

struct Binding {
    transcript: std::rc::Rc<Transcript>,
    id: u64,
    state: Entity<EditableTextState>,
    synced: String,
    too_long: bool,
    _changed: Subscription,
}

#[derive(Default)]
pub(crate) struct Inputs {
    pub(super) copied_message: Option<(u64, usize, std::time::Instant)>,
    pub questions: super::question_inputs::Inputs,
    bindings: HashMap<u64, Binding>,
}

impl Inputs {
    pub fn focused(&self, window: &Window, cx: &App) -> Option<FocusInputTarget> {
        self.questions.focused(window, cx).or_else(|| {
            self.bindings.iter().find_map(|(pane, binding)| {
                binding
                    .state
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
                    .then_some(FocusInputTarget::ChatComposer(*pane))
            })
        })
    }

    pub fn input_for_target(&self, target: FocusInputTarget) -> Option<&Entity<EditableTextState>> {
        match target {
            FocusInputTarget::ChatComposer(pane) => self.get(pane),
            FocusInputTarget::ChatQuestion { pane_id, q_idx } => self.questions.get(pane_id, q_idx),
            _ => None,
        }
    }

    pub(super) fn transcript(&self, pane: u64) -> Option<&std::rc::Rc<Transcript>> {
        self.bindings.get(&pane).map(|b| &b.transcript)
    }

    pub fn get(&self, pane: u64) -> Option<&Entity<EditableTextState>> {
        self.bindings.get(&pane).map(|b| &b.state)
    }

    pub fn sync(
        &mut self,
        snapshot: &GpuiWorkspaceSnapshot,
        open: &[(u64, u64)],
        questions: &[(u64, u64)],
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        self.questions.sync(snapshot, questions, cx);
        self.bindings
            .retain(|pane, b| open.contains(&(*pane, b.id)));
        for tile in &snapshot.tiles {
            let Some(chat) = &tile.native_chat else {
                continue;
            };
            let pane = tile.session_id;
            let id = chat.composer_id;
            if self.bindings.get(&pane).is_some_and(|b| {
                b.id == id
                    && (b.state.read(cx).as_str() == chat.composer
                        || (b.too_long && b.synced == chat.composer))
            }) {
                continue;
            }
            // A successful send or external replacement retires the old undo history.
            let state = cx
                .new(|cx| EditableTextState::new(StringStorage::from(chat.composer.as_str()), cx));
            let changed = cx.subscribe(&state, move |root, state, _: &TextChanged, cx| {
                let Some(b) = root.chat_inputs.bindings.get(&pane) else {
                    return;
                };
                if !root.app.gpui_chat_input_allowed(pane)
                    || b.id != id
                    || b.state != state
                    || root.effective_focus_target() != FocusInputTarget::ChatComposer(pane)
                {
                    return;
                }
                let base = b.synced.clone();
                let too_long = !crate::chat::message_fits(state.read(cx).as_str());
                if let Some(b) = root.chat_inputs.bindings.get_mut(&pane) {
                    b.too_long = too_long;
                }
                if too_long {
                    cx.notify();
                    return;
                }
                let text = state.read(cx).as_str().to_owned();
                if base == text {
                    return;
                }
                root.dispatch(
                    GpuiAction::ApplyNativeChatEdit {
                        pane_id: pane,
                        instance_id: id,
                        base,
                        text,
                    },
                    cx,
                );
                if let Some((current_id, text)) = root.app.gpui_chat_composer(pane)
                    && current_id == id
                    && let Some(b) = root.chat_inputs.bindings.get_mut(&pane)
                {
                    b.synced = text.to_owned();
                }
            });
            let transcript = self.transcript(pane).cloned().unwrap_or_default();
            self.bindings.insert(
                pane,
                Binding {
                    transcript,
                    id,
                    state,
                    synced: chat.composer.clone(),
                    too_long: false,
                    _changed: changed,
                },
            );
        }
    }

    pub fn render(
        &self,
        pane: u64,
        can_interrupt: bool,
        font_size: f32,
        french: bool,
        tokens: &super::theme::GpuiThemeTokens,
        cx: &mut Context<TesselGpuiRoot>,
    ) -> gpui::AnyElement {
        use gpui::{
            InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
            div, prelude::FluentBuilder, px,
        };
        use gpui_ce_elements::editable_text::text_area;
        let Some(binding) = self.bindings.get(&pane) else {
            return gpui::div().into_any_element();
        };
        let placeholder = if french {
            "Écrire un message à l'agent… (Entrée = envoyer, Maj+Entrée = nouvelle ligne)"
        } else {
            "Ask the agent… (Enter to send, Shift+Enter for newline)"
        };

        let expected_id = binding.id;
        let area = text_area(("native-chat-composer", pane))
            .state(binding.state.downgrade())
            .placeholder(placeholder)
            .key_context(if can_interrupt {
                "EditableText TesselChatComposer TesselChatWorking"
            } else {
                "EditableText TesselChatComposer"
            })
            .on_action(cx.listener(move |root, _: &Interrupt, window, cx| {
                use gpui::EntityInputHandler;
                if root.app.gpui_chat_can_interrupt(pane)
                    && root
                        .app
                        .gpui_chat_composer(pane)
                        .is_some_and(|(id, _)| id == expected_id)
                    && let Some(input) = root.chat_inputs.get(pane).cloned()
                    && input.update(cx, |s, cx| s.marked_text_range(window, cx).is_none())
                {
                    root.dispatch(GpuiAction::ChatInterrupt(pane), cx);
                }
            }))
            .on_action(cx.listener(move |root, _: &Submit, window, cx| {
                submit(root, pane, expected_id, window, cx)
            }))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |root, _, _, cx| {
                    root.focus_input = FocusInputTarget::ChatComposer(pane);
                    root.dispatch(GpuiAction::FocusSession(pane), cx);
                }),
            )
            .w_full()
            .min_h(px(44.))
            .max_h(px(120.))
            .overflow_y_scroll()
            .px(px(10.))
            .py(px(7.))
            .text_size(px(font_size - 0.5))
            .text_color(tokens.text_strong);
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(area)
            .when(binding.too_long, |el| {
                el.child(
                    div()
                        .px(px(10.))
                        .pb(px(7.))
                        .text_size(px(11.))
                        .text_color(tokens.warn)
                        .child(if french {
                            "Le message dépasse 100 000 unités UTF-16. Raccourcissez le brouillon."
                        } else {
                            crate::chat::MESSAGE_TOO_LONG
                        }),
                )
            })
            .into_any_element()
    }
}

pub(crate) fn submit(
    root: &mut TesselGpuiRoot,
    pane: u64,
    expected_id: u64,
    window: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    use gpui::EntityInputHandler;
    if !root.app.gpui_chat_input_allowed(pane) {
        return;
    }
    let Some(b) = root.chat_inputs.bindings.get(&pane) else {
        return;
    };
    let id = b.id;
    if id != expected_id {
        return;
    }
    let state = b.state.clone();
    let base = b.synced.clone();
    if state.update(cx, |s, cx| s.marked_text_range(window, cx).is_some()) {
        return;
    }
    if !crate::chat::message_fits(state.read(cx).as_str()) {
        if let Some(b) = root.chat_inputs.bindings.get_mut(&pane) {
            b.too_long = true;
        }
        cx.notify();
        return;
    }
    let text = state.read(cx).as_str().to_owned();
    // Flush native edits before sending, even if TextChanged is still queued.
    root.dispatch(
        GpuiAction::ApplyNativeChatEdit {
            pane_id: pane,
            instance_id: id,
            base,
            text: text.clone(),
        },
        cx,
    );
    if let Some((current_id, current)) = root.app.gpui_chat_composer(pane)
        && current_id == id
        && let Some(b) = root.chat_inputs.bindings.get_mut(&pane)
    {
        b.synced = current.to_owned();
    }
    root.dispatch(
        GpuiAction::NativeChatSend {
            pane_id: pane,
            instance_id: id,
            text,
        },
        cx,
    );
}

#[cfg(all(test, feature = "visual-test"))]
mod tests {
    use super::*;
    use gpui::{EntityInputHandler, TestAppContext};

    fn register(cx: &mut TestAppContext) {
        cx.update(|cx| {
            use gpui_ce_elements::editable_text::actions::{
                DEFAULT_INPUT_CONTEXT, default_bindings,
            };
            cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
        });
    }

    #[gpui::test]
    fn arrow_navigation_moves_keyboard_focus_between_chat_and_shell(cx: &mut TestAppContext) {
        register(cx);
        let temp = tempfile::tempdir().unwrap();
        let mut shell = None;
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(temp.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(temp.path().to_owned()), cx);
            shell = Some(root.app.install_terminal_find_fixture());
            root.dispatch(GpuiAction::LaunchClaudeChat, cx);
            root
        });
        let (id, input) = shell.unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(matches!(
                root.chat_inputs.focused(window, cx),
                Some(FocusInputTarget::ChatComposer(_))
            ))
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "alt-left x");
        assert_eq!(input.try_recv().unwrap(), b"x");
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app.gpui_default_focus_target(),
                FocusInputTarget::Terminal(Some(id))
            )
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "alt-right");
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(matches!(
                root.chat_inputs.focused(window, cx),
                Some(FocusInputTarget::ChatComposer(_))
            ))
        })
        .unwrap();
        assert!(input.try_recv().is_err());
    }

    #[gpui::test]
    fn native_chat_cursor_selection_newline_offline_and_ime(cx: &mut TestAppContext) {
        register(cx);
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app
                .apply_gpui_action(GpuiAction::AddProjectPath(project));
            root.app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
            root
        });
        cx.refresh().unwrap();
        let pane = view
            .update(cx, |root, window, cx| {
                let pane = root.app.gpui_snapshot(960., 640., 4.).tiles[0].session_id;
                assert_eq!(
                    root.chat_inputs.focused(window, cx),
                    Some(FocusInputTarget::ChatComposer(pane))
                );
                root.chat_inputs.get(pane).unwrap().update(cx, |s, cx| {
                    s.replace_text_in_range(None, "A😀Z", window, cx);
                });
                pane
            })
            .unwrap();
        cx.run_until_parked();
        view.update(cx, |root, window, cx| {
            assert_eq!(root.app.gpui_chat_composer(pane).unwrap().1, "A😀Z");
            root.chat_inputs.get(pane).unwrap().update(cx, |s, cx| {
                s.set_selected_text_range(1..3, window, cx);
                s.paste(gpui::ClipboardItem::new_string("é".into()), window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(view.into(), "shift-enter");
        view.update(cx, |root, _, _| {
            assert_eq!(root.app.gpui_chat_composer(pane).unwrap().1, "Aé\nZ");
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "enter");
        view.update(cx, |root, window, cx| {
            // No transport: send must retain the draft, never insert another newline.
            assert_eq!(root.app.gpui_chat_composer(pane).unwrap().1, "Aé\nZ");
            let input = root.chat_inputs.get(pane).unwrap().clone();
            input.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(Some(0..1), "日", None, window, cx);
            });
            let error = root.app.gpui_snapshot(960., 640., 4.).tiles[0]
                .native_chat
                .as_ref()
                .unwrap()
                .error
                .clone();
            let expected_id = root.app.gpui_chat_composer(pane).unwrap().0;
            submit(root, pane, expected_id, window, cx);
            assert_eq!(
                root.app.gpui_snapshot(960., 640., 4.).tiles[0]
                    .native_chat
                    .as_ref()
                    .unwrap()
                    .error,
                error
            );
            input.update(cx, |s, cx| {
                assert!(s.marked_text_range(window, cx).is_some());
                s.replace_text_in_range(None, "日本", window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert_eq!(root.app.gpui_chat_composer(pane).unwrap().1, "日本é\nZ");
            assert!(!root.app.gpui_handle_text_target_key(
                FocusInputTarget::ChatComposer(pane),
                "x",
                Some("x"),
                false,
                None
            ));
        })
        .unwrap();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Redo);
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert_eq!(root.app.gpui_chat_composer(pane).unwrap().1, "日本é\nZ")
        })
        .unwrap();
    }

    #[gpui::test]
    fn native_chat_submit_flushes_once_and_retires_sent_history(cx: &mut TestAppContext) {
        register(cx);
        for launch in [GpuiAction::LaunchCodexChat, GpuiAction::LaunchClaudeChat] {
            let dir = tempfile::tempdir().unwrap();
            let project = dir.path().join("project");
            std::fs::create_dir(&project).unwrap();
            let view = cx.add_window(|_, cx| {
                let mut root =
                    TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
                root.app
                    .apply_gpui_action(GpuiAction::AddProjectPath(project));
                root.app.apply_gpui_action(launch);
                root
            });
            cx.refresh().unwrap();
            let (pane, old, old_id, outgoing) = view
                .update(cx, |root, window, cx| {
                    let pane = root.app.gpui_snapshot(960., 640., 4.).tiles[0].session_id;
                    let outgoing = root.app.gpui_chat_sender_for_test(pane);
                    let old_id = root.app.gpui_chat_composer(pane).unwrap().0;
                    let old = root.chat_inputs.get(pane).unwrap().clone();
                    old.update(cx, |s, cx| {
                        s.replace_text_in_range(None, "É😀 original\nsecond line", window, cx)
                    });
                    // Do not drain TextChanged first: Submit itself must flush the native draft.
                    submit(root, pane, old_id, window, cx);
                    assert_eq!(root.app.gpui_chat_composer(pane).unwrap().1, "");
                    assert_ne!(root.app.gpui_chat_composer(pane).unwrap().0, old_id);
                    (pane, old, old_id, outgoing)
                })
                .unwrap();
            let sent = outgoing.try_recv().unwrap();
            let text = sent
                .pointer("/params/input/0/text")
                .or_else(|| sent.pointer("/message/content/0/text"))
                .unwrap()
                .as_str()
                .unwrap();
            assert_eq!(text, "É😀 original\nsecond line");
            assert!(outgoing.try_recv().is_err());
            cx.run_until_parked();
            cx.refresh().unwrap();
            view.update(cx, |root, window, cx| {
                assert_ne!(root.chat_inputs.get(pane), Some(&old));
                old.update(cx, |s, cx| {
                    s.replace_text_in_range(None, "late old event", window, cx)
                });
                submit(root, pane, old_id, window, cx);
                assert!(outgoing.try_recv().is_err());
            })
            .unwrap();
            cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
            cx.run_until_parked();
            view.update(cx, |root, _, _| {
                assert_eq!(root.app.gpui_chat_composer(pane).unwrap().1, "")
            })
            .unwrap();
        }
    }

    #[gpui::test]
    fn focused_escape_interrupts_the_provider_and_preserves_the_draft(cx: &mut TestAppContext) {
        use serde_json::json;
        register(cx);
        for claude in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let view = cx.add_window(|_, cx| {
                let mut root =
                    TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
                root.app.synthetic_native_profiles(dir.path());
                root.app
                    .apply_gpui_action(GpuiAction::AddProjectPath(dir.path().to_owned()));
                root.app.apply_gpui_action(if claude {
                    GpuiAction::LaunchClaudeChat
                } else {
                    GpuiAction::LaunchCodexChat
                });
                root
            });
            cx.refresh().unwrap();
            let (pane, outgoing) = view.update(cx, |root,window,cx| {
                let pane = root.app.gpui_snapshot(960.,640.,4.).tiles[0].session_id;
                let outgoing = root.app.gpui_chat_sender_for_test(pane);
                assert!(!root.app.gpui_chat_can_interrupt(pane));
                let id = root.app.gpui_chat_composer(pane).unwrap().0;
                root.chat_inputs.get(pane).unwrap().update(cx,|s,cx|s.replace_text_in_range(None,"start",window,cx));
                submit(root,pane,id,window,cx);
                outgoing.try_recv().unwrap();
                if !claude {
                    root.app.inject_codex_frame_for_test(pane,json!({"method":"turn/started","params":{"threadId":"synthetic-thread","turn":{"id":"turn-1"}}}));
                }
                assert!(root.app.gpui_chat_can_interrupt(pane));
                (pane,outgoing)
            }).unwrap();
            cx.refresh().unwrap();
            view.update(cx, |root, window, cx| {
                let input = root.chat_inputs.get(pane).unwrap().clone();
                input.read(cx).focus_handle(cx).focus(window, cx);
                input.update(cx, |s, cx| {
                    s.replace_and_mark_text_in_range(None, "draft", None, window, cx);
                });
            })
            .unwrap();
            cx.run_until_parked();
            cx.simulate_keystrokes(view.into(), "escape");
            assert!(
                outgoing.try_recv().is_err(),
                "IME composition must not interrupt"
            );
            view.update(cx, |root, window, cx| {
                root.chat_inputs.get(pane).unwrap().update(cx, |s, cx| {
                    s.replace_text_in_range(None, "next question", window, cx);
                });
            })
            .unwrap();
            cx.run_until_parked();
            cx.simulate_keystrokes(view.into(), "escape");
            let request = outgoing
                .try_recv()
                .expect("Escape reaches the active provider");
            if claude {
                assert_eq!(
                    request.pointer("/request/subtype"),
                    Some(&json!("interrupt"))
                );
            } else {
                assert_eq!(request["method"], "turn/interrupt");
                assert_eq!(request["params"]["turnId"], "turn-1");
            }
            view.update(cx, |root, _, _| {
                assert_eq!(
                    root.app.gpui_chat_composer(pane).unwrap().1,
                    "next question"
                )
            })
            .unwrap();
            assert!(outgoing.try_recv().is_err());
        }
    }

    #[gpui::test]
    fn native_chat_oversized_paste_is_retained_until_shortened(cx: &mut TestAppContext) {
        register(cx);
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app
                .apply_gpui_action(GpuiAction::AddProjectPath(project));
            root.app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
            root
        });
        cx.refresh().unwrap();
        let (pane, input) = view
            .update(cx, |root, window, cx| {
                let pane = root.app.gpui_snapshot(960., 640., 4.).tiles[0].session_id;
                let input = root.chat_inputs.get(pane).unwrap().clone();
                input.update(cx, |s, cx| {
                    s.replace_text_in_range(
                        None,
                        &"x".repeat(crate::chat::MESSAGE_UNITS + 1),
                        window,
                        cx,
                    )
                });
                (pane, input)
            })
            .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert_eq!(root.chat_inputs.get(pane), Some(&input));
            assert!(root.chat_inputs.bindings[&pane].too_long);
            assert_eq!(
                input.read(cx).as_str().len(),
                crate::chat::MESSAGE_UNITS + 1
            );
            assert_eq!(root.app.gpui_chat_composer(pane).unwrap().1, "");
            input.update(cx, |s, cx| {
                s.replace_text_in_range(
                    Some(0..crate::chat::MESSAGE_UNITS + 1),
                    "shortened",
                    window,
                    cx,
                )
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            assert_eq!(root.chat_inputs.get(pane), Some(&input));
            assert!(!root.chat_inputs.bindings[&pane].too_long);
            assert_eq!(root.app.gpui_chat_composer(pane).unwrap().1, "shortened");
        })
        .unwrap();
    }

    #[gpui::test]
    fn native_chat_retires_entities_on_conversation_reset_and_blurs_floating(
        cx: &mut TestAppContext,
    ) {
        register(cx);
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app
                .apply_gpui_action(GpuiAction::AddProjectPath(project));
            root.app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
            root
        });
        cx.refresh().unwrap();
        let (pane, old, id) = view
            .update(cx, |root, window, cx| {
                let pane = root.app.gpui_snapshot(960., 640., 4.).tiles[0].session_id;
                let old = root.chat_inputs.get(pane).unwrap().clone();
                let id = root.app.gpui_chat_composer(pane).unwrap().0;
                old.update(cx, |s, cx| {
                    s.replace_text_in_range(None, "kept", window, cx)
                });
                (pane, old, id)
            })
            .unwrap();
        cx.run_until_parked();
        view.update(cx, |root, _, cx| {
            root.dispatch(GpuiAction::ChatNewConversation(pane), cx)
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert_ne!(root.app.gpui_chat_composer(pane).unwrap().0, id);
            assert_ne!(root.chat_inputs.get(pane).unwrap(), &old);
            old.update(cx, |s, cx| {
                s.replace_text_in_range(None, "stale", window, cx)
            });
            root.app.apply_gpui_action(GpuiAction::ApplyNativeChatEdit {
                pane_id: pane,
                instance_id: id,
                base: "kept".into(),
                text: "stale".into(),
            });
            assert_eq!(root.app.gpui_chat_composer(pane).unwrap().1, "kept");
            root.focus_input = FocusInputTarget::None;
            root.app.set_floating_input_fixture(true);
            cx.notify();
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(root.chat_inputs.focused(window, cx).is_none())
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "x");
        view.update(cx, |root, _, _| {
            assert_eq!(root.app.gpui_chat_composer(pane).unwrap().1, "kept")
        })
        .unwrap();
    }
}
