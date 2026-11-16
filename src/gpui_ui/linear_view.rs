//! Local Linear dialog. The only secret-bearing object is the private input entity.
use super::{TesselGpuiRoot, private_key_input::Input, theme::GpuiThemeTokens};
use crate::linear_model::{
    LinearFilter,
    panel::{Command, Snapshot, issue_text},
};
use gpui::{
    AnyElement, App, ClipboardItem, Context, FocusHandle, KeyDownEvent, Role, SharedString,
    Subscription, Window, div, prelude::*, px,
};
use std::collections::BTreeMap;
gpui::actions!(tessel_linear, [SubmitKey]);

pub fn submit(
    root: &mut TesselGpuiRoot,
    _: &SubmitKey,
    window: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    if root.app.linear.view.open
        && root
            .linear_view
            .input
            .as_ref()
            .is_some_and(|i| i.focus_handle(cx).is_focused(window))
    {
        activate(root, root.app.linear.view.epoch, Click::Connect, window, cx);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Picker {
    Filter,
    Team,
    State,
    Agent,
    NextState,
}
#[derive(Clone)]
enum Click {
    Command(Command),
    Connect,
    Settings,
    Open,
    CopyLink,
    CopyIssue,
    Pick(Picker),
}
#[derive(Default)]
pub struct View {
    pub input: Option<Input>,
    epoch: u64,
    subscription: Option<Subscription>,
    picker: Option<Picker>,
    focus: BTreeMap<String, FocusHandle>,
    order: Vec<FocusHandle>,
}
impl View {
    pub fn sync(
        &mut self,
        snapshot: &Snapshot,
        window: &mut Window,
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        if !snapshot.open || self.epoch != snapshot.epoch {
            *self = Self {
                epoch: snapshot.epoch,
                ..Default::default()
            };
        }
        if snapshot.open
            && snapshot.identity.is_none()
            && !snapshot.loading_account
            && !snapshot.busy
        {
            if self.input.is_none() {
                let input = Input::new(cx);
                self.subscription = Some(cx.observe(&input.entity(), |_, _, cx| cx.notify()));
                input.focus(window, cx);
                self.input = Some(input);
            }
        } else {
            self.input = None;
            self.subscription = None;
        }
    }
    pub fn cycle(&self, backwards: bool, window: &mut Window, cx: &mut App) {
        if self.order.is_empty() {
            return;
        }
        let index = self.order.iter().position(|f| f.is_focused(window));
        let next = if backwards {
            index.unwrap_or(0).wrapping_add(self.order.len() - 1) % self.order.len()
        } else {
            index.map_or(0, |i| (i + 1) % self.order.len())
        };
        self.order[next].focus(window, cx);
    }
}
fn activate(
    root: &mut TesselGpuiRoot,
    epoch: u64,
    click: Click,
    w: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    if !root.app.linear.accepts(epoch) {
        return;
    }
    match click {
        Click::Command(command) => {
            root.linear_view.picker = None;
            root.app.linear.command(epoch, command);
        }
        Click::Connect => {
            if root.app.linear.view.secure != Some(true) {
                return;
            }
            if let Some(input) = root.linear_view.input.as_ref() {
                match input.take(w, cx) {
                    Ok(key) => root.app.linear.connect(epoch, key),
                    Err(error) => root.app.linear.view.error = error,
                }
            }
        }
        Click::Settings => cx.open_url("https://linear.app/settings/api"),
        Click::Pick(picker) => {
            root.linear_view.picker = (root.linear_view.picker != Some(picker)).then_some(picker)
        }
        Click::Open | Click::CopyLink | Click::CopyIssue => {
            if let Some(issue) = root.app.linear.view.selected.as_ref() {
                match click {
                    Click::Open => cx.open_url(&issue.url),
                    Click::CopyLink => {
                        cx.write_to_clipboard(ClipboardItem::new_string(issue.url.clone()))
                    }
                    Click::CopyIssue => {
                        cx.write_to_clipboard(ClipboardItem::new_string(issue_text(issue)))
                    }
                    _ => {}
                }
            }
        }
    }
    if !root.app.linear.view.open {
        root.linear_view = View::default();
        root.focus_handle.focus(w, cx);
    }
    cx.notify();
}
#[allow(clippy::too_many_arguments)]
fn button(
    id: &str,
    label: impl Into<SharedString>,
    enabled: bool,
    click: Click,
    view: &mut View,
    epoch: u64,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let id = id.to_owned();
    let focus = view
        .focus
        .entry(id.clone())
        .or_insert_with(|| cx.focus_handle())
        .clone();
    if enabled {
        view.order.push(focus.clone());
    }
    let keyboard = click.clone();
    div()
        .id(SharedString::from(id))
        .track_focus(&focus)
        .tab_stop(enabled)
        .role(Role::Button)
        .aria_disabled(!enabled)
        .px(px(8.))
        .py(px(5.))
        .rounded(px(4.))
        .border_1()
        .border_color(tokens.border)
        .bg(tokens.surface)
        .text_size(px(12.))
        .text_color(if enabled {
            tokens.text
        } else {
            tokens.text_dim
        })
        .when(enabled, |el| {
            el.cursor_pointer().hover(|s| s.bg(tokens.surface_2))
        })
        .on_click(cx.listener(move |root, _, w, cx| {
            cx.stop_propagation();
            if enabled {
                activate(root, epoch, click.clone(), w, cx);
            }
        }))
        .on_key_down(cx.listener(move |root, event: &KeyDownEvent, w, cx| {
            if enabled
                && focus.is_focused(w)
                && !event.is_held
                && matches!(event.keystroke.key.as_str(), "enter" | "space")
            {
                cx.stop_propagation();
                activate(root, epoch, keyboard.clone(), w, cx);
            }
        }))
        .child(label.into())
        .into_any_element()
}
pub fn render(
    s: &Snapshot,
    view: &mut View,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> Option<AnyElement> {
    if !s.open {
        return None;
    }
    view.order.clear();
    // Clear obsolete issue/option handles so repeated filters cannot grow the focus cache forever.
    view.focus.retain(|id, _| {
        (!id.starts_with("issue-") || s.issues.iter().any(|i| id == &format!("issue-{}", i.id)))
            && (!id.starts_with("option-") || view.picker.is_some())
    });
    let t = |fr, en| if french { fr } else { en };
    let epoch = s.epoch;
    let mut body = div()
        .id("linear-dialog-body")
        .flex()
        .flex_col()
        .gap(px(10.))
        .min_h_0()
        .overflow_y_scroll();
    body = body.child(
        div()
            .flex()
            .justify_between()
            .child(div().text_size(px(18.)).child("Linear"))
            .child(button(
                "linear-close",
                t("Fermer", "Close"),
                !s.busy,
                Click::Command(Command::Close),
                view,
                epoch,
                tokens,
                cx,
            )),
    );
    if !s.error.is_empty() {
        body = body
            .child(div().text_color(tokens.danger).child(s.error.clone()))
            .child(button(
                "linear-retry",
                t("Réessayer", "Retry"),
                !s.busy,
                Click::Command(Command::Retry),
                view,
                epoch,
                tokens,
                cx,
            ));
    }
    if s.loading_account {
        body = body.child(t("Lecture de la connexion…", "Reading connection…"));
    } else if let Some(identity) = &s.identity {
        body = body.child(
            div()
                .flex()
                .gap(px(6.))
                .items_center()
                .child(
                    identity
                        .viewer
                        .display_name
                        .as_ref()
                        .unwrap_or(&identity.viewer.name)
                        .clone(),
                )
                .child(button(
                    "linear-refresh",
                    t("Actualiser", "Refresh"),
                    !s.busy && !s.loading_issues,
                    Click::Command(Command::Refresh),
                    view,
                    epoch,
                    tokens,
                    cx,
                ))
                .child(button(
                    "linear-disconnect",
                    t("Déconnecter", "Disconnect"),
                    !s.busy,
                    Click::Command(Command::ConfirmDisconnect(true)),
                    view,
                    epoch,
                    tokens,
                    cx,
                )),
        );
        if s.confirm_disconnect {
            body = body
                .child(t(
                    "Supprimer la clé enregistrée ? Vos issues restent dans Linear.",
                    "Remove the saved key? Your issues stay in Linear.",
                ))
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(button(
                            "linear-keep",
                            t("Garder la connexion", "Keep connection"),
                            !s.busy,
                            Click::Command(Command::ConfirmDisconnect(false)),
                            view,
                            epoch,
                            tokens,
                            cx,
                        ))
                        .child(button(
                            "linear-confirm",
                            t("Déconnecter", "Disconnect"),
                            !s.busy,
                            Click::Command(Command::Disconnect),
                            view,
                            epoch,
                            tokens,
                            cx,
                        )),
                );
        }
        if let Some(issue) = &s.selected {
            body = body
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .flex_wrap()
                        .child(button(
                            "linear-back",
                            "← Issues",
                            !s.busy,
                            Click::Command(Command::Back),
                            view,
                            epoch,
                            tokens,
                            cx,
                        ))
                        .child(button(
                            "linear-open",
                            t("Ouvrir", "Open"),
                            !s.busy,
                            Click::Open,
                            view,
                            epoch,
                            tokens,
                            cx,
                        ))
                        .child(button(
                            "linear-copy-link",
                            t("Copier le lien", "Copy link"),
                            !s.busy,
                            Click::CopyLink,
                            view,
                            epoch,
                            tokens,
                            cx,
                        ))
                        .child(button(
                            "linear-copy",
                            t("Copier l’issue", "Copy issue"),
                            !s.busy,
                            Click::CopyIssue,
                            view,
                            epoch,
                            tokens,
                            cx,
                        )),
                )
                .child(
                    div()
                        .text_size(px(16.))
                        .child(format!("{} {}", issue.identifier, issue.title)),
                )
                .child(format!("{} · {}", issue.state.name, issue.team.name))
                .child(
                    div()
                        .font_family(super::theme::mono_font_family())
                        .whitespace_normal()
                        .child(issue.description.clone()),
                )
                .child(start_controls(s, view, french, tokens, cx));
        } else {
            let team = s
                .teams
                .iter()
                .find(|v| Some(&v.id) == s.selection.team_id.as_ref())
                .map(|v| v.name.as_str())
                .unwrap_or(t("Toutes les équipes", "All teams"));
            let state = s
                .states
                .iter()
                .find(|v| Some(&v.id) == s.selection.state_id.as_ref())
                .map(|v| v.name.as_str())
                .unwrap_or(t("Tous les états", "All states"));
            body = body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(button(
                        "linear-filter",
                        format!("{} ▾", s.selection.filter.label(french)),
                        !s.busy,
                        Click::Pick(Picker::Filter),
                        view,
                        epoch,
                        tokens,
                        cx,
                    ))
                    .child(button(
                        "linear-team",
                        format!("{team} ▾"),
                        !s.busy,
                        Click::Pick(Picker::Team),
                        view,
                        epoch,
                        tokens,
                        cx,
                    ))
                    .child(button(
                        "linear-state",
                        format!("{state} ▾"),
                        !s.busy && s.selection.team_id.is_some(),
                        Click::Pick(Picker::State),
                        view,
                        epoch,
                        tokens,
                        cx,
                    )),
            );
            if let Some(picker) = view.picker {
                let entries: Vec<(String, Command)> = match picker {
                    Picker::Agent | Picker::NextState => Vec::new(),
                    Picker::Filter => LinearFilter::ALL
                        .iter()
                        .map(|v| (v.label(french).into(), Command::Filter(*v)))
                        .collect(),
                    Picker::Team => std::iter::once((
                        t("Toutes les équipes", "All teams").into(),
                        Command::Team(None),
                    ))
                    .chain(
                        s.teams
                            .iter()
                            .map(|v| (v.name.clone(), Command::Team(Some(v.id.clone())))),
                    )
                    .collect(),
                    Picker::State => std::iter::once((
                        t("Tous les états", "All states").into(),
                        Command::State(None),
                    ))
                    .chain(
                        s.states
                            .iter()
                            .map(|v| (v.name.clone(), Command::State(Some(v.id.clone())))),
                    )
                    .collect(),
                };
                let mut menu = div()
                    .id("linear-options")
                    .max_h(px(160.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(3.));
                for (i, (label, command)) in entries.into_iter().enumerate() {
                    menu = menu.child(button(
                        &format!("option-{i}"),
                        label,
                        !s.busy,
                        Click::Command(command),
                        view,
                        epoch,
                        tokens,
                        cx,
                    ));
                }
                body = body.child(menu);
            }
            if s.loading_issues {
                body = body.child(t("Chargement…", "Loading…"));
            } else if s.issues.is_empty() {
                body = body.child(t(
                    "Aucune issue pour ces filtres.",
                    "No issues match these filters.",
                ));
            }
            if s.has_more {
                body = body.child(t(
                    "100 premières issues. Précisez les filtres.",
                    "First 100 issues. Narrow the filters.",
                ));
            }
            for issue in &s.issues {
                body = body.child(button(
                    &format!("issue-{}", issue.id),
                    format!(
                        "{} {}\n{} · {}",
                        issue.identifier, issue.title, issue.state.name, issue.team.name
                    ),
                    !s.busy && !s.loading_issues,
                    Click::Command(Command::Select(issue.id.clone())),
                    view,
                    epoch,
                    tokens,
                    cx,
                ));
            }
        }
    } else {
        body = body.child(t(
            "Créez une clé dans les paramètres Linear → Sécurité et accès.",
            "Create a key in Linear settings → Security & access.",
        ));
        if s.secure == Some(false) {
            body = body.child(div().text_color(tokens.warn).child(t(
                "Le stockage chiffré est indisponible sur cet ordinateur.",
                "Encrypted storage is unavailable on this computer.",
            )));
        }
        body = body.child(t("Clé API personnelle", "Personal API key"));
        if let Some(input) = &view.input {
            view.order.push(input.focus_handle(cx));
            body = body.child(input.render(french, tokens));
            if input.rejected() {
                body = body.child(div().text_color(tokens.warn).child(t(
                    "Clé refusée : utilisez au plus 4096 caractères ASCII.",
                    "Key rejected: use at most 4096 ASCII characters.",
                )));
            }
        }
        let can_connect = !s.busy
            && s.secure == Some(true)
            && view
                .input
                .as_ref()
                .is_some_and(|i| !i.is_empty(cx) && !i.rejected());
        body = body
            .child(button(
                "linear-settings",
                t("Ouvrir les paramètres Linear", "Open Linear settings"),
                !s.busy,
                Click::Settings,
                view,
                epoch,
                tokens,
                cx,
            ))
            .child(button(
                "linear-connect",
                t("Tester et enregistrer", "Test and save"),
                can_connect,
                Click::Connect,
                view,
                epoch,
                tokens,
                cx,
            ));
    }
    Some(
        div()
            .id("linear-backdrop")
            .absolute()
            .size_full()
            .left(px(0.))
            .top(px(0.))
            .bg(gpui::rgba(0x000000aa))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(move |root, _, w, cx| {
                activate(root, epoch, Click::Command(Command::Close), w, cx)
            }))
            .child(
                div()
                    .id("linear-dialog")
                    .w(px(760.))
                    .max_w_full()
                    .max_h_full()
                    .p(px(20.))
                    .m(px(20.))
                    .bg(tokens.panel)
                    .text_color(tokens.text)
                    .text_size(px(13.))
                    .border_1()
                    .border_color(tokens.border)
                    .rounded(px(8.))
                    .flex()
                    .flex_col()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(body),
            )
            .into_any_element(),
    )
}

fn start_controls(
    s: &Snapshot,
    view: &mut View,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let t = |fr, en| if french { fr } else { en };
    let agent = s
        .agents
        .iter()
        .find(|a| a.id == s.agent)
        .map(|a| a.label.as_str())
        .unwrap_or(t("Choisir un agent", "Choose agent"));
    let state = s
        .start_states
        .iter()
        .find(|v| Some(&v.id) == s.next_state.as_ref())
        .map(|v| v.name.as_str())
        .unwrap_or(t("Ne pas changer l’état", "Do not change state"));
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(if s.project_name.is_empty() {
            t(
                "Ouvrez Linear depuis un projet local pour démarrer une tâche.",
                "Open Linear from a local project to start a task.",
            )
            .to_owned()
        } else {
            format!("{} : {}", t("Projet", "Project"), s.project_name)
        })
        .child(button(
            "linear-launch-agent",
            format!("{agent} ▾"),
            !s.busy,
            Click::Pick(Picker::Agent),
            view,
            s.epoch,
            tokens,
            cx,
        ))
        .child(button(
            "linear-launch-copy",
            format!(
                "{} {}",
                if s.isolated { "☑" } else { "☐" },
                t(
                    "Créer une copie Git isolée",
                    "Create an isolated Git worktree"
                )
            ),
            !s.busy,
            Click::Command(Command::Isolated(!s.isolated)),
            view,
            s.epoch,
            tokens,
            cx,
        ))
        .child(button(
            "linear-launch-state",
            format!("{state} ▾"),
            !s.busy && !s.loading_start_states,
            Click::Pick(Picker::NextState),
            view,
            s.epoch,
            tokens,
            cx,
        ));
    let options: Vec<(String, Command, bool)> = match view.picker {
        Some(Picker::Agent) => s
            .agents
            .iter()
            .map(|a| (a.label.clone(), Command::Agent(a.id.clone()), a.available))
            .collect(),
        Some(Picker::NextState) => std::iter::once((
            t("Ne pas changer l’état", "Do not change state").to_owned(),
            Command::NextState(None),
            true,
        ))
        .chain(
            s.start_states
                .iter()
                .map(|v| (v.name.clone(), Command::NextState(Some(v.id.clone())), true)),
        )
        .collect(),
        _ => Vec::new(),
    };
    let mut menu = div()
        .id("linear-launch-options")
        .max_h(px(160.))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(3.));
    for (i, (label, command, enabled)) in options.into_iter().enumerate() {
        menu = menu.child(button(
            &format!("option-launch-{i}"),
            label,
            !s.busy && enabled,
            Click::Command(command),
            view,
            s.epoch,
            tokens,
            cx,
        ));
    }
    body = body.child(menu).child(button(
        "linear-launch-start",
        if s.busy {
            t("Préparation…", "Preparing…")
        } else {
            t("Démarrer la tâche", "Start task")
        },
        !s.busy
            && !s.project_name.is_empty()
            && s.agents.iter().any(|a| a.id == s.agent && a.available),
        Click::Command(Command::Start),
        view,
        s.epoch,
        tokens,
        cx,
    ));
    body.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linear_model::{LinearIdentity, LinearIssue, LinearPerson};
    use gpui::{EntityInputHandler, Keystroke, TestAppContext, VisualTestContext};
    #[gpui::test]
    fn native_launch_controls_accept_keyboard_and_block_busy_replays(cx: &mut TestAppContext) {
        let temp = tempfile::tempdir().unwrap();
        let view = cx.add_window(|w, cx| {
            w.activate();
            let mut root = TesselGpuiRoot::new_in_dir(Some(temp.path().join("state")), false, cx);
            root.app.install_linear_view_fixture();
            root
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |root, w, cx| {
            root.linear_view
                .focus
                .get("linear-launch-copy")
                .unwrap()
                .focus(w, cx)
        })
        .unwrap();
        VisualTestContext::from_window(view.into(), cx).simulate_event(KeyDownEvent {
            keystroke: Keystroke::parse("space").unwrap(),
            is_held: false,
            prefer_character_input: false,
        });
        view.update(cx, |root, w, cx| {
            assert!(!root.app.linear.view.isolated);
            assert!(root.app.linear.view.error.is_empty());
            root.linear_view
                .focus
                .get("linear-launch-start")
                .unwrap()
                .focus(w, cx);
        })
        .unwrap();
        VisualTestContext::from_window(view.into(), cx).simulate_event(KeyDownEvent {
            keystroke: Keystroke::parse("enter").unwrap(),
            is_held: false,
            prefer_character_input: false,
        });
        view.update(cx, |root, w, cx| {
            // The real UI polling loop already consumed the keyboard request. This
            // unbound rendering fixture must reach the controller and refuse launch.
            assert_eq!(
                root.app.linear.view.error,
                "Select a local owner project before opening Linear"
            );
            assert!(root.app.linear.requested_start.is_none());
            root.app.linear.view.busy = true;
            activate(root, 8, Click::Command(Command::Start), w, cx);
            assert!(root.app.linear.requested_start.is_none());
            root.app.linear.view.busy = false;
            activate(root, 7, Click::Command(Command::Start), w, cx);
            assert!(root.app.linear.requested_start.is_none());
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_dialog_back_keyboard_and_private_input_lifecycle(cx: &mut TestAppContext) {
        let temp = tempfile::tempdir().unwrap();
        let view = cx.add_window(|w, cx| {
            w.activate();
            let mut root = TesselGpuiRoot::new_in_dir(Some(temp.path().join("state")), false, cx);
            root.app.linear.view = Snapshot {
                open: true,
                epoch: 8,
                secure: Some(true),
                identity: Some(LinearIdentity {
                    viewer: LinearPerson {
                        name: "Synthetic account".into(),
                        ..Default::default()
                    },
                    organization: Default::default(),
                }),
                selected: Some(LinearIssue {
                    id: "fixture".into(),
                    title: "Fixture issue".into(),
                    description: "<script>plain text</script>".into(),
                    ..Default::default()
                }),
                ..Default::default()
            };
            root
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |root, w, cx| {
            assert!(root.app.gpui_has_open_modal());
            assert!(root.linear_view.input.is_none());
            root.linear_view
                .focus
                .get("linear-back")
                .unwrap()
                .focus(w, cx);
        })
        .unwrap();
        {
            let mut visual = VisualTestContext::from_window(view.into(), cx);
            visual.simulate_event(KeyDownEvent {
                keystroke: Keystroke::parse("enter").unwrap(),
                is_held: false,
                prefer_character_input: false,
            });
        }
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |root, _, cx| {
            assert!(root.app.linear.view.selected.is_none());
            root.app.linear.view.identity = None;
            cx.notify();
        })
        .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |root, w, cx| {
            let input = root.linear_view.input.as_ref().unwrap();
            input.entity().update(cx, |state, cx| {
                state.replace_text_in_range(None, "synthetic-key", w, cx)
            });
            assert_eq!(input.entity().read(cx).as_str(), "*************");
            input.focus(w, cx);
        })
        .unwrap();
        {
            let mut visual = VisualTestContext::from_window(view.into(), cx);
            visual.simulate_event(KeyDownEvent {
                keystroke: Keystroke::parse("enter").unwrap(),
                is_held: false,
                prefer_character_input: false,
            });
        }
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |root, w, cx| {
            // This deliberately unbound fixture has no worker path and cannot contact Linear.
            assert_eq!(root.app.linear.view.error, "No private data directory");
            assert!(root.linear_view.input.as_ref().unwrap().is_empty(cx));
            activate(root, 7, Click::Command(Command::Close), w, cx);
            assert!(root.app.linear.view.open);
            activate(root, 8, Click::Command(Command::Close), w, cx);
            assert!(!root.app.linear.view.open);
            assert!(root.linear_view.input.is_none());
        })
        .unwrap();
        assert!(!temp.path().join("state/linear.dat").exists());
    }
}
