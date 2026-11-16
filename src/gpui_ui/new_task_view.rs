use super::{TesselGpuiRoot, theme::GpuiThemeTokens};
use crate::new_task_form::{Command, Field, Panel};
use gpui::{
    AnyElement, App, Context, Entity, EntityInputHandler, FocusHandle, Focusable, KeyDownEvent,
    SharedString, Subscription, Window, div, prelude::*, px,
};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

gpui::actions!(tessel_new_task, [Submit]);
struct Input {
    field: Field,
    state: Entity<EditableTextState>,
    _changed: Subscription,
}

#[derive(Default)]
pub(super) struct View {
    epoch: u64,
    inputs: Vec<Input>,
    pub(super) focus: BTreeMap<String, FocusHandle>,
    order: Vec<FocusHandle>,
    review_order: Vec<FocusHandle>,
    review_anchor: Arc<Mutex<Option<gpui::Bounds<gpui::Pixels>>>>,
    review_menu_was_open: bool,
}
impl View {
    pub fn sync(&mut self, panel: &Panel, w: &mut Window, cx: &mut Context<TesselGpuiRoot>) {
        if !panel.open {
            *self = Self::default();
            return;
        }
        if self.epoch == panel.epoch {
            return;
        }
        *self = Self {
            epoch: panel.epoch,
            ..Default::default()
        };
        for (field, value) in [
            (Field::Title, &panel.title),
            (Field::Brief, &panel.brief),
            (Field::Branch, &panel.branch),
        ] {
            let state =
                cx.new(|cx| EditableTextState::new(StringStorage::from(value.as_str()), cx));
            let epoch = panel.epoch;
            let changed = cx.subscribe(&state, move |root, input, _: &TextChanged, cx| {
                if root.new_task_view.epoch == epoch
                    && root
                        .new_task_view
                        .inputs
                        .iter()
                        .any(|i| i.field == field && i.state == input)
                {
                    root.app.new_task_command(
                        epoch,
                        Command::Edit(field, input.read(cx).as_str().to_owned()),
                    );
                    cx.notify();
                }
            });
            if field == Field::Title {
                state.read(cx).focus_handle(cx).focus(w, cx);
            }
            self.inputs.push(Input {
                field,
                state,
                _changed: changed,
            });
        }
    }
    pub fn cycle(&self, back: bool, w: &mut Window, cx: &mut App) {
        let order = if self.review_order.is_empty() {
            &self.order
        } else {
            &self.review_order
        };
        if order.is_empty() {
            return;
        }
        let current = order.iter().position(|f| f.is_focused(w));
        let index = if back {
            current.unwrap_or(0).wrapping_add(order.len() - 1) % order.len()
        } else {
            current.map_or(0, |i| (i + 1) % order.len())
        };
        order[index].focus(w, cx);
    }
}
fn submit_input(
    root: &mut TesselGpuiRoot,
    epoch: u64,
    input: &Entity<EditableTextState>,
    w: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    if !root.app.new_task.reviewer_menu
        && root.new_task_view.epoch == epoch
        && root.new_task_view.inputs.iter().any(|i| &i.state == input)
    {
        activate(root, epoch, Command::Start, w, cx);
    }
}
fn activate(
    root: &mut TesselGpuiRoot,
    epoch: u64,
    command: Command,
    w: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    if !root.app.new_task.accepts(epoch) {
        return;
    }
    if matches!(command, Command::Start) {
        for input in &root.new_task_view.inputs {
            if input.field == Field::Branch && !root.app.new_task.use_copy() {
                continue;
            }
            if input
                .state
                .update(cx, |s, cx| s.marked_text_range(w, cx).is_some())
            {
                return;
            }
            root.app.new_task_command(
                epoch,
                Command::Edit(input.field, input.state.read(cx).as_str().to_owned()),
            );
        }
    }
    let chose_reviewer = matches!(command, Command::Reviewer(_));
    root.app.new_task_command(epoch, command);
    if chose_reviewer
        && !root.app.new_task.reviewer_menu
        && let Some(focus) = root.new_task_view.focus.get("new-task-review-trigger")
    {
        focus.focus(w, cx);
    }
    if !root.app.new_task.open {
        root.new_task_view = View::default();
        root.focus_handle.focus(w, cx);
    }
    cx.notify();
}

#[allow(clippy::too_many_arguments)]
fn button(
    id: &str,
    label: String,
    enabled: bool,
    selected: bool,
    command: Command,
    view: &mut View,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let focus = view
        .focus
        .entry(id.into())
        .or_insert_with(|| cx.focus_handle())
        .clone();
    if enabled {
        view.order.push(focus.clone());
    }
    let epoch = view.epoch;
    let key_command = command.clone();
    div()
        .id(SharedString::from(id.to_owned()))
        .track_focus(&focus)
        .tab_stop(enabled)
        .role(gpui::Role::Button)
        .aria_disabled(!enabled)
        .aria_label(label.clone())
        .px(px(10.))
        .py(px(7.))
        .rounded(px(5.))
        .border_1()
        .border_color(if selected {
            tokens.accent
        } else {
            tokens.border
        })
        .bg(if selected {
            tokens.accent_soft_bg
        } else {
            tokens.surface
        })
        .text_color(if enabled {
            tokens.text
        } else {
            tokens.text_dim
        })
        .when(enabled, |e| {
            e.cursor_pointer().hover(|s| s.bg(tokens.surface_2))
        })
        .on_click(cx.listener(move |root, _, w, cx| {
            cx.stop_propagation();
            if enabled {
                activate(root, epoch, command.clone(), w, cx);
            }
        }))
        .on_key_down(cx.listener(move |root, e: &KeyDownEvent, w, cx| {
            if enabled
                && focus.is_focused(w)
                && !e.is_held
                && matches!(e.keystroke.key.as_str(), "enter" | "space")
            {
                cx.stop_propagation();
                activate(root, epoch, key_command.clone(), w, cx);
            }
        }))
        .child(label)
        .into_any_element()
}
fn field(
    view: &mut View,
    which: Field,
    label: &str,
    placeholder: &str,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let input = &view.inputs[which.index()];
    view.order.push(input.state.read(cx).focus_handle(cx));
    let epoch = view.epoch;
    let captured = input.state.clone();
    let area = if which == Field::Brief {
        gpui_ce_elements::editable_text::text_area("new-task-brief")
            .state(input.state.downgrade())
            .key_context("EditableText TesselNewTaskArea")
            .on_action(cx.listener(move |root, _: &Submit, w, cx| {
                submit_input(root, epoch, &captured, w, cx)
            }))
            .placeholder(placeholder.to_owned())
            .w_full()
            .h(px(84.))
            .overflow_y_scroll()
            .px(px(9.))
            .py(px(7.))
            .text_size(px(13.))
            .text_color(tokens.text)
            .into_any_element()
    } else {
        gpui_ce_elements::editable_text::text_input(if which == Field::Title {
            "new-task-title"
        } else {
            "new-task-branch"
        })
        .state(input.state.downgrade())
        .key_context("EditableText TesselNewTaskLine")
        .on_action(
            cx.listener(move |root, _: &Submit, w, cx| submit_input(root, epoch, &captured, w, cx)),
        )
        .placeholder(placeholder.to_owned())
        .w_full()
        .h(px(32.))
        .px(px(9.))
        .py(px(6.))
        .text_size(px(13.))
        .text_color(tokens.text)
        .into_any_element()
    };
    div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .child(label.to_owned())
        .child(
            div()
                .border_1()
                .border_color(tokens.border)
                .rounded(px(5.))
                .bg(tokens.surface)
                .child(area),
        )
        .into_any_element()
}

fn reviewer_popup(
    panel: &Panel,
    view: &mut View,
    french: bool,
    tokens: &GpuiThemeTokens,
    w: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) -> Option<AnyElement> {
    let worker = panel.selected_existing().map(|t| t.pane);
    let eligible: Vec<_> = panel
        .reviewers
        .iter()
        .filter(|r| Some(r.target.pane) != worker)
        .collect();
    if !panel.reviewer_menu || eligible.is_empty() {
        view.review_menu_was_open = false;
        return None;
    }
    let bounds = view.review_anchor.lock().ok().and_then(|v| *v)?;
    let viewport = w.viewport_size();
    let width = f32::from(bounds.size.width).min((f32::from(viewport.width) - 32.).max(100.));
    let height = (((eligible.len() + 1) as f32 * 38.) + 10.)
        .min(200.)
        .min((f32::from(viewport.height) - 40.).max(40.));
    let left =
        f32::from(bounds.left()).clamp(16., (f32::from(viewport.width) - width - 16.).max(16.));
    let bottom = f32::from(bounds.bottom()) + 4.;
    let top = if bottom + height <= f32::from(viewport.height) - 16. {
        bottom
    } else {
        (f32::from(bounds.top()) - height - 4.).max(16.)
    };
    let mut options = vec![(
        "new-task-review-none".to_owned(),
        if french { "personne" } else { "nobody" }.to_owned(),
        None,
    )];
    options.extend(eligible.iter().map(|r| {
        (
            format!("new-task-review-{}", r.target.pane),
            r.label.clone(),
            Some(r.target.pane),
        )
    }));
    let mut list = div()
        .id("new-task-review-menu")
        .debug_selector(|| "new-task-review-menu".into())
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(width))
        .max_h(px(height))
        .p(px(5.))
        .flex()
        .flex_col()
        .gap(px(3.))
        .overflow_y_scroll()
        .role(gpui::Role::ListBox)
        .bg(tokens.surface_2)
        .border_1()
        .border_color(tokens.border)
        .rounded(px(5.))
        .on_click(|_, _, cx| cx.stop_propagation());
    for (id, label, pane) in options {
        list = list.child(button(
            &id,
            label,
            true,
            pane == panel.reviewer,
            Command::Reviewer(pane),
            view,
            tokens,
            cx,
        ));
        let focus = view.focus.get(&id).unwrap().clone();
        if !view.review_menu_was_open && pane == panel.reviewer {
            focus.focus(w, cx);
        }
        view.review_order.push(focus);
    }
    view.review_menu_was_open = true;
    let epoch = panel.epoch;
    Some(
        div()
            .id("new-task-review-dismiss")
            .absolute()
            .size_full()
            .left(px(0.))
            .top(px(0.))
            .on_click(cx.listener(move |root, _, w, cx| {
                cx.stop_propagation();
                activate(root, epoch, Command::ReviewerMenu(false), w, cx);
            }))
            .child(list)
            .into_any_element(),
    )
}

pub(super) fn render(
    panel: &Panel,
    view: &mut View,
    french: bool,
    tokens: &GpuiThemeTokens,
    w: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) -> Option<AnyElement> {
    if !panel.open || view.inputs.len() != 3 {
        return None;
    }
    let t = |fr, en| if french { fr } else { en };
    view.order.clear();
    view.review_order.clear();
    let epoch = panel.epoch;
    let mut body = div()
        .id("new-task-body")
        .flex()
        .flex_col()
        .gap(px(10.))
        .min_h_0()
        .overflow_y_scroll()
        .child(
            div()
                .text_size(px(16.))
                .child(t("Nouvelle tâche", "New task")),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(tokens.text_dim)
                .child(format!("{} · {}", panel.name, panel.source.display())),
        )
        .child(field(
            view,
            Field::Title,
            t("Titre", "Title"),
            t("Ce qu’il faut faire", "What should be done"),
            tokens,
            cx,
        ))
        .child(field(
            view,
            Field::Brief,
            t("Consignes pour l’agent", "Instructions for the agent"),
            t(
                "Détails, fichiers à modifier ou à laisser intacts, vérifications…",
                "Details, files to touch or leave alone, checks…",
            ),
            tokens,
            cx,
        ))
        .child(t("Agent", "Agent"));
    let mut choices = div().flex().gap(px(8.));
    for choice in &panel.choices {
        choices = choices.child(button(
            &format!("new-task-agent-{}", choice.id),
            choice.caption(french),
            choice.enabled,
            choice.id == panel.provider,
            Command::Provider(choice.id.clone()),
            view,
            tokens,
            cx,
        ));
    }
    body = body.child(choices).child(div().text_size(px(11.)).text_color(tokens.text_dim).child(t("Agents natifs Codex et Claude. Les agents de terminal ne sont pas encore disponibles ici.", "Native Codex and Claude agents. Terminal agents are not available here yet.")));
    if panel.is_new() {
        body = body
            .child(t("Où il travaille", "Where it works"))
            .child(button(
                "new-task-own-copy",
                t(
                    "Dans sa propre copie — recommandé",
                    "In its own copy — recommended",
                )
                .into(),
                panel.isolation == Some(true),
                panel.use_copy(),
                Command::Isolated(true),
                view,
                tokens,
                cx,
            ));
        if panel.isolation.is_none() {
            body = body.child(t(
                "Vérification du dépôt Git…",
                "Checking the Git repository…",
            ));
        } else if panel.isolation == Some(false) {
            body = body.child(t(
                "Aucune copie Git disponible pour ce projet.",
                "A Git copy is not available for this project.",
            ));
        }
        if panel.use_copy() {
            body = body
                .child(field(
                    view,
                    Field::Branch,
                    t("Branche de départ", "Start from branch"),
                    t("la branche courante", "the current branch"),
                    tokens,
                    cx,
                ))
                .child(button(
                    "new-task-env",
                    format!(
                        "{} {}",
                        if panel.copy_env { "☑" } else { "☐" },
                        t(
                            "Copier les fichiers .env du projet",
                            "Copy the project's .env files"
                        )
                    ),
                    true,
                    panel.copy_env,
                    Command::CopyEnv(!panel.copy_env),
                    view,
                    tokens,
                    cx,
                ))
                .child(button(
                    "new-task-setup",
                    format!(
                        "{} {}",
                        if panel.run_setup { "☑" } else { "☐" },
                        t(
                            "Y lancer d’abord .tessel/setup.ps1 (si le projet en a un)",
                            "Run .tessel/setup.ps1 in it first (if the project has one)"
                        )
                    ),
                    true,
                    panel.run_setup,
                    Command::RunSetup(!panel.run_setup),
                    view,
                    tokens,
                    cx,
                ));
        }
        body = body.child(button(
            "new-task-direct",
            t(
                "Directement dans le dossier du projet",
                "Directly in the project folder",
            )
            .into(),
            true,
            !panel.use_copy(),
            Command::Isolated(false),
            view,
            tokens,
            cx,
        ));
    }
    let worker = panel.selected_existing().map(|t| t.pane);
    if panel
        .reviewers
        .iter()
        .any(|r| Some(r.target.pane) != worker)
    {
        let selected = panel
            .reviewers
            .iter()
            .find(|r| Some(r.target.pane) == panel.reviewer)
            .map(|r| r.label.clone())
            .unwrap_or_else(|| t("personne", "nobody").into());
        let anchor = view.review_anchor.clone();
        body = body.child(t("Une fois terminé, demander une revue à", "When it is done, ask for a review by"))
        .child(div().id("new-task-review-combo").relative().role(gpui::Role::ComboBox).aria_expanded(panel.reviewer_menu)
            .child(button("new-task-review-trigger", format!("{selected} ▾"), true, false, Command::ReviewerMenu(!panel.reviewer_menu), view, tokens, cx))
            .child(gpui::canvas(move |bounds, _, _| { if let Ok(mut current) = anchor.lock() { *current = Some(bounds); } }, |_, _, _, _| {}).absolute().size_full()))
        .child(div().text_size(px(11.)).text_color(tokens.text_dim).child(t("Agents natifs connectés uniquement. La demande attend la fin du travail, des approbations et du brouillon du réviseur.", "Connected native agents only. The request waits for the reviewer's work, approvals and draft to finish.")));
    }
    let review_popup = reviewer_popup(panel, view, french, tokens, w, cx);
    body = body.child(
        div()
            .text_size(px(11.5))
            .text_color(tokens.text_dim)
            .child(t(
                "Le signal de fin du premier tour place la carte en Révision. Vous pouvez aussi la déplacer manuellement.",
                "The first turn’s completion signal moves the card to Review. You can also move it manually.",
            )),
    );
    if !panel.error.is_empty() {
        body = body.child(div().text_color(tokens.danger).child(panel.error.clone()));
    }
    body = body.child(
        div()
            .flex()
            .justify_end()
            .gap(px(8.))
            .child(button(
                "new-task-cancel",
                t("Annuler", "Cancel").into(),
                true,
                false,
                Command::Cancel,
                view,
                tokens,
                cx,
            ))
            .child(button(
                "new-task-start",
                t("Démarrer la tâche", "Start task").into(),
                panel.ready(),
                true,
                Command::Start,
                view,
                tokens,
                cx,
            )),
    );
    Some(
        div()
            .id("new-task-backdrop")
            .px(px(16.))
            .py(px(20.))
            .absolute()
            .size_full()
            .left(px(0.))
            .top(px(0.))
            .bg(gpui::rgba(0x000000aa))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(move |root, _, w, cx| {
                let command = if root.app.new_task.reviewer_menu {
                    Command::ReviewerMenu(false)
                } else {
                    Command::Cancel
                };
                activate(root, epoch, command, w, cx);
            }))
            .child(
                div()
                    .id("new-task-dialog")
                    .debug_selector(|| "new-task-dialog".to_owned())
                    .w(px(620.))
                    .max_w_full()
                    .max_h_full()
                    .p(px(22.))
                    .bg(tokens.panel)
                    .text_color(tokens.text)
                    .text_size(px(13.))
                    .border_1()
                    .border_color(tokens.border)
                    .rounded(px(14.))
                    .flex()
                    .flex_col()
                    .on_click(cx.listener(move |root, _, _, cx| {
                        cx.stop_propagation();
                        root.app
                            .new_task_command(epoch, Command::ReviewerMenu(false));
                        cx.notify();
                    }))
                    .child(body),
            )
            .when_some(review_popup, |element, popup| element.child(popup))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::new_task_form::Choice;
    use gpui::{Keystroke, TestAppContext, VisualTestContext};
    #[gpui::test]
    fn new_task_form_native_input_keyboard_and_stale_epoch(cx: &mut TestAppContext) {
        let temp = tempfile::tempdir().unwrap();
        let view = cx.add_window(|w, cx| {
            w.activate();
            let mut root = TesselGpuiRoot::new_in_dir(Some(temp.path().join("state")), false, cx);
            let existing = root
                .app
                .install_existing_task_fixture(&temp.path().join("fixture"));
            let existing_id = format!("pane:{}", existing.target.pane);
            root.app.new_task.begin(
                existing.target.project,
                temp.path().join("fixture"),
                "Fixture project".into(),
                vec![Choice {
                    id: "codex".into(),
                    label: "Codex".into(),
                    enabled: true,
                }],
                vec![crate::native_review::Choice {
                    label: "Fixture reviewer".into(),
                    target: crate::native_review::Target {
                        pane: 99,
                        project: 7,
                        cwd: temp.path().join("fixture"),
                        kind: crate::native_review::Kind::Codex,
                        epoch: 1,
                    },
                }],
            );
            root.app.new_task.existing = vec![existing];
            root.app.new_task.choices.push(Choice {
                id: existing_id,
                label: "Fixture existing agent".into(),
                enabled: true,
            });
            root.app.new_task.isolation = Some(true);
            root
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        let bounds = VisualTestContext::from_window(view.into(), cx)
            .debug_bounds("new-task-dialog")
            .unwrap();
        assert!(bounds.size.width <= px(620.));
        assert!(bounds.origin.x >= px(16.) && bounds.origin.y >= px(20.));
        let old = view
            .update(cx, |root, w, cx| {
                assert!(root.app.gpui_has_open_modal());
                let input = root.new_task_view.inputs[0].state.clone();
                input.update(cx, |state, cx| {
                    state.replace_text_in_range(None, "Titre Québec", w, cx)
                });
                input
            })
            .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            root.new_task_view
                .focus
                .get("new-task-review-trigger")
                .unwrap()
                .focus(w, cx);
        })
        .unwrap();
        VisualTestContext::from_window(view.into(), cx).simulate_event(KeyDownEvent {
            keystroke: Keystroke::parse("space").unwrap(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        let menu = VisualTestContext::from_window(view.into(), cx)
            .debug_bounds("new-task-review-menu")
            .unwrap();
        assert!(menu.origin.x >= px(16.) && menu.origin.y >= px(16.));
        VisualTestContext::from_window(view.into(), cx).simulate_event(KeyDownEvent {
            keystroke: Keystroke::parse("down").unwrap(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, w, _| {
            assert!(
                root.new_task_view
                    .focus
                    .get("new-task-review-99")
                    .unwrap()
                    .is_focused(w)
            );
        })
        .unwrap();
        for key in ["up", "enter"] {
            VisualTestContext::from_window(view.into(), cx).simulate_event(KeyDownEvent {
                keystroke: Keystroke::parse(key).unwrap(),
                is_held: false,
                prefer_character_input: false,
            });
            cx.run_until_parked();
            cx.refresh().unwrap();
        }
        view.update(cx, |root, _, _| {
            assert!(!root.app.new_task.reviewer_menu);
            assert!(root.app.new_task.reviewer.is_none());
        })
        .unwrap();
        VisualTestContext::from_window(view.into(), cx).simulate_event(KeyDownEvent {
            keystroke: Keystroke::parse("space").unwrap(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.run_until_parked();
        cx.refresh().unwrap();

        VisualTestContext::from_window(view.into(), cx).simulate_event(KeyDownEvent {
            keystroke: Keystroke::parse("escape").unwrap(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            assert!(root.app.new_task.open);
            assert!(!root.app.new_task.reviewer_menu);
        })
        .unwrap();
        view.update(cx, |root, w, cx| {
            assert!(!root.app.new_task.run_setup);
            root.new_task_view
                .focus
                .get("new-task-setup")
                .unwrap()
                .focus(w, cx);
        })
        .unwrap();
        VisualTestContext::from_window(view.into(), cx).simulate_event(KeyDownEvent {
            keystroke: Keystroke::parse("space").unwrap(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            assert!(root.app.new_task.run_setup);
        })
        .unwrap();
        view.update(cx, |root, w, cx| {
            assert_eq!(root.app.new_task.title, "Titre Québec");
            root.new_task_view
                .focus
                .get("new-task-env")
                .unwrap()
                .focus(w, cx);
        })
        .unwrap();
        VisualTestContext::from_window(view.into(), cx).simulate_event(KeyDownEvent {
            keystroke: Keystroke::parse("space").unwrap(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert!(!root.app.new_task.copy_env);
            root.new_task_view
                .focus
                .get(&format!(
                    "new-task-agent-pane:{}",
                    root.app.new_task.existing[0].target.pane
                ))
                .unwrap()
                .focus(w, cx);
        })
        .unwrap();
        VisualTestContext::from_window(view.into(), cx).simulate_event(KeyDownEvent {
            keystroke: Keystroke::parse("space").unwrap(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert_eq!(
                root.app.new_task.provider,
                format!("pane:{}", root.app.new_task.existing[0].target.pane)
            );
            assert!(!root.app.new_task.use_copy());
            assert!(root.app.new_task.reviewer.is_none());
            let hidden = root.new_task_view.focus.get("new-task-setup").unwrap();
            assert!(!root.new_task_view.order.iter().any(|f| f == hidden));
            let epoch = root.app.new_task.epoch;
            activate(root, epoch, Command::Cancel, w, cx);
            assert!(!root.app.new_task.open);
            root.app.new_task.begin(
                8,
                std::env::temp_dir(),
                "Second fixture".into(),
                vec![],
                vec![],
            );
            activate(root, epoch, Command::Cancel, w, cx);
            assert!(root.app.new_task.open);
            cx.notify();
        })
        .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |root, w, cx| {
            old.update(cx, |state, cx| {
                state.replace_text_in_range(None, "Stale input", w, cx)
            });
            assert!(root.app.new_task.title.is_empty());
            root.app.new_task.error = "sentinel".into();
            submit_input(root, root.app.new_task.epoch - 1, &old, w, cx);
            assert_eq!(root.app.new_task.error, "sentinel");
        })
        .unwrap();
    }
}
