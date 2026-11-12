//! Terminal search belongs to one terminal search opening, never just a pane ID.
use super::{FocusInputTarget, TesselGpuiRoot};
use crate::{app::gpui_bridge::GpuiAction, terminal::TerminalRenderSnapshot};
use gpui::{App, AppContext, Context, Entity, EntityInputHandler, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
use std::collections::HashMap;
const MAX_QUERY: usize = 65536;
struct Binding {
    id: u64,
    state: Entity<EditableTextState>,
    synced: String,
    too_long: bool,
    _changed: Subscription,
}
#[derive(Default)]
pub(crate) struct Inputs {
    bindings: HashMap<Option<u64>, Binding>,
    visible: Vec<(Option<u64>, u64)>,
    pub select_on_focus: Option<(Option<u64>, u64)>,
}
gpui::actions!(tessel_terminal_find, [Next, Previous, Dismiss]);
impl Inputs {
    pub fn begin_frame(&mut self) {
        self.visible.clear();
    }
    pub fn visible(&self, sid: Option<u64>, id: u64) -> bool {
        self.visible.contains(&(sid, id))
    }
    pub fn focused(&self, w: &Window, cx: &App) -> Option<FocusInputTarget> {
        self.bindings.iter().find_map(|(sid, b)| {
            b.state
                .read(cx)
                .focus_handle(cx)
                .is_focused(w)
                .then_some(FocusInputTarget::TerminalFind(*sid))
        })
    }
    pub fn get(&self, sid: Option<u64>, id: u64) -> Option<&Entity<EditableTextState>> {
        self.bindings
            .get(&sid)
            .filter(|b| b.id == id)
            .map(|b| &b.state)
    }
    pub fn retain(&mut self, ids: &[u64]) {
        self.bindings.retain(|_, b| ids.contains(&b.id));
    }
    pub fn sync(
        &mut self,
        sid: Option<u64>,
        snap: &TerminalRenderSnapshot,
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        if !snap.find_open {
            return;
        }
        let id = snap.find_input_id;
        self.visible.push((sid, id));
        let text = &snap.find_query;
        if self.bindings.get(&sid).is_some_and(|b| {
            b.id == id && (b.state.read(cx).as_str() == text || (b.too_long && b.synced == *text))
        }) {
            return;
        }
        let state = cx.new(|cx| EditableTextState::new(StringStorage::from(text.as_str()), cx));
        let changed = cx.subscribe(&state, move |root, state, _: &TextChanged, cx| {
            let Some(b) = root.terminal_find_inputs.bindings.get(&sid) else {
                return;
            };
            if b.id != id
                || b.state != state
                || root.app.gpui_terminal_find_input(sid).map(|v| v.0) != Some(id)
            {
                return;
            }
            let base = b.synced.clone();
            let too_long = state.read(cx).as_str().len() > MAX_QUERY;
            root.terminal_find_inputs
                .bindings
                .get_mut(&sid)
                .unwrap()
                .too_long = too_long;
            if too_long {
                cx.notify();
                return;
            }
            let text = state.read(cx).as_str().to_owned();
            if text != base {
                root.dispatch(
                    GpuiAction::ApplyNativeTerminalFindEdit {
                        session_id: sid,
                        instance_id: id,
                        base,
                        text,
                    },
                    cx,
                );
                acknowledge(root, sid, id);
            }
        });
        self.bindings.insert(
            sid,
            Binding {
                id,
                state,
                synced: text.clone(),
                too_long: false,
                _changed: changed,
            },
        );
    }
    pub fn render(
        &self,
        sid: Option<u64>,
        snap: &TerminalRenderSnapshot,
        french: bool,
        tokens: &super::theme::GpuiThemeTokens,
        cx: &mut Context<TesselGpuiRoot>,
    ) -> gpui::AnyElement {
        use gpui::{
            InteractiveElement, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder, px,
        };
        let id = snap.find_input_id;
        let Some(b) = self.bindings.get(&sid).filter(|b| b.id == id) else {
            return div().into_any_element();
        };
        let next_state = b.state.clone();
        let previous_state = b.state.clone();
        let dismiss_state = b.state.clone();
        let input = gpui_ce_elements::editable_text::text_input((
            "native-terminal-find",
            sid.unwrap_or(u64::MAX),
        ))
        .state(b.state.downgrade())
        .placeholder(if french {
            "Rechercher dans le terminal…"
        } else {
            "Find in terminal…"
        })
        .key_context("EditableText TesselTerminalFind")
        .on_action(cx.listener(move |root, _: &Next, w, cx| {
            if root.terminal_find_inputs.get(sid, id) == Some(&next_state) {
                run(root, sid, id, Some(1), w, cx);
            }
        }))
        .on_action(cx.listener(move |root, _: &Previous, w, cx| {
            if root.terminal_find_inputs.get(sid, id) == Some(&previous_state) {
                run(root, sid, id, Some(-1), w, cx);
            }
        }))
        .on_action(cx.listener(move |root, _: &Dismiss, w, cx| {
            if root.terminal_find_inputs.get(sid, id) == Some(&dismiss_state) {
                run(root, sid, id, None, w, cx);
            }
        }))
        .on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(move |root, _, _, cx| {
                if let Some(sid) = sid {
                    root.dispatch(GpuiAction::FocusSession(sid), cx);
                }
                root.focus_input = FocusInputTarget::TerminalFind(sid);
                cx.stop_propagation();
            }),
        )
        .w_full()
        .h(px(25.))
        .px(px(6.))
        .py(px(3.))
        .text_size(px(11.5))
        .text_color(tokens.text_strong);
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(tokens.term)
            .border_1()
            .border_color(tokens.accent)
            .rounded(px(4.))
            .child(input)
            .when(b.too_long, |el| {
                el.child(
                    div()
                        .text_size(px(10.))
                        .text_color(tokens.warn)
                        .child(if french {
                            "Recherche limitée à 64 Kio"
                        } else {
                            "Search limit: 64 KiB"
                        }),
                )
            })
            .into_any_element()
    }
}
fn acknowledge(root: &mut TesselGpuiRoot, sid: Option<u64>, id: u64) {
    if let Some((current, text)) = root.app.gpui_terminal_find_input(sid)
        && current == id
        && let Some(b) = root.terminal_find_inputs.bindings.get_mut(&sid)
    {
        b.synced = text.to_owned();
    }
}
pub(crate) fn run(
    root: &mut TesselGpuiRoot,
    sid: Option<u64>,
    id: u64,
    step: Option<isize>,
    w: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    let Some((current, query)) = root.app.gpui_terminal_find_input(sid) else {
        return;
    };
    if current != id {
        return;
    }
    let Some(b) = root
        .terminal_find_inputs
        .bindings
        .get(&sid)
        .filter(|b| b.id == id)
    else {
        return;
    };
    if b.synced != query {
        return;
    }
    let state = b.state.clone();
    let base = b.synced.clone();
    if state.update(cx, |s, cx| s.marked_text_range(w, cx).is_some()) {
        return;
    }
    let query = if step.is_none() {
        base.clone()
    } else {
        if state.read(cx).as_str().len() > MAX_QUERY {
            return;
        }
        state.read(cx).as_str().to_owned()
    };
    if step.is_some() && query != base {
        root.dispatch(
            GpuiAction::ApplyNativeTerminalFindEdit {
                session_id: sid,
                instance_id: id,
                base,
                text: query.clone(),
            },
            cx,
        );
        acknowledge(root, sid, id);
    }
    root.dispatch(
        GpuiAction::NativeTerminalFindCommand {
            session_id: sid,
            instance_id: id,
            query,
            step,
        },
        cx,
    );
    root.focus_input = if step.is_some() {
        FocusInputTarget::TerminalFind(sid)
    } else {
        FocusInputTarget::Terminal(sid)
    };
}

#[cfg(all(test, feature = "visual-test"))]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    fn bindings(cx: &mut TestAppContext) {
        cx.update(|cx| {
            use gpui_ce_elements::editable_text::actions::{
                DEFAULT_INPUT_CONTEXT, default_bindings,
            };
            cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
        });
    }
    #[gpui::test]
    fn native_terminal_find_unicode_undo_ime_and_no_shell_input(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut fixture = None;
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app
                .apply_gpui_action(GpuiAction::AddProjectPath(project));
            let (sid, rx) = root.app.install_terminal_find_fixture();
            root.dispatch(GpuiAction::TerminalOpenFind(Some(sid)), cx);
            fixture = Some((sid, rx));
            root
        });
        let (sid, rx) = fixture.unwrap();
        cx.refresh().unwrap();
        let (id, input) = view
            .update(cx, |root, window, cx| {
                let id = root.app.gpui_terminal_find_input(Some(sid)).unwrap().0;
                let input = root
                    .terminal_find_inputs
                    .get(Some(sid), id)
                    .unwrap()
                    .clone();
                assert!(input.read(cx).focus_handle(cx).is_focused(window));
                input.update(cx, |s, cx| {
                    s.replace_text_in_range(None, "al😀ha", window, cx);
                    s.set_selected_text_range(2..4, window, cx);
                    s.paste(gpui::ClipboardItem::new_string("p".into()), window, cx);
                });
                (id, input)
            })
            .unwrap();
        cx.run_until_parked();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Redo);
        cx.run_until_parked();
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-shift-f");
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert_eq!(
                root.app.gpui_terminal_find_input(Some(sid)),
                Some((id, "alpha"))
            );
            assert_eq!(
                input
                    .update(cx, |s, cx| s
                        .selected_text_range(false, window, cx)
                        .unwrap())
                    .range,
                0..5
            );
            input.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(Some(0..5), "日", None, window, cx)
            });
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "escape");
        view.update(cx, |root, window, cx| {
            assert!(root.app.gpui_terminal_find_input(Some(sid)).is_some());
            input.update(cx, |s, cx| {
                s.replace_text_in_range(None, "beta", window, cx)
            });
            run(root, Some(sid), id, Some(1), window, cx);
            assert_eq!(
                root.app.gpui_terminal_find_input(Some(sid)),
                Some((id, "beta"))
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "escape");
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            assert!(root.app.gpui_terminal_find_input(Some(sid)).is_none())
        })
        .unwrap();
        assert!(
            rx.try_recv().is_err(),
            "search editing must never write to the PTY"
        );
        cx.simulate_keystrokes(view.into(), "z");
        assert_eq!(
            rx.try_recv().unwrap(),
            b"z",
            "closing search restores terminal input"
        );
    }
    #[gpui::test]
    fn native_terminal_find_reopen_restart_and_project_scope(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut sid = 0;
        let mut receivers = Vec::new();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app
                .apply_gpui_action(GpuiAction::AddProjectPath(project));
            let (id, rx) = root.app.install_terminal_find_fixture();
            sid = id;
            receivers.push(rx);
            root.dispatch(GpuiAction::TerminalOpenFind(Some(sid)), cx);
            root
        });
        cx.refresh().unwrap();
        let (old_id, old) = view
            .update(cx, |root, _, _| {
                let id = root.app.gpui_terminal_find_input(Some(sid)).unwrap().0;
                (
                    id,
                    root.terminal_find_inputs
                        .get(Some(sid), id)
                        .unwrap()
                        .clone(),
                )
            })
            .unwrap();
        view.update(cx, |root, _, cx| {
            root.dispatch(GpuiAction::TerminalCloseFind(Some(sid)), cx);
            root.dispatch(GpuiAction::TerminalOpenFind(Some(sid)), cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert_ne!(
                root.app.gpui_terminal_find_input(Some(sid)).unwrap().0,
                old_id
            );
            old.update(cx, |s, cx| {
                s.replace_text_in_range(None, "stale", window, cx)
            });
            run(root, Some(sid), old_id, None, window, cx);
            assert!(root.app.gpui_terminal_find_input(Some(sid)).is_some());
            receivers.push(root.app.replace_terminal_find_fixture(sid));
            root.dispatch(GpuiAction::TerminalOpenFind(Some(sid)), cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        let new_id = view
            .update(cx, |root, _, _| {
                root.app.gpui_terminal_find_input(Some(sid)).unwrap().0
            })
            .unwrap();
        assert_ne!(new_id, old_id);
        let second = dir.path().join("second");
        std::fs::create_dir(&second).unwrap();
        view.update(cx, |root, window, cx| {
            root.dispatch(GpuiAction::AddProjectPath(second), cx);
            root.app
                .apply_gpui_action(GpuiAction::ApplyNativeTerminalFindEdit {
                    session_id: Some(sid),
                    instance_id: new_id,
                    base: String::new(),
                    text: "wrong project".into(),
                });
            run(root, Some(sid), new_id, None, window, cx);
            assert!(root.app.gpui_terminal_find_input(Some(sid)).is_none());
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(root.terminal_find_inputs.focused(window, cx).is_none())
        })
        .unwrap();
        for rx in receivers {
            assert!(rx.try_recv().is_err());
        }
    }
    #[gpui::test]
    fn native_floating_find_scope_and_oversized_draft(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let mut receiver = None;
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            receiver = Some(root.app.install_floating_find_fixture());
            root.dispatch(GpuiAction::TerminalOpenFind(None), cx);
            root
        });
        cx.refresh().unwrap();
        let (id, input) = view
            .update(cx, |root, window, cx| {
                let id = root.app.gpui_terminal_find_input(None).unwrap().0;
                let input = root.terminal_find_inputs.get(None, id).unwrap().clone();
                input.update(cx, |s, cx| {
                    s.replace_text_in_range(None, &"x".repeat(MAX_QUERY + 1), window, cx)
                });
                (id, input)
            })
            .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert_eq!(root.terminal_find_inputs.get(None, id), Some(&input));
            assert!(root.terminal_find_inputs.bindings[&None].too_long);
            assert_eq!(input.read(cx).as_str().len(), MAX_QUERY + 1);
            input.update(cx, |s, cx| {
                s.replace_text_in_range(Some(0..MAX_QUERY + 1), "alpha", window, cx)
            });
            run(root, None, id, Some(1), window, cx);
            assert_eq!(root.app.gpui_terminal_find_input(None), Some((id, "alpha")));
            root.dispatch(GpuiAction::OpenHelp, cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(root.terminal_find_inputs.focused(window, cx).is_none());
            run(root, None, id, None, window, cx);
            root.dispatch(GpuiAction::CloseOverlays, cx);
            assert_eq!(root.app.gpui_terminal_find_input(None), Some((id, "alpha")));
            root.app.set_floating_input_fixture(false);
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(root.terminal_find_inputs.focused(window, cx).is_none())
        })
        .unwrap();
        assert!(receiver.unwrap().try_recv().is_err());
    }
}
