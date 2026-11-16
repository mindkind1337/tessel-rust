//! Native decision answers: drafts belong to a project and exact question identity.
use super::{TesselGpuiRoot, theme::GpuiThemeTokens};
use crate::{
    app::gpui_bridge::GpuiAction,
    native_task_board::{Command, Scope},
    workspace::Task,
};
use gpui::{
    App, Context, Entity, EntityInputHandler, FocusHandle, Focusable, Subscription, Window, div,
    prelude::*, px,
};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
use std::collections::HashMap;
struct Entry {
    request: u64,
    input: Entity<EditableTextState>,
    buttons: Vec<FocusHandle>,
    _changed: Subscription,
}
#[derive(Default)]
pub(crate) struct Inputs {
    scope: Option<Scope>,
    entries: HashMap<u64, Entry>,
    blocked: bool,
}
impl Inputs {
    pub fn sync(
        &mut self,
        scope: Option<Scope>,
        tasks: &[Task],
        blocked: bool,
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        if self.scope != scope {
            self.entries.clear();
            self.scope = scope;
        }
        self.blocked = blocked;
        if scope.is_none() {
            return;
        }
        let cards: HashMap<_, _> = tasks.iter().map(|task| (task.id, task)).collect();
        self.entries.retain(|id, e| {
            cards.get(id).is_some_and(|t| {
                Some(t.project) == scope.map(|s| s.project)
                    && t.decision
                        .as_ref()
                        .is_some_and(|d| d.pending() && d.request == e.request)
            })
        });
        for task in tasks
            .iter()
            .filter(|t| Some(t.project) == scope.map(|s| s.project))
        {
            if let Some(decision) = &task.decision
                && decision.pending()
                && !self.entries.contains_key(&task.id)
            {
                let input = cx.new(|cx| EditableTextState::new(StringStorage::from(""), cx));
                let changed = cx.subscribe(&input, |_, _, _: &TextChanged, cx| cx.notify());
                let buttons = (0..=decision.options.len())
                    .map(|_| cx.focus_handle())
                    .collect();
                self.entries.insert(
                    task.id,
                    Entry {
                        request: decision.request,
                        input,
                        buttons,
                        _changed: changed,
                    },
                );
            }
        }
    }
    pub fn focused(&self, w: &Window, cx: &App) -> bool {
        self.entries
            .values()
            .any(|e| e.input.read(cx).focus_handle(cx).is_focused(w))
    }
}
#[derive(Clone)]
enum Reply {
    Choice(String),
    Draft(Entity<EditableTextState>),
}
fn submit(
    root: &mut TesselGpuiRoot,
    scope: Scope,
    id: u64,
    request: u64,
    reply: &Reply,
    w: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    if root.app.gpui_has_open_modal() || !w.is_window_active() {
        return;
    }
    let answer = match reply {
        Reply::Choice(text) => text.clone(),
        Reply::Draft(input) => {
            if input.update(cx, |s, cx| s.marked_text_range(w, cx).is_some()) {
                return;
            }
            input.read(cx).as_str().to_owned()
        }
    };
    root.dispatch_action(
        GpuiAction::NativeTaskBoard {
            scope,
            command: Command::AnswerDecision {
                id,
                request,
                answer,
            },
        },
        cx,
    );
}
#[allow(clippy::too_many_arguments)]
fn button(
    label: String,
    scope: Scope,
    id: u64,
    request: u64,
    index: usize,
    reply: Reply,
    handle: &FocusHandle,
    blocked: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> gpui::AnyElement {
    let click = reply.clone();
    let focus = handle.clone();
    div()
        .id(format!("decision-{id}-{request}-{index}"))
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .track_focus(handle)
        .tab_stop(!blocked)
        .px(px(8.))
        .py(px(4.))
        .rounded(px(4.))
        .bg(tokens.accent_soft_bg)
        .text_color(tokens.text)
        .text_size(px(11.5))
        .cursor_pointer()
        .when(blocked, |el| el.opacity(0.4))
        .on_click(cx.listener(move |root, _, w, cx| {
            cx.stop_propagation();
            if !blocked {
                submit(root, scope, id, request, &click, w, cx);
            }
        }))
        .on_key_down(cx.listener(move |root, event: &gpui::KeyDownEvent, w, cx| {
            if !blocked
                && focus.is_focused(w)
                && !event.is_held
                && matches!(event.keystroke.key.as_str(), "enter" | "space")
            {
                cx.stop_propagation();
                submit(root, scope, id, request, &reply, w, cx);
            }
        }))
        .child(label)
        .into_any_element()
}
pub(crate) fn render(
    inputs: &Inputs,
    scope: Scope,
    task: &Task,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> gpui::AnyElement {
    let Some(decision) = &task.decision else {
        return div().into_any_element();
    };
    let task_id = task.id;
    let mut body = div()
        .id(("task-decision", task.id))
        .debug_selector(move || format!("task-decision-{task_id}"))
        .flex()
        .flex_col()
        .gap(px(5.))
        .p(px(7.))
        .border_1()
        .border_color(tokens.border)
        .rounded(px(4.))
        .text_size(px(12.))
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(div().text_color(tokens.warn).child(if decision.pending() {
            if french {
                "Votre décision est requise"
            } else {
                "Your decision is needed"
            }
        } else if french {
            "Décision répondue"
        } else {
            "Decision answered"
        }))
        .child(div().whitespace_normal().child(decision.question.clone()));
    if let Some(answer) = &decision.answer {
        return body
            .child(div().whitespace_normal().child(answer.text.clone()))
            .into_any_element();
    }
    let Some(entry) = inputs
        .entries
        .get(&task.id)
        .filter(|e| e.request == decision.request)
    else {
        return body.into_any_element();
    };
    let mut choices = div().flex().flex_wrap().gap(px(4.));
    for (index, option) in decision.options.iter().enumerate() {
        choices = choices.child(button(
            option.clone(),
            scope,
            task.id,
            decision.request,
            index,
            Reply::Choice(option.clone()),
            &entry.buttons[index],
            inputs.blocked,
            tokens,
            cx,
        ));
    }
    body = body
        .child(choices)
        .child(div().child(if french {
            "Votre réponse"
        } else {
            "Your answer"
        }))
        .child(
            gpui_ce_elements::editable_text::text_input(format!("decision-answer-{}", task.id))
                .state(entry.input.downgrade())
                .key_context("EditableText TesselTaskDecision")
                .h(px(28.))
                .px(px(6.))
                .border_1()
                .border_color(tokens.border)
                .bg(tokens.term)
                .text_color(tokens.text),
        )
        .child(button(
            if french { "Répondre" } else { "Answer" }.into(),
            scope,
            task.id,
            decision.request,
            decision.options.len(),
            Reply::Draft(entry.input.clone()),
            entry.buttons.last().unwrap(),
            inputs.blocked,
            tokens,
            cx,
        ));
    body.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{SideTab, Store};
    fn exercise(cx: &mut gpui::TestAppContext, choice: bool) {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("state");
        let (store, mut state) = Store::open(&data).unwrap();
        let project = state.add_project(dir.path()).unwrap();
        let id = state.id().unwrap();
        let request = state.id().unwrap();
        state.tasks.push(serde_json::from_value(serde_json::json!({"id":id,"project":project,"title":"Synthetic question","column":0,
            "decision":{"request":request,"question":"Continue with the fixture?","options":["Continue"],"by":null,"asked_at_ms":1,"answer":null}})).unwrap());
        store.save(&state).unwrap();
        drop(store);
        let view = cx.add_window(|w, cx| {
            w.activate();
            let mut root = TesselGpuiRoot::new_in_dir(Some(data.clone()), false, cx);
            root.dispatch(GpuiAction::SelectSideTab(SideTab::Tasks), cx);
            root
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        {
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            let selector: &'static str = Box::leak(format!("task-decision-{id}").into_boxed_str());
            assert!(
                visual.debug_bounds(selector).is_some(),
                "decision question must render"
            );
        }
        view.update(cx, |root, w, cx| {
            let entry = root.task_decision_inputs.entries.get(&id).unwrap();
            if !choice {
                let input = entry.input.clone();
                w.focus(&input.read(cx).focus_handle(cx), cx);
                input.update(cx, |s, cx| {
                    s.replace_text_in_range(None, "Québec 😀", w, cx)
                });
            }
            w.focus(&entry.buttons[if choice { 0 } else { 1 }], cx);
            assert!(
                root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0]
                    .decision
                    .as_ref()
                    .unwrap()
                    .pending(),
                "blur is not an answer"
            );
        })
        .unwrap();
        cx.refresh().unwrap();
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
        let expected = if choice { "Continue" } else { "Québec 😀" };
        view.update(cx, |root, _, _| {
            let snapshot = root.app.gpui_snapshot(960., 640., 4.);
            assert_eq!(
                snapshot.side_panels.tasks[0]
                    .decision
                    .as_ref()
                    .unwrap()
                    .answer
                    .as_ref()
                    .unwrap()
                    .text,
                expected
            );
            assert_eq!(snapshot.side_panels.tasks[0].column, 0);
        })
        .unwrap();
        let loaded: crate::workspace::Workspace =
            serde_json::from_slice(&std::fs::read(data.join("workspace.json")).unwrap()).unwrap();
        assert_eq!(
            loaded.tasks[0]
                .decision
                .as_ref()
                .unwrap()
                .answer
                .as_ref()
                .unwrap()
                .text,
            expected
        );
    }
    #[gpui::test]
    fn native_freeform_answer_requires_submit_and_persists(cx: &mut gpui::TestAppContext) {
        exercise(cx, false);
    }
    #[gpui::test]
    fn native_choice_answer_is_keyboard_accessible(cx: &mut gpui::TestAppContext) {
        exercise(cx, true);
    }
}
