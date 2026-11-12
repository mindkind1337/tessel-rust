//! Palette and finder inputs are scoped to one opening and project.
use super::{FocusInputTarget, TesselGpuiRoot};
use crate::app::gpui_bridge::{GpuiAction, NativeSearchKind, NativeSearchScope};
use gpui::{App, AppContext, Context, Entity, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
use std::collections::HashMap;
gpui::actions!(
    tessel_native_search,
    [Accept, Insert, Previous, Next, Dismiss]
);
struct Binding {
    scope: NativeSearchScope,
    state: Entity<EditableTextState>,
    synced: String,
    too_long: bool,
    _changed: Subscription,
}
#[derive(Default)]
pub(crate) struct Inputs {
    bindings: HashMap<NativeSearchKind, Binding>,
}
fn target(kind: NativeSearchKind) -> FocusInputTarget {
    match kind {
        NativeSearchKind::Palette => FocusInputTarget::PaletteQuery,
        NativeSearchKind::Finder => FocusInputTarget::FinderQuery,
    }
}
fn oversized(kind: NativeSearchKind, text: &str) -> bool {
    kind == NativeSearchKind::Finder && text.chars().take(257).count() > 256
}
impl Inputs {
    pub fn focused(&self, window: &Window, cx: &App) -> Option<FocusInputTarget> {
        self.bindings.iter().find_map(|(kind, b)| {
            b.state
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
                .then_some(target(*kind))
        })
    }
    pub fn scope(&self, kind: NativeSearchKind) -> Option<NativeSearchScope> {
        self.bindings.get(&kind).map(|b| b.scope)
    }
    pub fn get(&self, kind: NativeSearchKind) -> Option<&Entity<EditableTextState>> {
        self.bindings.get(&kind).map(|b| &b.state)
    }
    pub fn sync(&mut self, scope: NativeSearchScope, text: &str, cx: &mut Context<TesselGpuiRoot>) {
        if self.bindings.get(&scope.kind).is_some_and(|b| {
            b.scope == scope
                && (b.state.read(cx).as_str() == text || (b.too_long && b.synced == text))
        }) {
            return;
        }
        let state = cx.new(|cx| EditableTextState::new(StringStorage::from(text), cx));
        let changed = cx.subscribe(&state, move |root, state, _: &TextChanged, cx| {
            let Some(b) = root.search_inputs.bindings.get(&scope.kind) else {
                return;
            };
            if b.scope != scope
                || b.state != state
                || root.app.gpui_native_search_input().map(|v| v.0) != Some(scope)
            {
                return;
            }
            let base = b.synced.clone();
            let too_long = oversized(scope.kind, state.read(cx).as_str());
            if let Some(b) = root.search_inputs.bindings.get_mut(&scope.kind) {
                b.too_long = too_long;
            }
            if too_long {
                cx.notify();
                return;
            }
            let text = state.read(cx).as_str().to_owned();
            if text != base {
                root.dispatch(GpuiAction::NativeSearchEdit { scope, base, text }, cx);
                acknowledge(root, scope);
            }
        });
        self.bindings.insert(
            scope.kind,
            Binding {
                scope,
                state,
                synced: text.to_owned(),
                too_long: false,
                _changed: changed,
            },
        );
    }
    pub fn render(
        &self,
        kind: NativeSearchKind,
        value: &str,
        placeholder: &str,
        french: bool,
        tokens: &super::theme::GpuiThemeTokens,
        cx: &mut Context<TesselGpuiRoot>,
    ) -> gpui::AnyElement {
        use gpui::{
            InteractiveElement, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder, px,
        };
        let Some(b) = self.bindings.get(&kind) else {
            return div()
                .child(gpui::SharedString::from(value.to_owned()))
                .into_any_element();
        };
        let scope = b.scope;
        let input =
            gpui_ce_elements::editable_text::text_input(match kind {
                NativeSearchKind::Palette => "native-palette",
                NativeSearchKind::Finder => "native-finder",
            })
            .state(b.state.downgrade())
            .placeholder(placeholder.to_owned())
            .key_context("EditableText TesselNativeSearch")
            .on_action(cx.listener(move |root, _: &Accept, w, cx| {
                act(root, scope, SearchAction::Accept(false), w, cx)
            }))
            .on_action(cx.listener(move |root, _: &Insert, w, cx| {
                act(root, scope, SearchAction::Accept(true), w, cx)
            }))
            .on_action(cx.listener(move |root, _: &Previous, w, cx| {
                act(root, scope, SearchAction::Step(-1), w, cx)
            }))
            .on_action(cx.listener(move |root, _: &Next, w, cx| {
                act(root, scope, SearchAction::Step(1), w, cx)
            }))
            .on_action(cx.listener(move |root, _: &Dismiss, w, cx| {
                act(root, scope, SearchAction::Dismiss, w, cx)
            }))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |root, _, _, _| root.focus_input = target(kind)),
            )
            .w_full()
            .h(px(28.))
            .px(px(8.))
            .py(px(5.))
            .text_size(px(11.5))
            .text_color(tokens.text_strong);
        div()
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(tokens.radius_px))
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.accent)
            .child(input)
            .when(b.too_long, |el| {
                el.child(
                    div()
                        .px(px(8.))
                        .text_size(px(11.))
                        .text_color(tokens.warn)
                        .child(if french {
                            "256 caractères maximum"
                        } else {
                            "256 characters maximum"
                        }),
                )
            })
            .into_any_element()
    }
}
fn acknowledge(root: &mut TesselGpuiRoot, scope: NativeSearchScope) {
    if let Some((current, text)) = root.app.gpui_native_search_input()
        && current == scope
        && let Some(b) = root.search_inputs.bindings.get_mut(&scope.kind)
    {
        b.synced = text.to_owned();
    }
}
enum SearchAction {
    Accept(bool),
    Step(isize),
    Dismiss,
}
fn act(
    root: &mut TesselGpuiRoot,
    scope: NativeSearchScope,
    action: SearchAction,
    window: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    use gpui::EntityInputHandler;
    if root.app.gpui_native_search_input().map(|v| v.0) != Some(scope) {
        return;
    }
    let Some(b) = root.search_inputs.bindings.get(&scope.kind) else {
        return;
    };
    if b.scope != scope {
        return;
    }
    let state = b.state.clone();
    let base = b.synced.clone();
    if state.update(cx, |s, cx| s.marked_text_range(window, cx).is_some()) {
        return;
    }
    if matches!(action, SearchAction::Dismiss) {
        root.dispatch(GpuiAction::CloseOverlays, cx);
        return;
    }
    if oversized(scope.kind, state.read(cx).as_str()) {
        if let Some(b) = root.search_inputs.bindings.get_mut(&scope.kind) {
            b.too_long = true;
        }
        cx.notify();
        return;
    }
    let query = state.read(cx).as_str().to_owned();
    if query != base {
        root.dispatch(
            GpuiAction::NativeSearchEdit {
                scope,
                base,
                text: query.clone(),
            },
            cx,
        );
        acknowledge(root, scope);
    }
    if root.app.gpui_native_search_input() != Some((scope, query.as_str())) {
        return;
    }
    match action {
        SearchAction::Accept(insert) => root.dispatch(
            GpuiAction::NativeSearchAccept {
                scope,
                query,
                choice: None,
                insert,
            },
            cx,
        ),
        SearchAction::Step(delta) => {
            root.dispatch(GpuiAction::NativeSearchStep { scope, delta }, cx)
        }
        SearchAction::Dismiss => {}
    }
    if root.app.gpui_native_search_input().map(|v| v.0) != Some(scope) {
        root.focus_input = FocusInputTarget::None;
    }
}
pub(crate) fn choose(
    root: &mut TesselGpuiRoot,
    scope: NativeSearchScope,
    query: &str,
    choice: &str,
    cx: &mut Context<TesselGpuiRoot>,
) {
    let Some(b) = root.search_inputs.bindings.get(&scope.kind) else {
        return;
    };
    if b.scope == scope && b.state.read(cx).as_str() == query {
        root.dispatch(
            GpuiAction::NativeSearchAccept {
                scope,
                query: query.into(),
                choice: Some(choice.into()),
                insert: false,
            },
            cx,
        );
        if root.app.gpui_native_search_input().map(|v| v.0) != Some(scope) {
            root.focus_input = FocusInputTarget::None;
        }
    }
}

#[cfg(all(test, feature = "visual-test"))]
mod tests {
    use super::*;
    use gpui::{EntityInputHandler, TestAppContext};
    fn bindings(cx: &mut TestAppContext) {
        cx.update(|cx| {
            use gpui_ce_elements::editable_text::actions::{
                DEFAULT_INPUT_CONTEXT, default_bindings,
            };
            cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
        });
    }
    #[gpui::test]
    fn native_palette_toggle_restores_editor_caret(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("focus.txt");
        std::fs::write(&path, "abc").unwrap();
        let doc = crate::files::Document::open(dir.path(), &path).unwrap();
        let id = doc.instance_id();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root
        });
        cx.refresh().unwrap();
        view.update(cx, |root, _, cx| {
            root.editor_input
                .get(id)
                .unwrap()
                .clone()
                .update(cx, |s, cx| s.move_to(1, cx));
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-k");
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-k");
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "x");
        view.update(cx, |root, window, cx| {
            assert_eq!(root.app.gpui_editor_document().unwrap().text, "axbc");
            assert!(root.editor_input.focused(window, cx));
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_palette_edit_navigation_ime_and_accept(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let view =
            cx.add_window(|_, cx| TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx));
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| window.focus(&root.focus_handle, cx))
            .unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-k");
        cx.refresh().unwrap();
        let input = view
            .update(cx, |root, window, cx| {
                let input = root
                    .search_inputs
                    .get(NativeSearchKind::Palette)
                    .unwrap()
                    .clone();
                assert!(input.read(cx).focus_handle(cx).is_focused(window));
                input.update(cx, |s, cx| {
                    s.replace_text_in_range(None, "th😀me", window, cx);
                    s.set_selected_text_range(2..4, window, cx);
                    s.paste(gpui::ClipboardItem::new_string("e".into()), window, cx);
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
            assert_eq!(root.app.gpui_native_search_input().unwrap().1, "theme")
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "down");
        view.update(cx, |root, window, cx| {
            assert_eq!(
                root.app
                    .gpui_snapshot(960., 640., 4.)
                    .modals
                    .palette_selected,
                1
            );
            input.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(Some(0..5), "日", None, window, cx)
            });
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "enter");
        view.update(cx, |root, window, cx| {
            assert!(root.app.gpui_is_palette_open());
            input.update(cx, |s, cx| {
                s.replace_text_in_range(None, "theme", window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "enter");
        view.update(cx, |root, _, _| assert!(!root.app.gpui_is_palette_open()))
            .unwrap();
    }
    #[gpui::test]
    fn native_palette_reopening_rejects_old_entities_and_modal_input(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx);
            root.app.apply_gpui_action(GpuiAction::ToggleCommandPalette);
            root
        });
        cx.refresh().unwrap();
        let (old, scope) = view
            .update(cx, |root, _, _| {
                (
                    root.search_inputs
                        .get(NativeSearchKind::Palette)
                        .unwrap()
                        .clone(),
                    root.app.gpui_native_search_input().unwrap().0,
                )
            })
            .unwrap();
        cx.simulate_keystrokes(view.into(), "escape");
        view.update(cx, |root, _, cx| {
            assert!(!root.app.gpui_is_palette_open());
            root.dispatch(GpuiAction::ToggleCommandPalette, cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert_ne!(
                root.search_inputs.get(NativeSearchKind::Palette),
                Some(&old)
            );
            assert_ne!(root.app.gpui_native_search_input().unwrap().0, scope);
            old.update(cx, |s, cx| s.replace_text_in_range(None, "old", window, cx));
            root.app.apply_gpui_action(GpuiAction::NativeSearchAccept {
                scope,
                query: String::new(),
                choice: Some("help".into()),
                insert: false,
            });
            assert!(root.app.gpui_is_palette_open());
            root.dispatch(GpuiAction::OpenHelp, cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(root.app.gpui_native_search_input().is_none());
            assert!(root.search_inputs.focused(window, cx).is_none());
            root.app.apply_gpui_action(GpuiAction::NativeSearchEdit {
                scope,
                base: String::new(),
                text: "hidden".into(),
            });
            assert!(
                root.app
                    .gpui_snapshot(960., 640., 4.)
                    .modals
                    .palette_query
                    .is_empty()
            );
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_finder_retains_overlong_query_rejects_stale_row_and_opens_file(
        cx: &mut TestAppContext,
    ) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::write(project.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(project.join("README.md"), "synthetic").unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app
                .apply_gpui_action(GpuiAction::AddProjectPath(project.clone()));
            root.app.install_finder_input_fixture(project);
            root
        });
        cx.refresh().unwrap();
        let (input, scope) = view
            .update(cx, |root, window, cx| {
                let scope = root.app.gpui_native_search_input().unwrap().0;
                let input = root
                    .search_inputs
                    .get(NativeSearchKind::Finder)
                    .unwrap()
                    .clone();
                input.update(cx, |s, cx| {
                    s.replace_text_in_range(None, &"é".repeat(257), window, cx)
                });
                (input, scope)
            })
            .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert_eq!(
                root.search_inputs.get(NativeSearchKind::Finder),
                Some(&input)
            );
            assert!(root.search_inputs.bindings[&NativeSearchKind::Finder].too_long);
            assert_eq!(input.read(cx).as_str().chars().count(), 257);
            input.update(cx, |s, cx| {
                s.replace_text_in_range(Some(0..257), "main", window, cx)
            });
            choose(root, scope, "", "README.md", cx); // renderer's old row, before queued edit settles
            assert!(root.app.gpui_native_search_input().is_some());
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            assert_eq!(root.app.gpui_native_search_input().unwrap().1, "main")
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "enter");
        cx.refresh().unwrap();
        for _ in 0..100 {
            let ready = view
                .update(cx, |root, _, _| {
                    root.app.poll_for_gpui();
                    root.app.gpui_editor_document().is_some()
                })
                .unwrap();
            if ready {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(root.app.gpui_native_search_input().is_none());
            assert_eq!(
                root.app.gpui_editor_document().unwrap().text,
                "fn main() {}\n"
            );
            assert!(root.editor_input.focused(window, cx));
        })
        .unwrap();
    }
}
