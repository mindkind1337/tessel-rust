//! Persistent native search history, with scoped actions for each side-panel query.
use super::{FocusInputTarget, TesselGpuiRoot};
use crate::app::gpui_bridge::{GpuiAction, NativePanelQueryScope};
use gpui::{App, AppContext, Context, Entity, EntityInputHandler, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
use std::collections::HashMap;
const LIMIT: usize = 8192;
type Token = (NativePanelQueryScope, Entity<EditableTextState>);
struct Binding {
    scope: NativePanelQueryScope,
    state: Entity<EditableTextState>,
    synced: String,
    too_long: bool,
    _changed: Subscription,
}
#[derive(Default)]
pub(crate) struct Inputs {
    bindings: HashMap<FocusInputTarget, Binding>,
    active: Option<NativePanelQueryScope>,
}
gpui::actions!(tessel_panel_query, [Clear, ConsumeEnter]);
impl Inputs {
    pub fn state(&self) -> Option<&Entity<EditableTextState>> {
        let scope = self.active?;
        self.bindings
            .get(&scope.target)
            .filter(|b| b.scope == scope)
            .map(|b| &b.state)
    }
    pub fn focused(&self, w: &Window, cx: &App) -> Option<FocusInputTarget> {
        self.bindings
            .iter()
            .find(|(_, b)| b.state.read(cx).focus_handle(cx).is_focused(w))
            .map(|(t, _)| *t)
    }
    pub fn sync(
        &mut self,
        scope: NativePanelQueryScope,
        text: &str,
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        self.active = Some(scope);
        let retained = self
            .bindings
            .get(&scope.target)
            .filter(|b| b.state.read(cx).as_str() == text || (b.too_long && b.synced == text));
        if retained.is_some_and(|b| b.scope == scope) {
            return;
        }
        let too_long = retained.is_some_and(|b| b.too_long);
        let state = retained
            .map(|b| b.state.clone())
            .unwrap_or_else(|| cx.new(|cx| EditableTextState::new(StringStorage::from(text), cx)));
        let changed = cx.subscribe(&state, move |root, state, _: &TextChanged, cx| {
            if root
                .panel_query_inputs
                .bindings
                .get(&scope.target)
                .is_some_and(|b| b.scope == scope && b.state == state)
            {
                flush(root, cx);
                cx.notify();
            }
        });
        self.bindings.insert(
            scope.target,
            Binding {
                scope,
                state,
                synced: text.to_owned(),
                too_long,
                _changed: changed,
            },
        );
    }
    pub fn render(
        &self,
        target: FocusInputTarget,
        french: bool,
        tokens: &super::theme::GpuiThemeTokens,
        cx: &mut Context<TesselGpuiRoot>,
    ) -> gpui::AnyElement {
        use gpui::{
            InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
            div, prelude::FluentBuilder, px,
        };
        let Some(b) = self
            .bindings
            .get(&target)
            .filter(|b| Some(b.scope) == self.active)
        else {
            return div().into_any_element();
        };
        let expected = (b.scope, b.state.clone());
        let clicked = expected.clone();
        let button = expected.clone();
        let placeholder = match (target, french) {
            (FocusInputTarget::DashboardQuery, true) => "Rechercher un agent, une tâche, un projet",
            (FocusInputTarget::DashboardQuery, false) => "Search agents, tasks, projects",
            (FocusInputTarget::TaskHistoryQuery, true) => "Chercher dans les titres",
            (FocusInputTarget::TaskHistoryQuery, false) => "Search titles",
            (FocusInputTarget::SessionsQuery, true) => "Rechercher des conversations",
            (FocusInputTarget::SessionsQuery, false) => "Search conversations",
            (_, true) => "Chercher dans les conversations",
            (_, false) => "Search the conversations",
        };
        let field = gpui_ce_elements::editable_text::text_input("native-panel-query")
            .state(b.state.downgrade())
            .key_context("EditableText TesselPanelQuery")
            .placeholder(placeholder)
            .on_action(
                cx.listener(move |root, _: &Clear, w, cx| clear(root, &expected, w, cx, true)),
            )
            .on_action(cx.listener(|_, _: &ConsumeEnter, _, _| {}))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |root, _, _, cx| {
                    if matches(root, &clicked) {
                        root.focus_input = target;
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
                                .id("native-panel-query-clear")
                                .px(px(6.))
                                .cursor_pointer()
                                .text_color(tokens.text_dim)
                                .on_click(cx.listener(move |root, _, w, cx| {
                                    clear(root, &button, w, cx, false)
                                }))
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
fn matches(root: &TesselGpuiRoot, token: &Token) -> bool {
    root.panel_query_inputs
        .bindings
        .get(&token.0.target)
        .is_some_and(|b| {
            b.scope == token.0
                && b.state == token.1
                && root.app.gpui_panel_query_input() == Some((b.scope, b.synced.as_str()))
        })
}
pub(crate) fn flush(root: &mut TesselGpuiRoot, cx: &mut Context<TesselGpuiRoot>) {
    let Some((scope, base)) = root.app.gpui_panel_query_input() else {
        return;
    };
    let Some(b) = root
        .panel_query_inputs
        .bindings
        .get(&scope.target)
        .filter(|b| b.scope == scope && b.synced == base)
    else {
        return;
    };
    let text = b.state.read(cx).as_str();
    let too_long = text.len() > LIMIT;
    if !too_long && text != base {
        root.app
            .apply_gpui_action(GpuiAction::NativePanelQueryEdit {
                scope,
                base: base.to_owned(),
                text: text.to_owned(),
            });
        if let Some((current, text)) = root.app.gpui_panel_query_input()
            && current == scope
            && let Some(b) = root.panel_query_inputs.bindings.get_mut(&scope.target)
        {
            b.synced = text.to_owned();
        }
    }
    if let Some(b) = root.panel_query_inputs.bindings.get_mut(&scope.target) {
        b.too_long = too_long;
    }
}
fn clear(
    root: &mut TesselGpuiRoot,
    token: &Token,
    w: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
    escape: bool,
) {
    if !matches(root, token)
        || token
            .1
            .update(cx, |s, cx| s.marked_text_range(w, cx).is_some())
    {
        return;
    }
    flush(root, cx);
    if token.1.read(cx).as_str().is_empty() {
        if escape {
            root.app
                .apply_gpui_action(GpuiAction::NativePanelQueryExitFullscreen {
                    scope: token.0,
                    base: String::new(),
                });
        }
    } else {
        token.1.update(cx, |s, cx| {
            let len = s.as_str().encode_utf16().count();
            s.push_undo_stop();
            s.replace_text_in_range(Some(0..len), "", w, cx);
            s.push_undo_stop();
        });
        flush(root, cx);
    }
    root.focus_input = token.0.target;
    w.focus(&token.1.read(cx).focus_handle(cx), cx);
    cx.notify();
}

#[cfg(all(test, feature = "visual-test"))]
mod tests {
    use super::*;
    use crate::workspace::SideTab;
    use gpui::TestAppContext;
    const PANELS: [(SideTab, FocusInputTarget); 4] = [
        (SideTab::Dashboard, FocusInputTarget::DashboardQuery),
        (SideTab::TaskHistory, FocusInputTarget::TaskHistoryQuery),
        (SideTab::History, FocusInputTarget::SessionsQuery),
        (SideTab::Conversations, FocusInputTarget::ConversationsQuery),
    ];
    fn bindings(cx: &mut TestAppContext) {
        cx.update(|cx| {
            use gpui_ce_elements::editable_text::actions::{
                DEFAULT_INPUT_CONTEXT, default_bindings,
            };
            cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
        });
    }
    #[gpui::test]
    fn native_panel_queries_edit_unicode_clear_and_restore_history(cx: &mut TestAppContext) {
        bindings(cx);
        for (tab, target) in PANELS {
            let dir = tempfile::tempdir().unwrap();
            let view = cx.add_window(|_, cx| {
                let mut root =
                    TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
                root.dispatch(GpuiAction::SelectSideTab(tab), cx);
                root.focus_input = target;
                root
            });
            cx.refresh().unwrap();
            let input = view
                .update(cx, |root, w, cx| {
                    let input = root.panel_query_inputs.state().unwrap().clone();
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
            cx.simulate_keystrokes(view.into(), "enter");
            view.update(cx, |root, _, _| {
                assert_eq!(root.app.gpui_panel_query_input().unwrap().1, "alpha")
            })
            .unwrap();
            cx.simulate_keystrokes(view.into(), "escape");
            cx.refresh().unwrap();
            view.update(cx, |root, _, _| {
                assert_eq!(root.app.gpui_panel_query_input().unwrap().1, "")
            })
            .unwrap();
            cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
            cx.run_until_parked();
            view.update(cx, |root, _, cx| {
                assert_eq!(root.app.gpui_panel_query_input().unwrap().1, "alpha");
                assert_eq!(root.panel_query_inputs.state(), Some(&input));
                assert_eq!(input.read(cx).as_str(), "alpha");
            })
            .unwrap();
        }
    }
    #[gpui::test]
    fn native_panel_queries_keep_separate_history_and_reject_old_scope(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::SelectSideTab(SideTab::Dashboard), cx);
            root.focus_input = FocusInputTarget::DashboardQuery;
            root
        });
        cx.refresh().unwrap();
        let old = view
            .update(cx, |root, w, cx| {
                let token = (
                    root.app.gpui_panel_query_input().unwrap().0,
                    root.panel_query_inputs.state().unwrap().clone(),
                );
                token
                    .1
                    .update(cx, |s, cx| s.replace_text_in_range(None, "first", w, cx));
                root.dispatch(GpuiAction::SelectSideTab(SideTab::Conversations), cx);
                assert_eq!(root.app.gpui_panel_query_input().unwrap().1, "");
                assert!(!matches(root, &token));
                clear(root, &token, w, cx, true);
                token
            })
            .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            let input = root.panel_query_inputs.state().unwrap().clone();
            input.update(cx, |s, cx| s.replace_text_in_range(None, "second", w, cx));
            root.dispatch(GpuiAction::SelectSideTab(SideTab::Dashboard), cx);
            root.focus_input = FocusInputTarget::DashboardQuery;
            assert_eq!(root.app.gpui_panel_query_input().unwrap().1, "first");
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert_eq!(root.panel_query_inputs.state(), Some(&old.1));
            assert!(!matches(root, &old));
            clear(root, &old, w, cx, true);
            root.dispatch(GpuiAction::AddProjectPath(project), cx);
            assert_eq!(root.app.gpui_panel_query_input().unwrap().1, "first");
        })
        .unwrap();
        cx.refresh().unwrap();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        view.update(cx, |root, _, cx| {
            assert_eq!(root.app.gpui_panel_query_input().unwrap().1, "");
            root.dispatch(GpuiAction::SelectSideTab(SideTab::Conversations), cx);
            assert_eq!(root.app.gpui_panel_query_input().unwrap().1, "second");
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_panel_queries_guard_ime_oversize_and_empty_escape(cx: &mut TestAppContext) {
        for (tab, target) in PANELS {
            let dir = tempfile::tempdir().unwrap();
            let view = cx.add_window(|_, cx| {
                let mut root =
                    TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
                root.dispatch(GpuiAction::SelectSideTab(tab), cx);
                root.dispatch(GpuiAction::ToggleRightFullscreen, cx);
                root.focus_input = target;
                root
            });
            cx.refresh().unwrap();
            view.update(cx, |root, w, cx| {
                let token = (
                    root.app.gpui_panel_query_input().unwrap().0,
                    root.panel_query_inputs.state().unwrap().clone(),
                );
                token.1.update(cx, |s, cx| {
                    s.replace_text_in_range(None, &"x".repeat(LIMIT + 1), w, cx)
                });
                root.poll_backend(cx);
                assert_eq!(root.app.gpui_panel_query_input().unwrap().1, "");
                assert_eq!(token.1.read(cx).as_str().len(), LIMIT + 1);
                clear(root, &token, w, cx, true);
                token.1.update(cx, |s, cx| {
                    s.replace_and_mark_text_in_range(None, "日", None, w, cx)
                });
                clear(root, &token, w, cx, true);
                assert_eq!(token.1.read(cx).as_str(), "日");
                token.1.update(cx, |s, cx| {
                    s.unmark_text(w, cx);
                    s.replace_text_in_range(Some(0..1), "", w, cx);
                });
                clear(root, &token, w, cx, true);
                let fullscreen = root.app.gpui_snapshot(960., 640., 4.).right_fullscreen;
                assert_eq!(fullscreen, target == FocusInputTarget::SessionsQuery);
                root.dispatch(GpuiAction::OpenHelp, cx);
                assert!(!matches(root, &token));
            })
            .unwrap();
        }
    }
}
