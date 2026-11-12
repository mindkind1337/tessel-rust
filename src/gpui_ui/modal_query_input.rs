//! Search fields tied to one Help/Settings overlay lifetime.
use super::{FocusInputTarget, TesselGpuiRoot};
use crate::app::gpui_bridge::GpuiAction;
use gpui::{App, AppContext, Context, Entity, EntityInputHandler, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
type Scope = (u64, FocusInputTarget);
const LIMIT: usize = 8192;
fn limit(target: FocusInputTarget) -> usize {
    if matches!(target, FocusInputTarget::GuestTaskDraft(_)) {
        4096
    } else {
        LIMIT
    }
}
struct Binding {
    scope: Scope,
    state: Entity<EditableTextState>,
    synced: String,
    too_long: bool,
    _changed: Subscription,
}
#[derive(Default)]
pub(crate) struct Input {
    binding: Option<Binding>,
}
gpui::actions!(tessel_modal_query, [Dismiss, ConsumeEnter]);
impl Input {
    pub fn focused(&self, w: &Window, cx: &App) -> Option<FocusInputTarget> {
        self.binding
            .as_ref()
            .filter(|b| b.state.read(cx).focus_handle(cx).is_focused(w))
            .map(|b| b.scope.1)
    }
    pub fn scope(&self) -> Option<Scope> {
        self.binding.as_ref().map(|b| b.scope)
    }
    pub fn state(&self) -> Option<&Entity<EditableTextState>> {
        self.binding.as_ref().map(|b| &b.state)
    }
    pub fn sync(&mut self, scope: Scope, text: &str, cx: &mut Context<TesselGpuiRoot>) {
        if self.binding.as_ref().is_some_and(|b| {
            b.scope == scope
                && (b.state.read(cx).as_str() == text || (b.too_long && b.synced == text))
        }) {
            return;
        }
        let state = cx.new(|cx| EditableTextState::new(StringStorage::from(text), cx));
        let changed = cx.subscribe(&state, move |root, state, _: &TextChanged, cx| {
            let Some(b) = root.modal_query_input.binding.as_ref() else {
                return;
            };
            if b.scope != scope
                || b.state != state
                || root.app.gpui_modal_query() != Some((scope, b.synced.as_str()))
            {
                return;
            }
            flush(root, cx);
            cx.notify();
        });
        self.binding = Some(Binding {
            scope,
            state,
            synced: text.to_owned(),
            too_long: false,
            _changed: changed,
        });
    }
    pub fn render(
        &self,
        target: FocusInputTarget,
        french: bool,
        tokens: &super::theme::GpuiThemeTokens,
        cx: &mut Context<TesselGpuiRoot>,
    ) -> gpui::AnyElement {
        use gpui::{
            InteractiveElement, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder, px,
        };
        let Some(b) = self.binding.as_ref().filter(|b| b.scope.1 == target) else {
            return div().into_any_element();
        };
        let expected = b.state.clone();
        let clicked = b.state.clone();
        let enter = b.state.clone();
        let scope = b.scope;
        let placeholder = match (target, french) {
            (FocusInputTarget::GuestTaskDraft(_), true) => "Nouvelle tâche…",
            (FocusInputTarget::GuestTaskDraft(_), false) => "New task…",
            (FocusInputTarget::HelpQuery, true) => "Filtrer les raccourcis…",
            (FocusInputTarget::HelpQuery, false) => "Search shortcuts…",
            (_, true) => "Rechercher un paramètre…",
            (_, false) => "Search settings…",
        };
        let field = gpui_ce_elements::editable_text::text_input("native-modal-query")
            .state(b.state.downgrade())
            .key_context("EditableText TesselModalQuery")
            .placeholder(placeholder)
            .on_action(cx.listener(move |root, _: &Dismiss, w, cx| {
                if root.modal_query_input.state() == Some(&expected) {
                    dismiss(root, w, cx);
                }
            }))
            .on_action(cx.listener(move |root, _: &ConsumeEnter, w, cx| {
                flush(root, cx);
                let FocusInputTarget::GuestTaskDraft(guest_scope) = scope.1 else {
                    return;
                };
                let Some(binding) = root.modal_query_input.binding.as_ref() else {
                    return;
                };
                if binding.scope != scope
                    || binding.state != enter
                    || binding.too_long
                    || binding.state.read(cx).as_str() != binding.synced
                    || root.app.gpui_modal_query() != Some((scope, binding.synced.as_str()))
                {
                    return;
                }
                if enter.update(cx, |s, cx| s.marked_text_range(w, cx).is_some()) {
                    return;
                }
                root.dispatch(
                    GpuiAction::Share(crate::share_panel::ShareGpuiAction::GuestTask {
                        scope: guest_scope,
                        action: crate::guest_task_board::Action::Add,
                    }),
                    cx,
                );
            }))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |root, _, _, cx| {
                    if root.modal_query_input.state() == Some(&clicked)
                        && root.app.gpui_modal_query().is_some()
                    {
                        root.focus_input = target;
                    }
                    cx.stop_propagation();
                }),
            )
            .w_full()
            .h(px(28.))
            .px(px(8.))
            .py(px(4.))
            .text_size(px(12.))
            .text_color(tokens.text);
        div()
            .w_full()
            .bg(tokens.term)
            .border_1()
            .border_color(tokens.border)
            .rounded(px(4.))
            .child(field)
            .when(b.too_long, |el| {
                el.child(
                    div()
                        .text_size(px(10.))
                        .text_color(tokens.warn)
                        .child(if french {
                            if matches!(target, FocusInputTarget::GuestTaskDraft(_)) {
                                "Brouillon limité à 4 Kio"
                            } else {
                                "Recherche limitée à 8 Kio"
                            }
                        } else {
                            if matches!(target, FocusInputTarget::GuestTaskDraft(_)) {
                                "Draft limit: 4 KiB"
                            } else {
                                "Search limit: 8 KiB"
                            }
                        }),
                )
            })
            .into_any_element()
    }
}
pub fn flush(root: &mut TesselGpuiRoot, cx: &mut Context<TesselGpuiRoot>) {
    let Some(b) = root.modal_query_input.binding.as_ref() else {
        return;
    };
    if root.app.gpui_modal_query() != Some((b.scope, b.synced.as_str())) {
        return;
    }
    let scope = b.scope;
    let base = b.synced.clone();
    let text = b.state.read(cx).as_str().to_owned();
    let was_too_long = b.too_long;
    let too_long = text.len() > limit(scope.1);
    root.modal_query_input.binding.as_mut().unwrap().too_long = too_long;
    if too_long {
        if !was_too_long && let FocusInputTarget::GuestTaskDraft(guest_scope) = scope.1 {
            root.dispatch(
                GpuiAction::Share(crate::share_panel::ShareGpuiAction::GuestDraftTouch(
                    guest_scope,
                )),
                cx,
            );
        }
    } else if text != base
        || (was_too_long && matches!(scope.1, FocusInputTarget::GuestTaskDraft(_)))
    {
        root.dispatch(GpuiAction::NativeModalQuery { scope, base, text }, cx);
        acknowledge(root);
    }
}

fn acknowledge(root: &mut TesselGpuiRoot) {
    if let Some((scope, text)) = root.app.gpui_modal_query()
        && let Some(b) = root.modal_query_input.binding.as_mut()
        && b.scope == scope
    {
        b.synced = text.to_owned();
    }
}
fn dismiss(root: &mut TesselGpuiRoot, w: &mut Window, cx: &mut Context<TesselGpuiRoot>) {
    let Some(b) = root.modal_query_input.binding.as_ref() else {
        return;
    };
    if root.app.gpui_modal_query() != Some((b.scope, b.synced.as_str())) {
        return;
    }
    let scope = b.scope;
    let base = b.synced.clone();
    let state = b.state.clone();
    if state.update(cx, |s, cx| s.marked_text_range(w, cx).is_some()) {
        return;
    }
    if !state.read(cx).as_str().is_empty() {
        state.update(cx, |s, cx| {
            let len = s.as_str().encode_utf16().count();
            s.push_undo_stop();
            s.replace_text_in_range(Some(0..len), "", w, cx);
            s.push_undo_stop();
        });
        root.dispatch(
            GpuiAction::NativeModalQuery {
                scope,
                base,
                text: String::new(),
            },
            cx,
        );
        acknowledge(root);
        if let Some(b) = root.modal_query_input.binding.as_mut() {
            b.too_long = false;
        }
    } else {
        root.dispatch(GpuiAction::DismissNativeModalQuery { scope, base }, cx);
        root.focus_input = FocusInputTarget::None;
        w.focus(&root.focus_handle, cx);
    }
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
    fn native_modal_query_unicode_history_and_two_stage_escape(cx: &mut TestAppContext) {
        bindings(cx);
        for settings in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let view = cx.add_window(|_, cx| {
                let mut root =
                    TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
                root.dispatch(
                    if settings {
                        GpuiAction::OpenSettings
                    } else {
                        GpuiAction::OpenHelp
                    },
                    cx,
                );
                root
            });
            cx.refresh().unwrap();
            let input = view
                .update(cx, |root, w, cx| {
                    let input = root.modal_query_input.state().unwrap().clone();
                    assert!(input.read(cx).focus_handle(cx).is_focused(w));
                    input.update(cx, |s, cx| {
                        s.replace_text_in_range(None, "al😀ha", w, cx);
                        s.set_selected_text_range(2..4, w, cx);
                        s.paste(gpui::ClipboardItem::new_string("p".into()), w, cx);
                    });
                    input
                })
                .unwrap();
            cx.run_until_parked();
            cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
            cx.run_until_parked();
            cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Redo);
            cx.run_until_parked();
            view.update(cx, |root, _, _| {
                assert_eq!(root.app.gpui_modal_query().unwrap().1, "alpha")
            })
            .unwrap();
            cx.simulate_keystrokes(view.into(), "enter");
            cx.run_until_parked();
            view.update(cx, |root, _, cx| {
                assert_eq!(input.read(cx).as_str(), "alpha");
                assert_eq!(root.app.gpui_modal_query().unwrap().1, "alpha");
            })
            .unwrap();
            cx.simulate_keystrokes(view.into(), "escape");
            cx.refresh().unwrap();
            view.update(cx, |root, _, cx| {
                assert_eq!(root.app.gpui_modal_query().unwrap().1, "");
                assert_eq!(
                    root.modal_query_input.state().unwrap().read(cx).as_str(),
                    ""
                );
            })
            .unwrap();
            cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
            cx.run_until_parked();
            view.update(cx, |root, _, _| {
                assert_eq!(root.app.gpui_modal_query().unwrap().1, "alpha")
            })
            .unwrap();
            cx.simulate_keystrokes(view.into(), "escape");
            cx.refresh().unwrap();
            cx.simulate_keystrokes(view.into(), "escape");
            cx.refresh().unwrap();
            view.update(cx, |root, w, cx| {
                assert!(root.app.gpui_modal_query().is_none());
                assert!(root.modal_query_input.focused(w, cx).is_none());
            })
            .unwrap();
        }
    }
    #[gpui::test]
    fn native_modal_query_ime_oversize_and_stale_reopening(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::OpenHelp, cx);
            root
        });
        cx.refresh().unwrap();
        let (scope, input) = view
            .update(cx, |root, w, cx| {
                let scope = root.app.gpui_modal_query().unwrap().0;
                let input = root.modal_query_input.state().unwrap().clone();
                input.update(cx, |s, cx| {
                    s.replace_text_in_range(None, &"x".repeat(LIMIT + 1), w, cx)
                });
                (scope, input)
            })
            .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert_eq!(root.modal_query_input.state(), Some(&input));
            assert!(root.modal_query_input.binding.as_ref().unwrap().too_long);
            assert_eq!(root.app.gpui_modal_query().unwrap().1, "");
            input.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(Some(0..LIMIT + 1), "日", None, w, cx)
            });
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "escape");
        view.update(cx, |root, w, cx| {
            assert!(root.app.gpui_modal_query().is_some());
            input.update(cx, |s, cx| s.replace_text_in_range(None, "pending", w, cx));
            dismiss(root, w, cx);
            assert_eq!(root.app.gpui_modal_query().unwrap().1, "");
            root.dispatch(GpuiAction::CloseOverlays, cx);
            root.dispatch(GpuiAction::OpenHelp, cx);
            assert_ne!(root.app.gpui_modal_query().unwrap().0, scope);
            input.update(cx, |s, cx| s.replace_text_in_range(None, "stale", w, cx));
            root.app
                .apply_gpui_action(GpuiAction::DismissNativeModalQuery {
                    scope,
                    base: String::new(),
                });
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, _, cx| {
            assert_ne!(root.modal_query_input.state(), Some(&input));
            assert_eq!(root.app.gpui_modal_query().unwrap().1, "");
            root.dispatch(GpuiAction::OpenSettings, cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app.gpui_modal_query().unwrap().0.1,
                FocusInputTarget::SettingsSearch
            )
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_settings_search_keeps_entity_across_pages_and_toggle_blurs(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app.apply_gpui_action(GpuiAction::OpenSettings);
            root
        });
        cx.refresh().unwrap();
        let input = view
            .update(cx, |root, w, cx| {
                let input = root.modal_query_input.state().unwrap().clone();
                assert!(input.read(cx).focus_handle(cx).is_focused(w));
                input.update(cx, |s, cx| s.replace_text_in_range(None, "git", w, cx));
                input
            })
            .unwrap();
        cx.run_until_parked();
        view.update(cx, |root, _, cx| {
            root.dispatch(GpuiAction::OpenSettingsPage("appearance".into()), cx)
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, _, cx| {
            assert_eq!(root.modal_query_input.state(), Some(&input));
            assert_eq!(root.app.gpui_modal_query().unwrap().1, "git");
            root.dispatch(GpuiAction::OpenSettings, cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert!(root.app.gpui_modal_query().is_none());
            assert!(root.modal_query_input.focused(w, cx).is_none());
            assert_eq!(root.focus_input, FocusInputTarget::None);
        })
        .unwrap();
    }
}

#[cfg(all(test, feature = "visual-test"))]
#[path = "guest_task_input_tests.rs"]
mod guest_tests;
