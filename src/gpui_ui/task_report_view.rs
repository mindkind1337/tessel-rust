//! Task report content is display-only until file provenance is ported.
use super::{TesselGpuiRoot, theme::GpuiThemeTokens};
use crate::{
    app::gpui_bridge::GpuiAction,
    native_task_board::{Command, Scope},
    workspace::Task,
};
use gpui::{Context, FocusHandle, div, prelude::*, px};
pub fn render(
    task: &Task,
    scope: Scope,
    open: bool,
    focus: &FocusHandle,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> gpui::AnyElement {
    let report = task.report.as_ref().expect("report exists");
    let label = crate::tasks_board_panel::CardReport {
        failed: report.outcome == crate::task_report::Outcome::Failed,
        summary: String::new(),
        files: report.files.clone(),
    }
    .label(french);
    let id = task.id;
    let key_focus = focus.clone();
    let toggle = move |root: &mut TesselGpuiRoot, cx: &mut Context<TesselGpuiRoot>| {
        root.dispatch_action(
            GpuiAction::NativeTaskBoard {
                scope,
                command: Command::ToggleReport(id),
            },
            cx,
        )
    };
    let mut body = div()
        .id(("report", id))
        .flex()
        .flex_col()
        .gap(px(5.))
        .min_w_0()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .id(("report-toggle", id))
                .debug_selector(move || format!("report-toggle-{id}"))
                .track_focus(focus)
                .tab_stop(true)
                .role(gpui::Role::Button)
                .aria_expanded(open)
                .aria_label(label.clone())
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(if report.outcome == crate::task_report::Outcome::Failed {
                    tokens.warn
                } else {
                    tokens.text
                })
                .on_click(cx.listener(move |root, _, _, cx| {
                    cx.stop_propagation();
                    if !root.app.gpui_has_open_modal() {
                        toggle(root, cx);
                    }
                }))
                .on_key_down(cx.listener(move |root, event: &gpui::KeyDownEvent, w, cx| {
                    if key_focus.is_focused(w)
                        && !event.is_held
                        && matches!(event.keystroke.key.as_str(), "enter" | "space")
                    {
                        cx.stop_propagation();
                        if !root.app.gpui_has_open_modal() {
                            toggle(root, cx);
                        }
                    }
                }))
                .child(format!("{} {label}", if open { "▾" } else { "▸" })),
        );
    if open {
        let mut detail = div()
            .id(("report-body", id))
            .debug_selector(move || format!("report-body-{id}"))
            .max_h(px(240.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(5.))
            .text_size(px(12.))
            .child(div().whitespace_normal().child(report.summary.clone()));
        for file in &report.files {
            detail = detail.child(div().whitespace_normal().child(file.clone()));
        }
        body = body.child(detail);
    }
    body.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{SideTab, Store};
    #[gpui::test]
    fn report_toggles_by_keyboard_without_mutating_or_opening_report_paths(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("state");
        let (store, mut state) = Store::open(&data).unwrap();
        let project = state.add_project(dir.path()).unwrap();
        let id = state.id().unwrap();
        state.tasks.push(serde_json::from_value(serde_json::json!({"id":id,"project":project,"title":"Synthetic report","column":0,
   "report":{"outcome":"failed","summary":"**Plain report**","files":["javascript:alert(1)","../private.txt"],"by":null,"at_ms":1}})).unwrap());
        store.save(&state).unwrap();
        let before = serde_json::to_value(&state).unwrap();
        drop(store);
        let view = cx.add_window(|w, cx| {
            w.activate();
            let mut root = TesselGpuiRoot::new_in_dir(Some(data.clone()), false, cx);
            root.dispatch(GpuiAction::SelectSideTab(SideTab::Tasks), cx);
            root
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        let selector: &'static str = Box::leak(format!("report-body-{id}").into_boxed_str());
        {
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            assert!(visual.debug_bounds(selector).is_none());
        }
        view.update(cx, |root, w, cx| {
            w.focus(root.task_card_input.report_handle(id).unwrap(), cx)
        })
        .unwrap();
        for expected in [true, false] {
            {
                let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
                visual.simulate_event(gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("enter").unwrap(),
                    is_held: false,
                    prefer_character_input: false,
                });
            }
            cx.run_until_parked();
            cx.refresh().unwrap();
            {
                let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
                assert_eq!(visual.debug_bounds(selector).is_some(), expected);
            }
        }
        let after: serde_json::Value =
            serde_json::from_slice(&std::fs::read(data.join("workspace.json")).unwrap()).unwrap();
        assert_eq!(after["tasks"], before["tasks"]);
        assert_eq!(after["sessions"], before["sessions"]);
    }
}
