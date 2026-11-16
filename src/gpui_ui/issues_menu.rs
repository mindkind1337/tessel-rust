//! Anchored, keyboard-operable Issues popover; no project is required to connect Linear.
use super::{TesselGpuiRoot, theme::GpuiThemeTokens};
use crate::{app::gpui_bridge::GpuiAction, toolbar_fit::MoreMenuAction};
use gpui::{
    AnyElement, Bounds, Context, FocusHandle, KeyDownEvent, Pixels, Role, Window, canvas, div,
    prelude::*, px, svg,
};
use std::sync::{Arc, Mutex};
pub struct Menu {
    button: FocusHandle,
    popup: FocusHandle,
    items: [FocusHandle; 2],
    anchor: Arc<Mutex<Option<Bounds<Pixels>>>>,
    selected: usize,
}
impl Menu {
    pub fn new(cx: &mut Context<TesselGpuiRoot>) -> Self {
        Self {
            button: cx.focus_handle(),
            popup: cx.focus_handle(),
            items: [cx.focus_handle(), cx.focus_handle()],
            anchor: Default::default(),
            selected: 0,
        }
    }
}
fn toggle(root: &mut TesselGpuiRoot, w: &mut Window, cx: &mut Context<TesselGpuiRoot>) {
    root.dispatch_action(GpuiAction::ToggleIssuesMenu, cx);
    root.issues_menu.selected = 0;
    if root.app.gpui_issues_menu_open() {
        root.issues_menu.items[0].focus(w, cx);
    } else {
        root.issues_menu.button.focus(w, cx);
    }
}
fn choose(root: &mut TesselGpuiRoot, action: MoreMenuAction, cx: &mut Context<TesselGpuiRoot>) {
    if root.app.gpui_issues_menu_open() {
        root.dispatch_action(GpuiAction::ToolbarMore(action), cx);
    }
}
pub fn button(
    menu: &Menu,
    open: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let anchor = menu.anchor.clone();
    div()
        .id("toolbar-issues")
        .debug_selector(|| "toolbar-issues".into())
        .track_focus(&menu.button)
        .tab_stop(true)
        .role(Role::Button)
        .aria_label("Issues")
        .aria_expanded(open)
        .relative()
        .w(px(30.))
        .h(px(28.))
        .rounded(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .bg(if open {
            tokens.accent_soft_bg
        } else {
            gpui::rgba(0)
        })
        .cursor_pointer()
        .hover(|s| s.bg(tokens.surface_2))
        .on_click(cx.listener(toggle_click))
        .on_key_down(cx.listener(|root, event: &KeyDownEvent, w, cx| {
            if !event.is_held && matches!(event.keystroke.key.as_str(), "enter" | "space" | "down")
            {
                cx.stop_propagation();
                toggle(root, w, cx);
            }
        }))
        .child(
            svg()
                .data(crate::issues_menu::SVG)
                .size(px(16.))
                .text_color(tokens.text_dim),
        )
        .child(
            canvas(
                move |bounds, _, _| {
                    if let Ok(mut current) = anchor.lock() {
                        *current = Some(bounds);
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .into_any_element()
}
fn toggle_click(
    root: &mut TesselGpuiRoot,
    _: &gpui::ClickEvent,
    w: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    toggle(root, w, cx);
}
pub fn popup(
    menu: &Menu,
    open: bool,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> Option<AnyElement> {
    if !open {
        return None;
    }
    let bounds = menu.anchor.lock().ok().and_then(|v| *v)?;
    let left = (f32::from(bounds.right()) - 200.).max(0.);
    let top = f32::from(bounds.bottom()) + 5.;
    let mut list = div()
        .id("toolbar-issues-menu")
        .debug_selector(|| "toolbar-issues-menu".into())
        .track_focus(&menu.popup)
        .role(Role::Menu)
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(200.))
        .p(px(5.))
        .rounded(px(9.))
        .bg(tokens.surface_2)
        .border_1()
        .border_color(tokens.border_strong)
        .on_click(|_, _, cx| cx.stop_propagation())
        .on_key_down(cx.listener(|root, event: &KeyDownEvent, w, cx| {
            match event.keystroke.key.as_str() {
                "up" | "down" | "tab" => {
                    root.issues_menu.selected ^= 1;
                    root.issues_menu.items[root.issues_menu.selected].focus(w, cx);
                    cx.notify();
                }
                "enter" | "space" if !event.is_held => {
                    let action = crate::issues_menu::entries(false)[root.issues_menu.selected].0;
                    choose(root, action, cx);
                }
                "escape" => toggle(root, w, cx),
                _ => return,
            }
            cx.stop_propagation();
        }));
    for (index, (action, label)) in crate::issues_menu::entries(french).into_iter().enumerate() {
        list = list.child(
            div()
                .id(("issues-item", index))
                .track_focus(&menu.items[index])
                .tab_stop(true)
                .role(Role::MenuItem)
                .aria_label(label)
                .px(px(8.))
                .h(px(30.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .text_size(px(12.))
                .text_color(tokens.text)
                .bg(if index == menu.selected {
                    tokens.surface_3
                } else {
                    gpui::rgba(0)
                })
                .cursor_pointer()
                .hover(|s| s.bg(tokens.surface_3))
                .on_click(cx.listener(move |root, _, _, cx| {
                    cx.stop_propagation();
                    choose(root, action, cx);
                }))
                .child(label),
        );
    }
    Some(
        div()
            .id("toolbar-issues-backdrop")
            .absolute()
            .size_full()
            .left(px(0.))
            .top(px(0.))
            .on_click(cx.listener(toggle_click))
            .child(list)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Keystroke, TestAppContext, VisualTestContext};
    fn key(view: gpui::WindowHandle<TesselGpuiRoot>, cx: &mut TestAppContext, key: &str) {
        let mut visual = VisualTestContext::from_window(view.into(), cx);
        visual.simulate_event(KeyDownEvent {
            keystroke: Keystroke::parse(key).unwrap(),
            is_held: false,
            prefer_character_input: false,
        });
    }
    #[gpui::test]
    fn issues_menu_keyboard_works_without_project_and_opens_local_linear(cx: &mut TestAppContext) {
        let temp = tempfile::tempdir().unwrap();
        let view = cx.add_window(|w, cx| {
            w.activate();
            TesselGpuiRoot::new_in_dir(Some(temp.path().join("state")), false, cx)
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |root, w, cx| {
            assert!(root.app.state.active_project.is_none());
            root.issues_menu.button.focus(w, cx);
        })
        .unwrap();
        key(view, cx, "enter");
        cx.refresh().unwrap();
        cx.run_until_parked();
        {
            let mut visual = VisualTestContext::from_window(view.into(), cx);
            assert!(visual.debug_bounds("toolbar-issues-menu").is_some());
        }
        key(view, cx, "escape");
        view.update(cx, |root, w, _| {
            assert!(!root.app.gpui_issues_menu_open());
            assert!(root.issues_menu.button.is_focused(w));
        })
        .unwrap();
        key(view, cx, "enter");
        cx.refresh().unwrap();
        cx.run_until_parked();
        key(view, cx, "down");
        view.update(cx, |root, w, _| {
            assert!(root.issues_menu.items[1].is_focused(w))
        })
        .unwrap();
        key(view, cx, "enter");
        view.update(cx, |root, _, _| {
            assert!(!root.app.gpui_issues_menu_open());
            assert!(root.app.linear.view.open);
            assert!(!root.app.gpui_snapshot(1000., 700., 4.).modals.github_open);
            // Close authority before TempDir cleanup; no detached store operation can recreate it.
            root.app.linear = Default::default();
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            toggle(root, w, cx);
            cx.notify();
        })
        .unwrap();
        // The test platform's resize() alone deliberately does not emit the OS callback.
        cx.simulate_window_resize(view.into(), gpui::size(px(400.), px(600.)));
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |root, _, _| assert!(!root.app.gpui_issues_menu_open()))
            .unwrap();
        assert!(!temp.path().join("state/linear.dat").exists());
    }
}
