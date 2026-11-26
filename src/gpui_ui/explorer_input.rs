//! Native file filter; query semantics persist, events belong to one visible project.
use super::{FocusInputTarget, TesselGpuiRoot};
use crate::app::gpui_bridge::{GpuiAction, NativeExplorerScope};
use gpui::{App, AppContext, Context, Entity, EntityInputHandler, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
const LIMIT: usize = 8192;
struct Binding {
    scope: NativeExplorerScope,
    state: Entity<EditableTextState>,
    synced: String,
    too_long: bool,
    _changed: Subscription,
}
#[derive(Default)]
pub(crate) struct Input {
    binding: Option<Binding>,
    pub outline_scroll: gpui::ScrollHandle,
    pub last_outline_scroll: std::cell::Cell<Option<(u64, u64)>>,
}
gpui::actions!(tessel_explorer_filter, [Clear, ConsumeEnter]);
impl Input {
    pub fn state(&self) -> Option<&Entity<EditableTextState>> {
        self.binding.as_ref().map(|b| &b.state)
    }
    pub fn focused(&self, w: &Window, cx: &App) -> bool {
        self.state()
            .is_some_and(|s| s.read(cx).focus_handle(cx).is_focused(w))
    }
    pub fn sync(
        &mut self,
        scope: NativeExplorerScope,
        text: &str,
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        if self.binding.as_ref().is_some_and(|b| {
            b.scope == scope
                && (b.state.read(cx).as_str() == text || (b.too_long && b.synced == text))
        }) {
            return;
        }
        let retained = self
            .binding
            .as_ref()
            .filter(|b| b.state.read(cx).as_str() == text || (b.too_long && b.synced == text));
        let too_long = retained.is_some_and(|b| b.too_long);
        let state = retained
            .map(|b| b.state.clone())
            .unwrap_or_else(|| cx.new(|cx| EditableTextState::new(StringStorage::from(text), cx)));
        let captured = scope.clone();
        let changed = cx.subscribe(&state, move |root, state, _: &TextChanged, cx| {
            if root
                .explorer_input
                .binding
                .as_ref()
                .is_some_and(|b| b.scope == captured && b.state == state)
            {
                flush(root, cx);
                cx.notify();
            }
        });
        self.binding = Some(Binding {
            scope,
            state,
            synced: text.to_owned(),
            too_long,
            _changed: changed,
        });
    }
    pub fn render(
        &self,
        content: bool,
        french: bool,
        tokens: &super::theme::GpuiThemeTokens,
        cx: &mut Context<TesselGpuiRoot>,
    ) -> gpui::AnyElement {
        use gpui::{
            InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
            div, prelude::FluentBuilder, px,
        };
        let Some(b) = &self.binding else {
            return div().into_any_element();
        };
        let expected = (b.scope.clone(), b.state.clone());
        let clicked = (b.scope.clone(), b.state.clone());
        let button = (b.scope.clone(), b.state.clone());
        let field = gpui_ce_elements::editable_text::text_input("native-explorer-filter")
            .state(b.state.downgrade())
            .key_context("EditableText TesselExplorerFilter")
            .placeholder(match (content, french) {
                (true, true) => "Rechercher dans les fichiers…",
                (true, false) => "Search file contents…",
                (false, true) => "Filtrer les fichiers…",
                (false, false) => "Filter files…",
            })
            .on_action(cx.listener(move |root, _: &Clear, w, cx| clear(root, &expected, w, cx)))
            .on_action(cx.listener(|_, _: &ConsumeEnter, _, _| {}))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |root, _, _, cx| {
                    if matches(root, &clicked) {
                        root.focus_input = FocusInputTarget::ExplorerFilter;
                    }
                    cx.stop_propagation();
                }),
            )
            .flex_1()
            .min_w_0()
            .h(px(28.))
            .px(px(8.))
            .py(px(4.))
            .text_size(px(12.))
            .text_color(tokens.text);
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .bg(tokens.term)
                    .border_1()
                    .border_color(tokens.border)
                    .rounded(px(4.))
                    .child(field)
                    .when(!b.state.read(cx).as_str().is_empty(), |el| {
                        el.child(
                            div()
                                .id("native-explorer-clear")
                                .px(px(6.))
                                .cursor_pointer()
                                .text_color(tokens.text_dim)
                                .on_click(
                                    cx.listener(move |root, _, w, cx| clear(root, &button, w, cx)),
                                )
                                .child("×"),
                        )
                    }),
            )
            .when(b.too_long, |el| {
                el.child(
                    div()
                        .text_size(px(10.))
                        .text_color(tokens.warn)
                        .child(if french {
                            "Recherche limitée à 8 Kio"
                        } else {
                            "Search limit: 8 KiB"
                        }),
                )
            })
            .into_any_element()
    }
}
fn matches(
    root: &TesselGpuiRoot,
    expected: &(NativeExplorerScope, Entity<EditableTextState>),
) -> bool {
    root.explorer_input.binding.as_ref().is_some_and(|b| {
        b.scope == expected.0
            && b.state == expected.1
            && root
                .app
                .gpui_explorer_input()
                .is_some_and(|(s, t)| s == b.scope && t == b.synced)
    })
}
pub(crate) fn flush(root: &mut TesselGpuiRoot, cx: &mut Context<TesselGpuiRoot>) {
    let Some(b) = root.explorer_input.binding.as_ref() else {
        return;
    };
    if !matches(root, &(b.scope.clone(), b.state.clone())) {
        return;
    }
    let text = b.state.read(cx).as_str();
    let too_long = text.len() > LIMIT;
    if !too_long && text != b.synced {
        // No workspace transition here: avoid recursively flushing other native fields.
        root.app.apply_gpui_action(GpuiAction::NativeExplorerEdit {
            scope: b.scope.clone(),
            base: b.synced.clone(),
            text: text.to_owned(),
        });
        if let Some((scope, text)) = root.app.gpui_explorer_input()
            && let Some(b) = root.explorer_input.binding.as_mut()
            && b.scope == scope
        {
            b.synced = text.to_owned();
        }
    }
    if let Some(b) = root.explorer_input.binding.as_mut() {
        b.too_long = too_long;
    }
}
fn clear(
    root: &mut TesselGpuiRoot,
    expected: &(NativeExplorerScope, Entity<EditableTextState>),
    w: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    if !matches(root, expected)
        || expected
            .1
            .update(cx, |s, cx| s.marked_text_range(w, cx).is_some())
    {
        return;
    }
    expected.1.update(cx, |s, cx| {
        if !s.as_str().is_empty() {
            let len = s.as_str().encode_utf16().count();
            s.push_undo_stop();
            s.replace_text_in_range(Some(0..len), "", w, cx);
            s.push_undo_stop();
        }
    });
    flush(root, cx);
    root.focus_input = FocusInputTarget::ExplorerFilter;
    w.focus(&expected.1.read(cx).focus_handle(cx), cx);
    cx.notify();
}

#[cfg(all(test, feature = "visual-test"))]
mod tests {
    use super::*;
    use crate::workspace::SideTab;
    use gpui::TestAppContext;
    fn prepare(
        root: &mut TesselGpuiRoot,
        path: std::path::PathBuf,
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        std::fs::create_dir_all(&path).unwrap();
        root.dispatch(GpuiAction::AddProjectPath(path), cx);
        if root.app.gpui_explorer_input().is_none() {
            root.dispatch(GpuiAction::SelectSideTab(SideTab::Files), cx);
        }
        root.focus_input = FocusInputTarget::ExplorerFilter;
    }
    fn bindings(cx: &mut TestAppContext) {
        cx.update(|cx| {
            use gpui_ce_elements::editable_text::actions::{
                DEFAULT_INPUT_CONTEXT, default_bindings,
            };
            cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
        });
    }
    #[gpui::test]
    fn native_explorer_unicode_history_escape_and_mode_preserve_query(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            prepare(&mut root, dir.path().join("project"), cx);
            root
        });
        cx.refresh().unwrap();
        let input = view
            .update(cx, |root, w, cx| {
                let input = root.explorer_input.state().unwrap().clone();
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
        view.update(cx, |root, _, cx| {
            assert_eq!(root.app.gpui_explorer_input().unwrap().1, "alpha");
            root.dispatch(GpuiAction::ExplorerToggleSearchMode, cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "enter");
        view.update(cx, |root, _, _| {
            assert_eq!(root.explorer_input.state(), Some(&input));
            assert_eq!(root.app.gpui_explorer_input().unwrap().1, "alpha");
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "escape");
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            assert_eq!(root.app.gpui_explorer_input().unwrap().1, "")
        })
        .unwrap();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert_eq!(root.app.gpui_explorer_input().unwrap().1, "alpha")
        })
        .unwrap();
        let second = dir.path().join("second");
        std::fs::create_dir(&second).unwrap();
        view.update(cx, |root, w, cx| {
            input.update(cx, |s, cx| {
                s.push_undo_stop();
                s.replace_text_in_range(Some(0..5), "beta", w, cx);
                s.push_undo_stop();
            });
            root.dispatch(GpuiAction::AddProjectPath(second), cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert_eq!(root.explorer_input.state(), Some(&input));
            assert_eq!(root.app.gpui_explorer_input().unwrap().1, "alpha");
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_explorer_flushes_before_project_switch_and_rejects_closed_scope(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let second = dir.path().join("second");
        std::fs::create_dir(&second).unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            prepare(&mut root, dir.path().join("first"), cx);
            root
        });
        cx.refresh().unwrap();
        let old = view
            .update(cx, |root, w, cx| {
                let old = (
                    root.app.gpui_explorer_input().unwrap().0,
                    root.explorer_input.state().unwrap().clone(),
                );
                let first = root.app.gpui_explorer_input().unwrap().0.owner.project;
                old.1
                    .update(cx, |s, cx| s.replace_text_in_range(None, "kept", w, cx));
                root.dispatch(GpuiAction::AddProjectPath(second), cx);
                assert_eq!(root.app.gpui_explorer_input().unwrap().1, "kept");
                assert!(!matches(root, &old));
                clear(root, &old, w, cx);
                root.dispatch(GpuiAction::SelectProject(first), cx);
                assert!(!matches(root, &old));
                clear(root, &old, w, cx);
                assert_eq!(root.app.gpui_explorer_input().unwrap().1, "kept");
                old
            })
            .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert_eq!(root.explorer_input.state(), Some(&old.1));
            assert!(!matches(root, &old));
            let current = (
                root.app.gpui_explorer_input().unwrap().0,
                root.explorer_input.state().unwrap().clone(),
            );
            root.dispatch(GpuiAction::OpenHelp, cx);
            clear(root, &current, w, cx);
            assert!(root.app.gpui_explorer_input().is_none());
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_explorer_retains_oversize_and_defers_clear_during_ime(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            prepare(&mut root, dir.path().join("project"), cx);
            root
        });
        cx.refresh().unwrap();
        let input = view
            .update(cx, |root, w, cx| {
                let input = root.explorer_input.state().unwrap().clone();
                input.update(cx, |s, cx| {
                    s.replace_text_in_range(None, &"x".repeat(LIMIT + 1), w, cx)
                });
                root.poll_backend(cx);
                input
            })
            .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert_eq!(root.explorer_input.state(), Some(&input));
            assert_eq!(input.read(cx).as_str().len(), LIMIT + 1);
            assert_eq!(root.app.gpui_explorer_input().unwrap().1, "");
            let token = (root.app.gpui_explorer_input().unwrap().0, input.clone());
            clear(root, &token, w, cx);
            assert!(input.read(cx).as_str().is_empty());
            input.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(None, "日", None, w, cx)
            });
            clear(root, &token, w, cx);
            assert_eq!(input.read(cx).as_str(), "日");
        })
        .unwrap();
    }
}
