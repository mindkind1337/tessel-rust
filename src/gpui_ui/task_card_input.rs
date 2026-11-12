//! Stable card focus and project-scoped internal drag payloads.
use super::{TesselGpuiRoot, theme::GpuiThemeTokens};
use crate::{
    app::gpui_bridge::GpuiAction,
    native_task_board::{Command, Scope},
    workspace::Task,
};
use gpui::{
    App, Context, FocusHandle, IntoElement, ParentElement, Render, Styled, Window, div, px,
};
use std::collections::{HashMap, HashSet};
#[derive(Default)]
pub(crate) struct Inputs {
    scope: Option<Scope>,
    handles: HashMap<u64, FocusHandle>,
    board: Option<FocusHandle>,
    confirm: Option<[FocusHandle; 2]>,
    dialog_open: bool,
    return_focus: Option<(FocusHandle, Option<u64>)>,
}
impl Inputs {
    pub fn sync(&mut self, scope: Option<Scope>, tasks: &[Task], cx: &App) {
        if self.scope != scope {
            self.handles.clear();
            self.scope = scope;
            self.board = scope.map(|_| cx.focus_handle());
            self.confirm = scope.map(|_| [cx.focus_handle(), cx.focus_handle()]);
            self.dialog_open = false;
            self.return_focus = None;
        }
        let ids: HashSet<_> = tasks.iter().map(|t| t.id).collect();
        self.handles.retain(|id, _| ids.contains(id));
        if scope.is_some() {
            for task in tasks {
                self.handles
                    .entry(task.id)
                    .or_insert_with(|| cx.focus_handle());
            }
        }
    }
    pub fn confirm_handle(&self, index: usize) -> Option<&FocusHandle> {
        self.confirm.as_ref().and_then(|h| h.get(index))
    }
    pub fn confirm_focused(&self, w: &Window) -> bool {
        self.dialog_open
            && self
                .confirm
                .as_ref()
                .is_some_and(|h| h.iter().any(|h| h.is_focused(w)))
    }
    pub fn cycle_confirm(&self, w: &mut Window, cx: &mut App) {
        if let Some(h) = &self.confirm {
            w.focus(if h[0].is_focused(w) { &h[1] } else { &h[0] }, cx);
        }
    }
    pub fn remember_delete(&mut self, action: &GpuiAction, w: &Window, cx: &App) {
        let GpuiAction::NativeTaskBoard { scope, command } = action else {
            return;
        };
        let card = match command {
            Command::RequestDeleteOne(id) => Some(*id),
            Command::RequestDelete => None,
            _ => return,
        };
        if self.scope == Some(*scope) {
            self.return_focus = w.focused(cx).map(|h| (h, card));
        }
    }
    pub fn sync_dialog(
        &mut self,
        state: &crate::native_task_board::State,
        any_modal: bool,
        w: &mut Window,
        cx: &mut App,
    ) {
        let open = state.pending_delete.is_some();
        if open && !self.dialog_open {
            if self.return_focus.is_none() {
                self.return_focus = w.focused(cx).map(|h| (h, self.focused(w, cx)));
            }
            if let Some(h) = self.confirm_handle(0) {
                w.focus(h, cx);
            }
        } else if !open
            && self.dialog_open
            && let Some((h, card)) = self.return_focus.take()
            && !any_modal
        {
            let valid = card.map_or(state.selecting, |id| self.handles.contains_key(&id));
            if valid {
                w.focus(&h, cx);
            } else if let Some(h) = &self.board {
                w.focus(h, cx);
            }
        }
        self.dialog_open = open;
    }
    pub fn board_handle(&self) -> Option<&FocusHandle> {
        self.board.as_ref()
    }
    pub fn contains_focus(&self, w: &Window, cx: &App) -> bool {
        self.board
            .as_ref()
            .is_some_and(|h| h.contains_focused(w, cx))
    }
    pub fn handle(&self, id: u64) -> Option<&FocusHandle> {
        self.handles.get(&id)
    }
    pub fn focused(&self, w: &Window, cx: &App) -> Option<u64> {
        self.handles
            .iter()
            .find_map(|(id, h)| h.contains_focused(w, cx).then_some(*id))
    }
}
#[derive(Clone)]
pub(crate) struct Drag {
    pub scope: Scope,
    pub id: u64,
    pub title: String,
}
pub(crate) struct Ghost {
    pub title: String,
    pub tokens: GpuiThemeTokens,
}
impl Render for Ghost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(240.))
            .truncate()
            .p(px(8.))
            .bg(self.tokens.panel)
            .text_color(self.tokens.text_strong)
            .border_1()
            .border_color(self.tokens.border)
            .rounded(px(4.))
            .child(self.title.clone())
    }
}
pub(crate) fn step(
    root: &mut TesselGpuiRoot,
    scope: Scope,
    id: u64,
    delta: i8,
    cx: &mut Context<TesselGpuiRoot>,
) {
    root.dispatch_action(
        GpuiAction::NativeTaskBoard {
            scope,
            command: Command::Step { id, delta },
        },
        cx,
    );
}

/// Separate parent consumes pointer events after the actual button has seen them:
/// its click stays functional, while the draggable card never records its press.
pub(crate) fn control(
    id: String,
    label: &'static str,
    kind: Control,
    action: GpuiAction,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> gpui::AnyElement {
    use gpui::{
        AppContext, InteractiveElement, StatefulInteractiveElement, prelude::FluentBuilder,
    };
    let theme = *tokens;
    let content = match kind {
        Control::Rename => gpui::svg().data(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="black" stroke-width="1.3" stroke-linejoin="round"><path d="M11.1 2.6a1.5 1.5 0 0 1 2.1 0l.2.2a1.5 1.5 0 0 1 0 2.1L6 12.3 2.8 13.2l.9-3.2 7.4-7.4Z"/><path d="m9.8 3.9 2.3 2.3"/></svg>"#.as_slice()).size(px(13.)).text_color(tokens.text_dim).into_any_element(),
        Control::Delete => super::icons::GpuiIcon::Close.render(12., tokens.text_dim).into_any_element(),
        Control::Pick(selected) => div().size(px(15.)).border_1().border_color(tokens.accent).rounded(px(2.)).when(selected, |d| d.bg(tokens.accent).child(super::icons::GpuiIcon::Check.render(13., tokens.accent_fg))).into_any_element(),
    };
    let is_pick = matches!(kind, Control::Pick(_));
    let debug_id = id.clone();
    div()
        .id(gpui::SharedString::from(format!("{id}-boundary")))
        .flex_shrink_0()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .id(gpui::SharedString::from(id))
                .debug_selector(move || debug_id)
                .focusable()
                .tab_stop(true)
                .role(if is_pick {
                    gpui::Role::CheckBox
                } else {
                    gpui::Role::Button
                })
                .aria_label(label)
                .when(is_pick, |d| {
                    d.aria_toggled(if matches!(kind, Control::Pick(true)) {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                })
                .focus(|style| style.border_1().border_color(theme.accent))
                .w(px(if is_pick { 15. } else { 26. }))
                .h(px(if is_pick { 15. } else { 24. }))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .cursor_pointer()
                .when(!is_pick, |d| {
                    d.border_1()
                        .border_color(tokens.border)
                        .hover(|s| s.bg(tokens.surface_3))
                })
                .tooltip(move |_, cx| {
                    cx.new(|_| Tooltip {
                        label,
                        tokens: theme,
                    })
                    .into()
                })
                .on_click(cx.listener(move |root, _, w, cx| {
                    root.task_card_input.remember_delete(&action, w, cx);
                    root.dispatch_action(action.clone(), cx);
                }))
                .child(content),
        )
        .into_any_element()
}
pub(crate) fn head_control(
    id: &'static str,
    label: String,
    action: Option<GpuiAction>,
    danger: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> gpui::AnyElement {
    use gpui::{InteractiveElement, StatefulInteractiveElement, prelude::FluentBuilder};
    let enabled = action.is_some();
    div()
        .id(id)
        .debug_selector(move || id.to_owned())
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .aria_disabled(!enabled)
        .h(px(22.))
        .px(px(8.))
        .rounded(px(5.))
        .border_1()
        .border_color(if danger {
            super::theme::rgba_with_alpha(tokens.danger, 0.55)
        } else {
            tokens.border
        })
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(11.))
        .text_color(if danger {
            gpui::rgb(0xffb4b6)
        } else {
            tokens.text_dim
        })
        .when(enabled, |d| {
            d.focusable()
                .tab_stop(true)
                .cursor_pointer()
                .focus(|s| s.border_color(tokens.accent))
                .hover(|s| s.bg(tokens.surface_3))
        })
        .when(!enabled, |d| d.opacity(0.4))
        .on_click(cx.listener(move |root, _, w, cx| {
            if let Some(action) = &action {
                root.task_card_input.remember_delete(action, w, cx);
                root.dispatch_action(action.clone(), cx);
            }
        }))
        .child(label)
        .into_any_element()
}
pub(crate) fn confirm_control(
    id: &'static str,
    label: String,
    primary: bool,
    handle: &FocusHandle,
    action: GpuiAction,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> gpui::AnyElement {
    use gpui::{InteractiveElement, StatefulInteractiveElement};
    div()
        .id(id)
        .track_focus(handle)
        .tab_stop(true)
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .px(px(8.))
        .py(px(4.5))
        .rounded(px(tokens.radius_px - 1.))
        .border_1()
        .bg(if primary {
            tokens.accent_soft
        } else {
            tokens.surface
        })
        .border_color(tokens.border)
        .focus(|s| s.border_color(tokens.accent))
        .text_size(px(11.))
        .text_color(tokens.text_strong)
        .on_click(cx.listener(move |root, _, _, cx| root.dispatch_action(action.clone(), cx)))
        .child(label)
        .into_any_element()
}
#[derive(Clone, Copy)]
pub(crate) enum Control {
    Rename,
    Delete,
    Pick(bool),
}
struct Tooltip {
    label: &'static str,
    tokens: GpuiThemeTokens,
}
impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p(px(6.))
            .rounded(px(4.))
            .bg(self.tokens.panel)
            .text_color(self.tokens.text)
            .text_size(px(11.))
            .child(self.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{tasks_board_panel::TasksBoardAction, workspace::SideTab};
    use gpui::TestAppContext;
    // simulate_keystrokes sends only KeyDown; native buttons activate on KeyUp.
    fn keys(cx: &mut TestAppContext, window: gpui::AnyWindowHandle, sequence: &str) {
        for key in sequence.split(' ') {
            cx.refresh().unwrap();
            let keystroke = gpui::Keystroke::parse(key).unwrap();
            let mut v = gpui::VisualTestContext::from_window(window, cx);
            v.simulate_event(gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            v.simulate_event(gpui::KeyUpEvent { keystroke });
        }
        cx.refresh().unwrap();
    }

    #[gpui::test]
    fn card_keyboard_moves_keep_focus_and_ignore_plain_arrows_and_stale_scope(
        cx: &mut TestAppContext,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let view = cx.add_window(|w, cx| {
            w.activate();
            let mut root = TesselGpuiRoot::new_in_dir(Some(temp.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(project), cx);
            root.dispatch(GpuiAction::SelectSideTab(SideTab::Tasks), cx);
            root.dispatch(
                GpuiAction::TasksBoard(TasksBoardAction::Add("Card".into())),
                cx,
            );
            root
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        let (id, scope, handle) = view
            .update(cx, |root, w, cx| {
                let snap = root.app.gpui_snapshot(960., 640., 4.);
                let id = snap.side_panels.tasks[0].id;
                let handle = root.task_card_input.handle(id).unwrap().clone();
                w.focus(&handle, cx);
                (id, snap.side_panels.task_board.scope.unwrap(), handle)
            })
            .unwrap();
        cx.refresh().unwrap();
        {
            let selector: &'static str = Box::leak(format!("task-timing-{id}").into_boxed_str());
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            let timing = visual
                .debug_bounds(selector)
                .expect("new card date is rendered");
            let card_selector: &'static str = Box::leak(format!("task-card-{id}").into_boxed_str());
            let card = visual.debug_bounds(card_selector).unwrap();
            assert!(timing.origin.x >= card.origin.x && timing.right() <= card.right());
            assert!(timing.bottom() <= card.bottom());
        }
        for key in ["right", "shift-alt-right", "ctrl-alt-right", "alt-left"] {
            keys(cx, view.into(), key);
            view.update(cx, |root, _, _| {
                assert_eq!(
                    root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].column,
                    0
                )
            })
            .unwrap();
        }
        for (key, column) in [("alt-right", 1), ("alt-right", 2), ("alt-left", 1)] {
            keys(cx, view.into(), key);
            cx.refresh().unwrap();
            view.update(cx, |root, w, cx| {
                assert_eq!(
                    root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].column,
                    column
                );
                assert!(handle.is_focused(w));
                assert_eq!(root.task_card_input.focused(w, cx), Some(id));
            })
            .unwrap();
        }
        {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            v.simulate_event(gpui::KeyDownEvent {
                keystroke: gpui::Keystroke::parse("alt-right").unwrap(),
                is_held: true,
                prefer_character_input: false,
            });
        }
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].column,
                1
            )
        })
        .unwrap();
        // Exercise the real title drag and column body drop handlers via pointer events.
        let title_selector: &'static str = Box::leak(format!("task-title-{id}").into_boxed_str());
        let (from, to) = {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            (
                v.debug_bounds(title_selector).unwrap().center(),
                v.debug_bounds("task-drop-2").unwrap().center(),
            )
        };
        {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            v.simulate_mouse_move(from, None, gpui::Modifiers::default());
            v.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
            v.simulate_mouse_move(to, gpui::MouseButton::Left, gpui::Modifiers::default());
        }
        cx.refresh().unwrap();
        cx.update(|cx| assert!(cx.active_drag().is_some()));
        {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            v.simulate_mouse_move(to, gpui::MouseButton::Left, gpui::Modifiers::default());
            v.simulate_mouse_up(to, gpui::MouseButton::Left, gpui::Modifiers::default());
        }
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].column,
                2
            )
        })
        .unwrap();
        cx.update(|cx| assert!(cx.active_drag().is_none()));
        let from = gpui::VisualTestContext::from_window(view.into(), cx)
            .debug_bounds(title_selector)
            .unwrap()
            .center();
        {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            v.simulate_mouse_move(from, None, gpui::Modifiers::default());
            v.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
            v.simulate_mouse_move(
                from + gpui::point(px(15.), px(0.)),
                gpui::MouseButton::Left,
                gpui::Modifiers::default(),
            );
        }
        cx.refresh().unwrap();
        cx.update(|cx| assert!(cx.active_drag().is_some()));
        keys(cx, view.into(), "escape");
        cx.update(|cx| assert!(cx.active_drag().is_none()));
        gpui::VisualTestContext::from_window(view.into(), cx).simulate_mouse_up(
            from,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        view.update(cx, |root, w, cx| {
            root.dispatch(
                GpuiAction::NativeTaskBoard {
                    scope,
                    command: Command::Move { id, column: 3 },
                },
                cx,
            );
            root.dispatch(
                GpuiAction::NativeTaskBoard {
                    scope,
                    command: Command::ToggleSelection,
                },
                cx,
            );
            w.focus(&handle, cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        keys(cx, view.into(), "escape");
        view.update(cx, |root, _, cx| {
            assert!(
                !root
                    .app
                    .gpui_snapshot(960., 640., 4.)
                    .side_panels
                    .task_board
                    .selecting
            );
            root.dispatch(
                GpuiAction::NativeTaskBoard {
                    scope,
                    command: Command::Move { id, column: 2 },
                },
                cx,
            );
        })
        .unwrap();

        view.update(cx, |root, _, cx| {
            root.dispatch(GpuiAction::SelectSideTab(SideTab::Files), cx)
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert_eq!(root.task_card_input.focused(w, cx), None);
            step(root, scope, id, 1, cx);
            root.dispatch(GpuiAction::SelectSideTab(SideTab::Tasks), cx);
            assert_eq!(
                root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].column,
                2
            );
        })
        .unwrap();
    }
    #[gpui::test]
    fn card_controls_do_not_drag_or_toggle_parent_and_blank_card_area_drags(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            cx.bind_keys([
                gpui::KeyBinding::new(
                    "escape",
                    super::super::task_rename_input::Cancel,
                    Some("TesselTaskRename"),
                ),
                gpui::KeyBinding::new(
                    "enter",
                    super::super::task_rename_input::Save,
                    Some("TesselTaskRename"),
                ),
            ])
        });
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let view = cx.add_window(|w, cx| {
            w.activate();
            let mut root = TesselGpuiRoot::new_in_dir(Some(temp.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(project), cx);
            root.dispatch(GpuiAction::SelectSideTab(SideTab::Tasks), cx);
            root.dispatch(
                GpuiAction::TasksBoard(TasksBoardAction::Add("Card".into())),
                cx,
            );
            root
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        let (id, scope) = view
            .update(cx, |root, _, _| {
                let s = root.app.gpui_snapshot(960., 640., 4.);
                (
                    s.side_panels.tasks[0].id,
                    s.side_panels.task_board.scope.unwrap(),
                )
            })
            .unwrap();
        let selector =
            |prefix: &str| -> &'static str { Box::leak(format!("{prefix}-{id}").into_boxed_str()) };
        let rename = selector("task-rename");
        let card = selector("task-card");
        let pick = selector("task-pick");
        let delete = selector("task-delete");
        let button = gpui::VisualTestContext::from_window(view.into(), cx)
            .debug_bounds(rename)
            .unwrap()
            .center();
        {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            v.simulate_mouse_move(button, None, gpui::Modifiers::default());
            v.simulate_mouse_down(button, gpui::MouseButton::Left, gpui::Modifiers::default());
            v.simulate_mouse_move(
                button + gpui::point(px(40.), px(0.)),
                gpui::MouseButton::Left,
                gpui::Modifiers::default(),
            );
        }
        cx.update(|cx| {
            assert!(
                cx.active_drag().is_none(),
                "control press must not reach card drag"
            )
        });
        {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            v.simulate_mouse_up(
                button + gpui::point(px(40.), px(0.)),
                gpui::MouseButton::Left,
                gpui::Modifiers::default(),
            );
            v.simulate_mouse_move(button, None, gpui::Modifiers::default());
            v.simulate_click(button, gpui::Modifiers::default());
        }
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert!(
                root.task_rename_input.focused(w, cx),
                "rename button remains functional"
            );
            let edit = root
                .app
                .gpui_snapshot(960., 640., 4.)
                .side_panels
                .task_board
                .edit
                .unwrap();
            root.dispatch(
                GpuiAction::NativeTaskBoard {
                    scope,
                    command: Command::CancelEdit {
                        serial: edit.serial,
                    },
                },
                cx,
            );
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            w.focus(&root.task_card_input.handle(id).unwrap().clone(), cx)
        })
        .unwrap();
        cx.refresh().unwrap();
        keys(cx, view.into(), "tab enter");
        view.update(cx, |root, w, cx| {
            assert!(
                root.task_rename_input.focused(w, cx),
                "Tab then Enter activates rename"
            )
        })
        .unwrap();
        keys(cx, view.into(), "escape");
        view.update(cx, |root, w, _| {
            assert!(
                root.task_card_input.handle(id).unwrap().is_focused(w),
                "Escape returns focus to the card"
            )
        })
        .unwrap();
        keys(cx, view.into(), "tab enter");
        keys(cx, view.into(), "enter");
        view.update(cx, |root, w, _| {
            assert!(
                root.task_card_input.handle(id).unwrap().is_focused(w),
                "Save returns focus to the card"
            )
        })
        .unwrap();
        keys(cx, view.into(), "tab tab");
        let delete_focus = view.update(cx, |_, w, cx| w.focused(cx).unwrap()).unwrap();
        {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            for is_held in [false, true, true] {
                v.simulate_event(gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("space").unwrap(),
                    is_held,
                    prefer_character_input: false,
                });
            }
        }
        view.update(cx, |root, _, _| {
            assert!(
                root.app
                    .gpui_snapshot(960., 640., 4.)
                    .side_panels
                    .task_board
                    .pending_delete
                    .is_none(),
                "held keydown does not repeatedly activate"
            )
        })
        .unwrap();
        gpui::VisualTestContext::from_window(view.into(), cx).simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("space").unwrap(),
        });
        cx.refresh().unwrap();

        view.update(cx, |root, w, _| {
            assert!(
                root.app
                    .gpui_snapshot(960., 640., 4.)
                    .side_panels
                    .task_board
                    .pending_delete
                    .is_some()
            );
            assert!(
                root.task_card_input
                    .confirm_handle(0)
                    .unwrap()
                    .is_focused(w),
                "confirmation starts on Cancel"
            );
        })
        .unwrap();
        keys(cx, view.into(), "tab");
        view.update(cx, |root, w, _| {
            assert!(
                root.task_card_input
                    .confirm_handle(1)
                    .unwrap()
                    .is_focused(w)
            )
        })
        .unwrap();
        keys(cx, view.into(), "shift-tab");
        view.update(cx, |root, w, _| {
            assert!(
                root.task_card_input
                    .confirm_handle(0)
                    .unwrap()
                    .is_focused(w)
            )
        })
        .unwrap();
        keys(cx, view.into(), "escape");
        view.update(cx, |root, w, _| {
            assert!(
                root.app
                    .gpui_snapshot(960., 640., 4.)
                    .side_panels
                    .task_board
                    .pending_delete
                    .is_none()
            );
            assert!(
                delete_focus.is_focused(w),
                "closing confirmation restores its trigger"
            );
        })
        .unwrap();
        let (from, to) = {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            (
                v.debug_bounds(card).unwrap().origin + gpui::point(px(6.), px(6.)),
                v.debug_bounds("task-drop-3").unwrap().center(),
            )
        };
        {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            v.simulate_mouse_move(from, None, gpui::Modifiers::default());
            v.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
            v.simulate_mouse_move(to, gpui::MouseButton::Left, gpui::Modifiers::default());
        }
        cx.refresh().unwrap();
        cx.update(|cx| assert!(cx.active_drag().is_some(), "card padding is draggable"));
        {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            v.simulate_mouse_move(to, gpui::MouseButton::Left, gpui::Modifiers::default());
            v.simulate_mouse_up(to, gpui::MouseButton::Left, gpui::Modifiers::default());
        }
        view.update(cx, |root, _, cx| {
            assert_eq!(
                root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].column,
                3
            );
            root.dispatch(
                GpuiAction::NativeTaskBoard {
                    scope,
                    command: Command::ToggleSelection,
                },
                cx,
            );
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            w.focus(&root.task_card_input.handle(id).unwrap().clone(), cx)
        })
        .unwrap();
        cx.refresh().unwrap();
        keys(cx, view.into(), "tab space");
        view.update(cx, |root, _, _| {
            assert!(
                root.app
                    .gpui_snapshot(960., 640., 4.)
                    .side_panels
                    .task_board
                    .picked
                    .contains(&id),
                "Space activates checkbox once"
            )
        })
        .unwrap();
        keys(cx, view.into(), "space");
        view.update(cx, |root, w, cx| {
            assert!(
                root.app
                    .gpui_snapshot(960., 640., 4.)
                    .side_panels
                    .task_board
                    .picked
                    .is_empty()
            );
            w.focus(&root.task_card_input.handle(id).unwrap().clone(), cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        keys(cx, view.into(), "shift-tab space");
        view.update(cx, |root, _, cx| {
            assert!(
                !root
                    .app
                    .gpui_snapshot(960., 640., 4.)
                    .side_panels
                    .task_board
                    .selecting,
                "Shift+Tab reaches Cancel in header"
            );
            root.dispatch(
                GpuiAction::NativeTaskBoard {
                    scope,
                    command: Command::ToggleSelection,
                },
                cx,
            );
        })
        .unwrap();
        cx.refresh().unwrap();
        let checkbox = {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            assert!(
                v.debug_bounds(delete).is_none(),
                "select mode hides individual delete"
            );
            v.debug_bounds(pick).unwrap().center()
        };
        {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            v.simulate_mouse_move(checkbox, None, gpui::Modifiers::default());
            v.simulate_click(checkbox, gpui::Modifiers::default());
        }
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            assert!(
                root.app
                    .gpui_snapshot(960., 640., 4.)
                    .side_panels
                    .task_board
                    .picked
                    .contains(&id),
                "checkbox toggles once, not twice via parent"
            )
        })
        .unwrap();
        let blank = gpui::VisualTestContext::from_window(view.into(), cx)
            .debug_bounds(card)
            .unwrap()
            .origin
            + gpui::point(px(6.), px(6.));
        {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            v.simulate_mouse_move(blank, None, gpui::Modifiers::default());
            v.simulate_click(blank, gpui::Modifiers::default());
        }
        view.update(cx, |root, _, _| {
            assert!(
                root.app
                    .gpui_snapshot(960., 640., 4.)
                    .side_panels
                    .task_board
                    .picked
                    .is_empty()
            )
        })
        .unwrap();
        {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            v.simulate_mouse_down(blank, gpui::MouseButton::Left, gpui::Modifiers::default());
            v.simulate_mouse_move(
                blank + gpui::point(px(20.), px(0.)),
                gpui::MouseButton::Left,
                gpui::Modifiers::default(),
            );
        }
        cx.update(|cx| assert!(cx.active_drag().is_some()));
        gpui::VisualTestContext::from_window(view.into(), cx).deactivate_window();
        cx.refresh().unwrap();
        cx.update(|cx| {
            assert!(
                cx.active_drag().is_none(),
                "focus loss cancels the card drag"
            )
        });
    }
}
