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
}
impl Inputs {
    pub fn sync(&mut self, scope: Option<Scope>, tasks: &[Task], cx: &App) {
        if self.scope != scope {
            self.handles.clear();
            self.scope = scope;
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
    pub fn handle(&self, id: u64) -> Option<&FocusHandle> {
        self.handles.get(&id)
    }
    pub fn focused(&self, w: &Window) -> Option<u64> {
        self.handles
            .iter()
            .find_map(|(id, h)| h.is_focused(w).then_some(*id))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{tasks_board_panel::TasksBoardAction, workspace::SideTab};
    use gpui::TestAppContext;
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
        for key in ["right", "shift-alt-right", "ctrl-alt-right", "alt-left"] {
            cx.simulate_keystrokes(view.into(), key);
            view.update(cx, |root, _, _| {
                assert_eq!(
                    root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].column,
                    0
                )
            })
            .unwrap();
        }
        for (key, column) in [("alt-right", 1), ("alt-right", 2), ("alt-left", 1)] {
            cx.simulate_keystrokes(view.into(), key);
            cx.refresh().unwrap();
            view.update(cx, |root, w, _| {
                assert_eq!(
                    root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].column,
                    column
                );
                assert!(handle.is_focused(w));
                assert_eq!(root.task_card_input.focused(w), Some(id));
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
        cx.simulate_keystrokes(view.into(), "escape");
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
        cx.simulate_keystrokes(view.into(), "escape");
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
            assert_eq!(root.task_card_input.focused(w), None);
            step(root, scope, id, 1, cx);
            root.dispatch(GpuiAction::SelectSideTab(SideTab::Tasks), cx);
            assert_eq!(
                root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].column,
                2
            );
        })
        .unwrap();
    }
}
