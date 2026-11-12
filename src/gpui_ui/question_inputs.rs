//! Free-text answers retain native selection/undo, scoped to the displayed request.
use super::{FocusInputTarget, TesselGpuiRoot};
use crate::app::gpui_bridge::{GpuiAction, GpuiWorkspaceSnapshot};
use gpui::{App, AppContext, Context, Entity, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
use std::collections::HashMap;

gpui::actions!(tessel_chat_questions, [SubmitAnswers]);
struct Binding {
    id: u64,
    state: Entity<EditableTextState>,
    synced: String,
    too_long: bool,
    _changed: Subscription,
}
#[derive(Default)]
pub(crate) struct Inputs {
    bindings: HashMap<(u64, usize), Binding>,
}
fn oversized(text: &str) -> bool {
    text.chars().take(4097).count() > 4096
}
impl Inputs {
    pub fn focused(&self, window: &Window, cx: &App) -> Option<FocusInputTarget> {
        self.bindings.iter().find_map(|(&(pane_id, q_idx), b)| {
            b.state
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
                .then_some(FocusInputTarget::ChatQuestion { pane_id, q_idx })
        })
    }
    pub fn get(&self, pane: u64, index: usize) -> Option<&Entity<EditableTextState>> {
        self.bindings.get(&(pane, index)).map(|b| &b.state)
    }
    pub fn sync(
        &mut self,
        snapshot: &GpuiWorkspaceSnapshot,
        open: &[(u64, u64)],
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        self.bindings
            .retain(|(pane, _), b| open.contains(&(*pane, b.id)));
        for tile in &snapshot.tiles {
            let Some(chat) = &tile.native_chat else {
                continue;
            };
            for (index, q) in chat
                .questions
                .iter()
                .enumerate()
                .filter(|(_, q)| q.options.is_empty() || q.is_other)
            {
                let pane = tile.session_id;
                let id = q.request_instance_id;
                if self.bindings.get(&(pane, index)).is_some_and(|b| {
                    b.id == id
                        && (b.state.read(cx).as_str() == q.text
                            || (b.too_long && b.synced == q.text))
                }) {
                    continue;
                }
                let state =
                    cx.new(|cx| EditableTextState::new(StringStorage::from(q.text.as_str()), cx));
                let changed = cx.subscribe(&state, move |root, state, _: &TextChanged, cx| {
                    let Some(b) = root.chat_inputs.questions.bindings.get(&(pane, index)) else {
                        return;
                    };
                    if !root.app.gpui_chat_input_allowed(pane)
                        || b.id != id
                        || b.state != state
                        || root.effective_focus_target()
                            != (FocusInputTarget::ChatQuestion {
                                pane_id: pane,
                                q_idx: index,
                            })
                    {
                        return;
                    }
                    let base = b.synced.clone();
                    let too_long = oversized(state.read(cx).as_str());
                    if let Some(b) = root.chat_inputs.questions.bindings.get_mut(&(pane, index)) {
                        b.too_long = too_long;
                    }
                    if too_long {
                        cx.notify();
                        return;
                    }
                    let text = state.read(cx).as_str().to_owned();
                    if text == base {
                        return;
                    }
                    root.dispatch(
                        GpuiAction::ApplyNativeQuestionEdit {
                            pane_id: pane,
                            request_instance_id: id,
                            q_idx: index,
                            base,
                            text,
                        },
                        cx,
                    );
                    acknowledge(root, pane, index, id);
                });
                self.bindings.insert(
                    (pane, index),
                    Binding {
                        id,
                        state,
                        synced: q.text.clone(),
                        too_long: false,
                        _changed: changed,
                    },
                );
            }
        }
    }
    pub fn render(
        &self,
        pane: u64,
        index: usize,
        french: bool,
        tokens: &super::theme::GpuiThemeTokens,
        cx: &mut Context<TesselGpuiRoot>,
    ) -> gpui::AnyElement {
        use gpui::{
            InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
            div, prelude::FluentBuilder, px,
        };
        let Some(b) = self.bindings.get(&(pane, index)) else {
            return div().into_any_element();
        };
        let id = b.id;
        let input = gpui_ce_elements::editable_text::text_area(gpui::SharedString::from(format!(
            "native-question-{pane}-{index}"
        )))
        .state(b.state.downgrade())
        .placeholder(if french {
            "Votre réponse libre…"
        } else {
            "Type your answer…"
        })
        .key_context("EditableText TesselChatQuestion")
        .on_action(cx.listener(move |root, _: &SubmitAnswers, window, cx| {
            submit(root, pane, id, false, window, cx)
        }))
        .on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(move |root, _, _, cx| {
                root.focus_input = FocusInputTarget::ChatQuestion {
                    pane_id: pane,
                    q_idx: index,
                };
                root.dispatch(GpuiAction::FocusSession(pane), cx);
            }),
        )
        .w_full()
        .min_h(px(32.))
        .max_h(px(100.))
        .overflow_y_scroll()
        .px(px(7.))
        .py(px(4.))
        .text_size(px(11.))
        .text_color(tokens.text_strong);
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(input)
            .when(b.too_long, |el| {
                el.child(
                    div()
                        .text_color(tokens.warn)
                        .text_size(px(11.))
                        .child(if french {
                            "Réponse trop longue (4 096 caractères maximum)."
                        } else {
                            "Answer too long (4,096 characters maximum)."
                        }),
                )
            })
            .into_any_element()
    }
}
fn acknowledge(root: &mut TesselGpuiRoot, pane: u64, index: usize, id: u64) {
    if let Some((current, text)) = root.app.gpui_chat_question_state(pane, index)
        && current == id
        && let Some(b) = root.chat_inputs.questions.bindings.get_mut(&(pane, index))
    {
        b.synced = text.to_owned();
    }
}
pub(crate) fn submit(
    root: &mut TesselGpuiRoot,
    pane: u64,
    id: u64,
    skip: bool,
    window: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    use gpui::EntityInputHandler;
    if !root.app.gpui_chat_input_allowed(pane)
        || root.app.gpui_chat_request_id(pane, false) != Some(id)
    {
        return;
    }
    if !skip {
        let inputs: Vec<_> = root
            .chat_inputs
            .questions
            .bindings
            .iter()
            .filter(|((p, _), b)| *p == pane && b.id == id)
            .map(|((_, i), b)| (*i, b.state.clone(), b.synced.clone()))
            .collect();
        // Check the whole request before committing any provisional composition.
        for (index, state, _) in &inputs {
            if state.update(cx, |s, cx| s.marked_text_range(window, cx).is_some()) {
                return;
            }
            if oversized(state.read(cx).as_str()) {
                if let Some(b) = root.chat_inputs.questions.bindings.get_mut(&(pane, *index)) {
                    b.too_long = true;
                }
                cx.notify();
                return;
            }
        }
        for (index, state, base) in inputs {
            let text = state.read(cx).as_str().to_owned();
            root.dispatch(
                GpuiAction::ApplyNativeQuestionEdit {
                    pane_id: pane,
                    request_instance_id: id,
                    q_idx: index,
                    base,
                    text: text.clone(),
                },
                cx,
            );
            if root.app.gpui_chat_question_state(pane, index) != Some((id, text.as_str())) {
                return;
            }
            acknowledge(root, pane, index, id);
        }
    }
    root.dispatch(
        GpuiAction::ChatSubmitQuestions {
            pane_id: pane,
            request_instance_id: id,
            skip,
        },
        cx,
    );
}

#[cfg(all(test, feature = "visual-test"))]
mod tests {
    use super::*;
    use gpui::{EntityInputHandler, TestAppContext};
    use serde_json::json;
    fn request(id: &str) -> serde_json::Value {
        json!({"type":"control_request","request_id":id,"request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"header":"First","question":"First answer?","options":[{"label":"One"},{"label":"Two"}]},{"header":"Second","question":"Second answer?","options":[{"label":"One"},{"label":"Two"}]}]}}})
    }
    #[gpui::test]
    fn native_codex_question_flush_preserves_selected_option(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let view = cx.add_window(|_,cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")),false,cx);
            root.app.apply_gpui_action(GpuiAction::AddProjectPath(project));
            root.app.apply_gpui_action(GpuiAction::LaunchCodexChat);
            let pane = root.app.gpui_snapshot(960.,640.,4.).tiles[0].session_id;
            let _receiver = root.app.gpui_chat_sender_for_test(pane);
            let composer = root.app.gpui_chat_composer(pane).unwrap().0;
            root.app.apply_gpui_action(GpuiAction::ApplyNativeChatEdit {pane_id:pane,instance_id:composer,base:String::new(),text:"begin".into()});
            root.app.apply_gpui_action(GpuiAction::NativeChatSend {pane_id:pane,instance_id:composer,text:"begin".into()});
            root.app.inject_codex_frame_for_test(pane,json!({"method":"turn/started","params":{"threadId":"synthetic-thread","turn":{"id":"turn-1"}}}));
            root.app.inject_codex_frame_for_test(pane,json!({"id":7,"method":"item/tool/requestUserInput","params":{"threadId":"synthetic-thread","turnId":"turn-1","questions":[{"id":"choice","header":"Choice","question":"Which?","isOther":true,"options":[{"label":"One","description":"First"},{"label":"Two","description":"Second"}]}]}}));
            root
        });
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            let pane = root.app.gpui_snapshot(960., 640., 4.).tiles[0].session_id;
            let id = root.app.gpui_chat_request_id(pane, false).unwrap();
            root.app
                .apply_gpui_action(GpuiAction::ChatSelectQuestionOption {
                    pane_id: pane,
                    request_instance_id: id,
                    q_idx: 0,
                    opt_idx: 0,
                });
            let snap = root.app.gpui_snapshot(960., 640., 4.);
            assert!(
                snap.tiles[0]
                    .native_chat
                    .as_ref()
                    .unwrap()
                    .questions_can_submit
            );
            // Flushing the unchanged empty Other field must not select Other.
            submit(root, pane, id, false, window, cx);
            assert!(root.app.gpui_chat_request_id(pane, false).is_none());
        })
        .unwrap();
    }

    #[gpui::test]
    fn native_questions_edit_selection_flush_answers_and_restore_composer(cx: &mut TestAppContext) {
        cx.update(|cx| {
            use gpui_ce_elements::editable_text::actions::{
                DEFAULT_INPUT_CONTEXT, default_bindings,
            };
            cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
        });
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app
                .apply_gpui_action(GpuiAction::AddProjectPath(project));
            root.app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
            let pane = root.app.gpui_snapshot(960., 640., 4.).tiles[0].session_id;
            root.app.inject_claude_frame_for_test(pane, request("one"));
            root.focus_input = FocusInputTarget::ChatQuestion {
                pane_id: pane,
                q_idx: 0,
            };
            root
        });
        cx.refresh().unwrap();
        let (pane, id) = view
            .update(cx, |root, window, cx| {
                let FocusInputTarget::ChatQuestion { pane_id: pane, .. } = root.focus_input else {
                    panic!("question focus")
                };
                let id = root.app.gpui_chat_request_id(pane, false).unwrap();
                let input = root.chat_inputs.questions.get(pane, 0).unwrap().clone();
                assert!(input.read(cx).focus_handle(cx).is_focused(window));
                input.update(cx, |s, cx| {
                    s.replace_text_in_range(None, "A😀Z", window, cx);
                    s.set_selected_text_range(1..3, window, cx);
                    s.paste(gpui::ClipboardItem::new_string("é".into()), window, cx);
                });
                (pane, id)
            })
            .unwrap();
        cx.run_until_parked();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Redo);
        cx.run_until_parked();
        cx.simulate_keystrokes(view.into(), "shift-enter");
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app.gpui_chat_question_state(pane, 0),
                Some((id, "Aé\nZ"))
            )
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "enter");
        view.update(cx, |root, window, cx| {
            assert_eq!(root.app.gpui_chat_request_id(pane, false), Some(id)); // second answer still required
            let first = root.chat_inputs.questions.get(pane, 0).unwrap().clone();
            first.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(Some(0..1), "日", None, window, cx)
            });
            let second = root.chat_inputs.questions.get(pane, 1).unwrap().clone();
            second.update(cx, |s, cx| {
                s.replace_text_in_range(None, "second", window, cx)
            });
            submit(root, pane, id, false, window, cx);
            assert_eq!(root.app.gpui_chat_request_id(pane, false), Some(id)); // IME must not submit
            first.update(cx, |s, cx| {
                s.replace_text_in_range(None, "日本", window, cx)
            });
            submit(root, pane, id, false, window, cx); // flush both fields before queued changes
            assert!(root.app.gpui_chat_request_id(pane, false).is_none());
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(root.chat_inputs.questions.get(pane, 0).is_none());
            assert_eq!(
                root.chat_inputs.focused(window, cx),
                Some(FocusInputTarget::ChatComposer(pane))
            );
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_questions_keep_oversized_answers_and_reject_replaced_requests(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app
                .apply_gpui_action(GpuiAction::AddProjectPath(project));
            root.app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
            let pane = root.app.gpui_snapshot(960., 640., 4.).tiles[0].session_id;
            root.app.inject_claude_frame_for_test(pane, request("one"));
            root.focus_input = FocusInputTarget::ChatQuestion {
                pane_id: pane,
                q_idx: 0,
            };
            root
        });
        cx.refresh().unwrap();
        let (pane, id, old) = view
            .update(cx, |root, window, cx| {
                let FocusInputTarget::ChatQuestion { pane_id: pane, .. } = root.focus_input else {
                    panic!("question focus")
                };
                let id = root.app.gpui_chat_request_id(pane, false).unwrap();
                let old = root.chat_inputs.questions.get(pane, 0).unwrap().clone();
                old.update(cx, |s, cx| {
                    s.replace_text_in_range(None, &"é".repeat(4097), window, cx)
                });
                (pane, id, old)
            })
            .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert_eq!(root.chat_inputs.questions.get(pane, 0), Some(&old));
            assert!(root.chat_inputs.questions.bindings[&(pane, 0)].too_long);
            assert_eq!(old.read(cx).as_str().chars().count(), 4097);
            assert_eq!(root.app.gpui_chat_question_state(pane, 0), Some((id, "")));
            old.update(cx, |s, cx| {
                s.replace_text_in_range(Some(0..4097), "short", window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(!root.chat_inputs.questions.bindings[&(pane, 0)].too_long);
            assert_eq!(
                root.app.gpui_chat_question_state(pane, 0),
                Some((id, "short"))
            );
            submit(root, pane, id, true, window, cx);
            root.app.inject_claude_frame_for_test(pane, request("two"));
            cx.notify();
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            let new_id = root.app.gpui_chat_request_id(pane, false).unwrap();
            assert_ne!(new_id, id);
            old.update(cx, |s, cx| {
                s.replace_text_in_range(None, "old event", window, cx)
            });
            root.app
                .apply_gpui_action(GpuiAction::ChatSelectQuestionOption {
                    pane_id: pane,
                    request_instance_id: id,
                    q_idx: 0,
                    opt_idx: 0,
                });
            root.app
                .apply_gpui_action(GpuiAction::ApplyNativeQuestionEdit {
                    pane_id: pane,
                    request_instance_id: id,
                    q_idx: 0,
                    base: String::new(),
                    text: "old".into(),
                });
            submit(root, pane, id, true, window, cx);
            assert_eq!(
                root.app.gpui_chat_question_state(pane, 0),
                Some((new_id, ""))
            );
            let snap = root.app.gpui_snapshot(960., 640., 4.);
            assert!(
                snap.tiles[0].native_chat.as_ref().unwrap().questions[0]
                    .options
                    .iter()
                    .all(|o| !o.2)
            );
        })
        .unwrap();
    }
}
