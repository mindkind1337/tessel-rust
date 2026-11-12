//! Scoped native task-assignee combobox; selection never grants project authority.
use super::{TesselGpuiRoot, theme::GpuiThemeTokens};
use crate::{
    app::gpui_bridge::GpuiAction,
    native_task_board::{Command, Scope},
    task_agents_model::Choice,
    workspace::Task,
};
use gpui::{App, Context, FocusHandle, anchored, deferred, div, point, prelude::*, px};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
#[derive(Default)]
pub(crate) struct Inputs {
    scope: Option<Scope>,
    blocked: bool,
    handles: HashMap<u64, FocusHandle>,
    bounds: HashMap<u64, Rc<Cell<Option<gpui::Bounds<gpui::Pixels>>>>>,
    menu: Option<Menu>,
}
struct Menu {
    scope: Scope,
    id: u64,
    selected: Option<u64>,
    prefix: String,
    typed: Instant,
    scroll: gpui::ScrollHandle,
}
impl Inputs {
    pub fn sync(&mut self, scope: Option<Scope>, tasks: &[Task], blocked: bool, cx: &App) {
        if self.scope != scope {
            self.handles.clear();
            self.bounds.clear();
            self.menu = None;
            self.scope = scope;
        }
        self.blocked = blocked;
        let ids: HashSet<_> = tasks
            .iter()
            .filter(|t| t.column <= 1)
            .map(|t| t.id)
            .collect();
        self.handles.retain(|id, _| ids.contains(id));
        self.bounds.retain(|id, _| ids.contains(id));
        for task in tasks.iter().filter(|t| t.column <= 1) {
            self.bounds
                .entry(task.id)
                .or_insert_with(|| Rc::new(Cell::new(None)));
            self.handles
                .entry(task.id)
                .or_insert_with(|| cx.focus_handle());
        }
        if blocked
            || self
                .menu
                .as_ref()
                .is_some_and(|m| !self.handles.contains_key(&m.id))
        {
            self.menu = None;
        }
    }
    fn open(&mut self, scope: Scope, id: u64, pane: Option<u64>) {
        if !self.blocked && self.scope == Some(scope) && self.handles.contains_key(&id) {
            self.menu = Some(Menu {
                scope,
                id,
                selected: pane,
                prefix: String::new(),
                typed: Instant::now(),
                scroll: gpui::ScrollHandle::new(),
            });
        }
    }
    fn is_open(&self, scope: Scope, id: u64) -> bool {
        self.menu
            .as_ref()
            .is_some_and(|m| m.scope == scope && m.id == id)
    }
}
fn choose(
    root: &mut TesselGpuiRoot,
    scope: Scope,
    id: u64,
    pane: Option<u64>,
    cx: &mut Context<TesselGpuiRoot>,
) {
    if root.task_agent_inputs.is_open(scope, id) {
        root.task_agent_inputs.menu = None;
        root.dispatch_action(
            GpuiAction::NativeTaskBoard {
                scope,
                command: Command::Assign { id, pane },
            },
            cx,
        );
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn render(
    inputs: &Inputs,
    scope: Scope,
    task: &Task,
    choices: Arc<Vec<Choice>>,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> gpui::AnyElement {
    let id = task.id;
    let assigned = choices.iter().find(|p| Some(p.id) == task.timeline.pane_id);
    let no_agent = if french { "Aucun agent" } else { "No agent" };
    let unassigned = if french {
        "Non assignée"
    } else {
        "Unassigned"
    };
    if task.column > 1 {
        use gpui::StyledImage;
        let fallback_color = tokens.text;
        return if let Some(agent) = assigned {
            let kind = agent.provider.clone();
            let icon = if crate::agent_brand_assets::canonical(&kind).is_some() {
                gpui::img(move |window: &mut gpui::Window, _: &mut gpui::App| {
                    Some(
                        crate::agent_brand_assets::raster(&kind, window.scale_factor())
                            .ok_or_else(|| gpui::ImageCacheError::Asset("embedded brand".into())),
                    )
                })
                .with_fallback(move || {
                    gpui::svg()
                        .data(crate::agent_brand_assets::FALLBACK_SVG)
                        .size(px(13.))
                        .text_color(fallback_color)
                        .into_any_element()
                })
                .size(px(13.))
                .flex_shrink_0()
                .into_any_element()
            } else {
                gpui::svg()
                    .data(crate::agent_brand_assets::FALLBACK_SVG)
                    .size(px(13.))
                    .flex_shrink_0()
                    .text_color(tokens.text)
                    .into_any_element()
            };
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .min_w_0()
                .text_size(px(11.5))
                .text_color(tokens.text)
                .child(icon)
                .child(div().truncate().child(agent.label.clone()))
                .into_any_element()
        } else {
            div()
                .text_size(px(11.5))
                .text_color(tokens.text_dim)
                .child(no_agent)
                .into_any_element()
        };
    }
    let Some(handle) = inputs.handles.get(&id).cloned() else {
        return div().into_any_element();
    };
    let pane = task.timeline.pane_id;
    // Keep an absent pane's identity. A blank legacy choice is never auto-unassigned.
    let label = assigned.map(|p| p.label.clone()).unwrap_or_else(|| {
        if pane.is_some() {
            "\u{a0}".into()
        } else {
            unassigned.into()
        }
    });
    let open = inputs.is_open(scope, id);
    let button_bounds = inputs.bounds.get(&id).unwrap().clone();
    let paint_bounds = button_bounds.clone();
    let focused = handle.clone();
    let key_choices = choices.clone();
    let mut button = div()
        .relative()
        .id(("task-agent", id))
        .debug_selector(move || format!("task-agent-{id}"))
        .track_focus(&handle)
        .tab_stop(true)
        .role(gpui::Role::ComboBox)
        .aria_expanded(open)
        .aria_label(if french {
            "L'agent qui fait cette tâche"
        } else {
            "The agent doing this task"
        })
        .h(px(24.))
        .min_w(px(80.))
        .max_w_full()
        .px(px(6.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(4.))
        .border_1()
        .border_color(tokens.border)
        .text_size(px(11.))
        .text_color(tokens.text_dim)
        .cursor_pointer()
        .focus(|s| s.border_color(tokens.accent))
        .hover(|s| s.bg(tokens.surface_3))
        .on_click(cx.listener(move |root, event: &gpui::ClickEvent, w, cx| {
            if matches!(event, gpui::ClickEvent::Keyboard(_)) {
                return;
            }
            w.focus(&handle, cx);
            if root.task_agent_inputs.is_open(scope, id) {
                root.task_agent_inputs.menu = None;
            } else {
                root.task_agent_inputs.open(scope, id, pane);
            }
            cx.notify();
        }))
        .on_key_down(cx.listener(move |root, event: &gpui::KeyDownEvent, w, cx| {
            if !focused.is_focused(w) {
                return;
            }
            let key = &event.keystroke;
            if key.modifiers.control || key.modifiers.platform || key.modifiers.alt {
                return;
            }
            if key.key == "tab" {
                root.task_agent_inputs.menu = None;
                return;
            }
            let was_open = root.task_agent_inputs.is_open(scope, id);
            if key.key == "escape" {
                if was_open {
                    root.task_agent_inputs.menu = None;
                    cx.stop_propagation();
                    cx.notify();
                }
                return;
            }
            let navigation = matches!(
                key.key.as_str(),
                "up" | "down" | "home" | "end" | "enter" | "space"
            );
            let typed = key.key.chars().count() == 1;
            if !navigation && !typed {
                return;
            }
            cx.stop_propagation();
            if event.is_held && matches!(key.key.as_str(), "enter" | "space") {
                return;
            }
            if !was_open {
                root.task_agent_inputs.open(scope, id, pane);
                if navigation {
                    if let Some(m) = &mut root.task_agent_inputs.menu {
                        if matches!(key.key.as_str(), "up" | "end") {
                            m.selected = key_choices.last().map(|p| p.id);
                            m.scroll.scroll_to_item(key_choices.len());
                        } else if key.key == "home" {
                            m.selected = None;
                        }
                    }
                    cx.notify();
                    return;
                }
            }
            if was_open && matches!(key.key.as_str(), "enter" | "space") {
                if let Some(m) = &root.task_agent_inputs.menu {
                    let selected = m.selected;
                    choose(root, scope, id, selected, cx);
                }
                return;
            }
            if let Some(m) = root
                .task_agent_inputs
                .menu
                .as_mut()
                .filter(|m| m.scope == scope && m.id == id)
            {
                let values: Vec<Option<u64>> = std::iter::once(None)
                    .chain(key_choices.iter().map(|p| Some(p.id)))
                    .collect();
                let index = values.iter().position(|p| *p == m.selected).unwrap_or(0);
                m.selected = match key.key.as_str() {
                    "home" => None,
                    "end" => *values.last().unwrap(),
                    "down" => values[(index + 1).min(values.len() - 1)],
                    "up" => values[index.saturating_sub(1)],
                    _ => m.selected,
                };
                if typed && key.key != " " {
                    if m.typed.elapsed() > Duration::from_millis(700) {
                        m.prefix.clear();
                    }
                    if m.prefix.len() < 128 {
                        m.prefix.push_str(&key.key.to_lowercase());
                    }
                    m.typed = Instant::now();
                    let first = m.prefix.chars().next().unwrap_or_default();
                    let needle = if m.prefix.chars().all(|c| c == first) {
                        first.to_string()
                    } else {
                        m.prefix.clone()
                    };
                    let start = if needle.chars().count() == 1 {
                        index + 1
                    } else {
                        index
                    };
                    for n in 0..values.len() {
                        let i = (start + n) % values.len();
                        let label = if i == 0 {
                            unassigned
                        } else {
                            key_choices[i - 1].label.as_str()
                        };
                        if label.to_lowercase().starts_with(&needle) {
                            m.selected = values[i];
                            break;
                        }
                    }
                }
            }
            if let Some(m) = &root.task_agent_inputs.menu {
                let index = m
                    .selected
                    .and_then(|id| key_choices.iter().position(|p| p.id == id).map(|i| i + 1))
                    .unwrap_or(0);
                m.scroll.scroll_to_item(index);
            }
            cx.notify();
        }))
        .child(div().flex_1().min_w_0().truncate().child(label))
        .child("⌄")
        .child(
            gpui::canvas(
                move |bounds, _, _| paint_bounds.set(Some(bounds)),
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        );
    if open {
        let selected = inputs.menu.as_ref().and_then(|m| m.selected);
        let mut menu = div()
            .id(("task-agent-menu", id))
            .role(gpui::Role::ListBox)
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, _, cx| cx.stop_propagation())
            .debug_selector(move || format!("task-agent-menu-{id}"))
            .w(px(220.))
            .max_h(px(240.))
            .overflow_y_scroll()
            .track_scroll(&inputs.menu.as_ref().unwrap().scroll)
            .flex()
            .flex_col()
            .p(px(4.))
            .rounded(px(5.))
            .border_1()
            .border_color(tokens.border)
            .bg(tokens.surface_2)
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(
                cx.listener(move |root, event: &gpui::MouseDownEvent, _, cx| {
                    if button_bounds
                        .get()
                        .is_some_and(|b| b.contains(&event.position))
                    {
                        return;
                    }
                    if root.task_agent_inputs.is_open(scope, id) {
                        root.task_agent_inputs.menu = None;
                        cx.notify();
                    }
                }),
            );
        for (value, label) in std::iter::once((None, unassigned.to_owned()))
            .chain(choices.iter().map(|p| (Some(p.id), p.label.clone())))
        {
            menu = menu.child(
                div()
                    .id(("task-agent-option", value.unwrap_or(0)))
                    .role(gpui::Role::ListBoxOption)
                    .aria_selected(selected == value)
                    .when(selected == value, |d| d.aria_active_descendant())
                    .truncate()
                    .debug_selector(move || format!("task-agent-option-{}", value.unwrap_or(0)))
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(3.))
                    .text_size(px(11.))
                    .text_color(tokens.text)
                    .when(selected == value, |d| d.bg(tokens.accent_soft_bg))
                    .hover(|s| s.bg(tokens.surface_3))
                    .cursor_pointer()
                    .on_click(cx.listener(move |root, _, _, cx| choose(root, scope, id, value, cx)))
                    .child(label),
            );
        }
        button = button.child(
            deferred(
                anchored()
                    .offset(point(px(0.), px(26.)))
                    .snap_to_window_with_margin(px(6.))
                    .child(menu),
            )
            .priority(2),
        );
    }
    div()
        .id(("task-agent-boundary", id))
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(button)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        tasks_board_panel::TasksBoardAction,
        workspace::{SideTab, Store},
    };
    use gpui::TestAppContext;
    fn keys(cx: &mut TestAppContext, window: gpui::AnyWindowHandle, sequence: &str) {
        for key in sequence.split(' ') {
            cx.refresh().unwrap();
            let k = gpui::Keystroke::parse(key).unwrap();
            let mut v = gpui::VisualTestContext::from_window(window, cx);
            v.simulate_event(gpui::KeyDownEvent {
                keystroke: k.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            v.simulate_event(gpui::KeyUpEvent { keystroke: k });
        }
        cx.refresh().unwrap();
    }
    #[gpui::test]
    fn task_agent_native_menu_keyboard_is_scoped_and_does_not_drag(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("state");
        let (store, mut state) = Store::open(&data).unwrap();
        let project = state.add_project(dir.path()).unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let other = state.add_project(other_dir.path()).unwrap();
        state.active_project = Some(project);
        for (id, pid, name) in [(7, project, "Ada"), (8, other, "Foreign")] {
            let cwd = state
                .projects
                .iter()
                .find(|p| p.id == pid)
                .unwrap()
                .path
                .clone();
            state.sessions.push(serde_json::from_value(serde_json::json!({"id":id,"project":pid,"title":name,"launch":{"Agent":"codex"},"cwd":cwd})).unwrap());
        }
        state.next_pane = 9;
        store.save(&state).unwrap();
        drop(store);
        let view = cx.add_window(|w, cx| {
            w.activate();
            let mut root = TesselGpuiRoot::new_in_dir(Some(data), false, cx);
            root.dispatch(GpuiAction::SelectSideTab(SideTab::Tasks), cx);
            root.dispatch(
                GpuiAction::TasksBoard(TasksBoardAction::Add("Assign me".into())),
                cx,
            );
            root
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        let (id, scope) = view
            .update(cx, |root, w, cx| {
                let snap = root.app.gpui_snapshot(960., 640., 4.);
                assert_eq!(
                    snap.side_panels
                        .task_agents
                        .iter()
                        .map(|p| p.id)
                        .collect::<Vec<_>>(),
                    vec![7]
                );
                let id = snap.side_panels.tasks[0].id;
                w.focus(root.task_agent_inputs.handles.get(&id).unwrap(), cx);
                (id, snap.side_panels.task_board.scope.unwrap())
            })
            .unwrap();
        keys(cx, view.into(), "enter");
        view.update(cx, |root, _, _| {
            assert!(root.task_agent_inputs.is_open(scope, id))
        })
        .unwrap();
        keys(cx, view.into(), "down");
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.task_agent_inputs.menu.as_ref().unwrap().selected,
                Some(7)
            )
        })
        .unwrap();
        keys(cx, view.into(), "enter");
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0]
                    .timeline
                    .pane_id,
                Some(7)
            );
            assert!(root.task_agent_inputs.menu.is_none());
        })
        .unwrap();
        keys(cx, view.into(), "enter home escape");
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0]
                    .timeline
                    .pane_id,
                Some(7)
            )
        })
        .unwrap();
        let selector: &'static str = Box::leak(format!("task-agent-{id}").into_boxed_str());
        let pos = gpui::VisualTestContext::from_window(view.into(), cx)
            .debug_bounds(selector)
            .unwrap()
            .center();
        fn click(
            cx: &mut TestAppContext,
            w: gpui::AnyWindowHandle,
            pos: gpui::Point<gpui::Pixels>,
        ) {
            {
                let mut v = gpui::VisualTestContext::from_window(w, cx);
                v.simulate_mouse_move(pos, None, gpui::Modifiers::default());
                v.simulate_mouse_down(pos, gpui::MouseButton::Left, gpui::Modifiers::default());
                v.simulate_mouse_up(pos, gpui::MouseButton::Left, gpui::Modifiers::default());
            }
            cx.refresh().unwrap();
        }
        click(cx, view.into(), pos);
        view.update(cx, |root, _, _| {
            assert!(root.task_agent_inputs.is_open(scope, id))
        })
        .unwrap();
        click(cx, view.into(), pos);
        view.update(cx, |root, _, _| {
            assert!(!root.task_agent_inputs.is_open(scope, id))
        })
        .unwrap();
        for (selector, selected) in [
            ("task-agent-option-0", None),
            ("task-agent-option-7", Some(7)),
        ] {
            click(cx, view.into(), pos);
            let option = gpui::VisualTestContext::from_window(view.into(), cx)
                .debug_bounds(selector)
                .unwrap()
                .center();
            click(cx, view.into(), option);
            view.update(cx, |root, _, _| {
                assert!(
                    root.task_agent_inputs.menu.is_none(),
                    "choosing must not reopen menu"
                );
                assert_eq!(
                    root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0]
                        .timeline
                        .pane_id,
                    selected
                );
            })
            .unwrap();
        }

        {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            v.simulate_mouse_down(pos, gpui::MouseButton::Left, gpui::Modifiers::default());
            v.simulate_mouse_move(
                pos + point(px(40.), px(0.)),
                Some(gpui::MouseButton::Left),
                gpui::Modifiers::default(),
            );
        }
        cx.update(|cx| assert!(cx.active_drag().is_none()));
        {
            let mut v = gpui::VisualTestContext::from_window(view.into(), cx);
            v.simulate_mouse_up(pos, gpui::MouseButton::Left, gpui::Modifiers::default());
        }
        view.update(cx, |root, _, cx| {
            root.task_agent_inputs.open(scope, id, Some(7));
            root.dispatch(GpuiAction::SelectProject(other), cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, _, cx| {
            assert!(root.task_agent_inputs.menu.is_none());
            choose(root, scope, id, None, cx);
            root.dispatch(
                GpuiAction::NativeTaskBoard {
                    scope,
                    command: Command::Assign { id, pane: Some(8) },
                },
                cx,
            );
            root.dispatch(GpuiAction::SelectProject(project), cx);
            assert_eq!(
                root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0]
                    .timeline
                    .pane_id,
                Some(7)
            );
            root.dispatch(
                GpuiAction::TasksBoard(TasksBoardAction::Move { id, column: 2 }),
                cx,
            );
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, _, cx| {
            assert!(!root.task_agent_inputs.handles.contains_key(&id));
            root.dispatch(
                GpuiAction::TasksBoard(TasksBoardAction::Assign { id, pane: None }),
                cx,
            );
            assert_eq!(
                root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0]
                    .timeline
                    .pane_id,
                Some(7)
            );
            root.dispatch(
                GpuiAction::TasksBoard(TasksBoardAction::ShowAgent { id, pane: 7 }),
                cx,
            );
            let current_scope = root
                .app
                .gpui_snapshot(960., 640., 4.)
                .side_panels
                .task_board
                .scope
                .unwrap();
            assert!(
                root.app
                    .gpui_task_agent_focus(current_scope, id, 7)
                    .is_some()
            );
        })
        .unwrap();
    }
}
