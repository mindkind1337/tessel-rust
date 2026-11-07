//! Native header menu backed by the shared parity labels and implemented actions.
use crate::pane_menu_model::{
    self, PaneActionsMenuInput, PaneMenuAction, PaneMenuEntry, PaneMenuLang,
};
use eframe::egui;

pub fn button(ui: &mut egui::Ui, icon: crate::icons::Icon, label: &str) -> egui::Response {
    let response = ui.add(
        egui::Button::new("")
            .frame(false)
            .min_size(egui::vec2(24.0, 22.0)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let danger = icon == crate::icons::Icon::PaneClose && response.hovered() && ui.is_enabled();
    if danger {
        ui.painter()
            .rect_filled(response.rect, 5.0, ui.visuals().error_fg_color);
    }
    let color = if danger {
        egui::Color32::WHITE
    } else if response.hovered() && ui.is_enabled() {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    crate::icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(response.rect.center(), egui::Vec2::splat(14.0)),
        icon,
        color,
    );
    response.on_hover_text(label)
}

#[derive(Default)]
pub struct MenuState<'a> {
    pub model_choice: bool,
    pub model_pending: bool,
    pub model_manual: bool,
    pub model_text: Option<String>,
    pub title: &'a str,
    pub session_id: Option<&'a str>,
    pub can_restart: bool,
    pub launch_stale: bool,
    pub maximized: bool,
    pub multiple_panes: bool,
    pub terminal_output: bool,
    pub can_rename: bool,
    pub is_agent: bool,
    pub sleeping: bool,
    pub exited: bool,
    pub asks_approval: bool,
    pub activity: Option<pane_menu_model::PaneAgentActivity>,
    pub needs_you: bool,
    pub worktree: Option<(String, String)>,
    pub team: Option<pane_menu_model::TeamMembership>,
    pub can_switch_chat_mode: bool,
    pub chat_view_open: bool,
    pub has_selection: bool,
    pub voice_typing: bool,
    pub other_panes: Vec<pane_menu_model::OtherPaneTarget>,
    pub language: PaneMenuLang,
}

fn build_filtered(
    state: &MenuState<'_>,
) -> (Vec<pane_menu_model::PaneMenuFact>, Vec<PaneMenuEntry>) {
    let built = pane_menu_model::build_pane_actions_menu(
        &PaneActionsMenuInput {
            pane_title: state.title.into(),
            session_id: state.session_id.map(str::to_owned),
            launch_stale: state.can_restart && state.launch_stale,
            is_maximized: state.maximized,
            can_arrange_panes: state.multiple_panes,
            has_model_choice: state.model_choice,
            model_text: state.model_text.clone(),
            is_agent: state.is_agent,
            sleeping: state.sleeping,
            exited: state.exited,
            asks_approval: state.asks_approval,
            activity: state.activity,
            needs_you: state.needs_you,
            worktree: state.worktree.clone(),
            team: state.team.clone(),
            can_open_as_chat: state.can_switch_chat_mode,
            chat_view_available: state.can_switch_chat_mode,
            chat_view_open: state.chat_view_open,
            has_selection: state.has_selection,
            other_panes: if state.terminal_output {
                state.other_panes.clone()
            } else {
                Vec::new()
            },
            disabled_reasons: if state.model_pending {
                [(
                    "menuModel".into(),
                    match state.language {
                        PaneMenuLang::Fr => {
                            "Le volet démarre ou redémarre. Attendez la fin de l'opération."
                        }
                        PaneMenuLang::En => {
                            "The pane is starting or restarting. Wait for it to finish."
                        }
                    }
                    .into(),
                )]
                .into()
            } else {
                Default::default()
            },
            ..Default::default()
        },
        state.language,
    );

    let raw_entries: Vec<PaneMenuEntry> = built
        .entries
        .into_iter()
        .filter(|entry| match entry {
            PaneMenuEntry::Separator => true,
            PaneMenuEntry::SectionLabel(label) => {
                // Keep "Send selection to" header when terminal_output + other_panes exist;
                // skip "Ask to review..." since AskReview is not wired yet.
                state.terminal_output
                    && !state.other_panes.is_empty()
                    && (label == "Send selection to" || label == "Envoyer la sélection à")
            }
            PaneMenuEntry::Item { action, .. } => match action {
                PaneMenuAction::Copy
                | PaneMenuAction::Paste
                | PaneMenuAction::CopyOutput
                | PaneMenuAction::Clear
                | PaneMenuAction::ResetTerminal
                | PaneMenuAction::Find => state.terminal_output,
                PaneMenuAction::VoiceTyping => state.voice_typing && state.terminal_output,
                PaneMenuAction::SendSelectionTo(_) => {
                    state.terminal_output && !state.other_panes.is_empty()
                }
                PaneMenuAction::Restart | PaneMenuAction::RestartApplySettings => state.can_restart,
                PaneMenuAction::CopySessionId => state.session_id.is_some_and(|id| !id.is_empty()),
                PaneMenuAction::Rename => state.can_rename,
                PaneMenuAction::ChooseModel => state.model_choice,
                PaneMenuAction::ToggleTeamLead(_) | PaneMenuAction::LeaveTeam(_) => {
                    state.team.is_some()
                }
                PaneMenuAction::OpenOrSwitchChat => state.can_switch_chat_mode,
                PaneMenuAction::OpenHere
                | PaneMenuAction::ToggleMaximize
                | PaneMenuAction::ArrangeGrid
                | PaneMenuAction::SplitRight
                | PaneMenuAction::SplitDown
                | PaneMenuAction::ClosePane => true,
                _ => false,
            },
            _ => false,
        })
        .collect();

    // Collapse leading, trailing, and consecutive separators cleanly:
    let mut cleaned: Vec<PaneMenuEntry> = Vec::with_capacity(raw_entries.len());
    for entry in raw_entries {
        if matches!(entry, PaneMenuEntry::Separator)
            && (cleaned.is_empty() || matches!(cleaned.last(), Some(PaneMenuEntry::Separator)))
        {
            continue;
        }
        cleaned.push(entry);
    }
    while matches!(cleaned.last(), Some(PaneMenuEntry::Separator)) {
        cleaned.pop();
    }

    (built.facts, cleaned)
}

#[cfg(test)]
fn entries(state: &MenuState<'_>) -> Vec<PaneMenuEntry> {
    build_filtered(state).1
}

pub fn show(ui: &mut egui::Ui, state: &MenuState<'_>) -> Option<PaneMenuAction> {
    let (facts, entries) = build_filtered(state);
    ui.strong(state.title);
    if !facts.is_empty() {
        ui.add_space(2.0);
        for fact in &facts {
            let row = ui.horizontal(|ui| {
                ui.weak(&fact.label);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&fact.value).size(11.5).strong());
                });
            });
            if !fact.tooltip.is_empty() {
                row.response.on_hover_text(&fact.tooltip);
            }
        }
    }
    ui.separator();
    let mut selected = None;
    let mut last_was_sep = true;
    for entry in entries {
        match entry {
            PaneMenuEntry::Separator => {
                if !last_was_sep {
                    ui.separator();
                    last_was_sep = true;
                }
            }
            PaneMenuEntry::SectionLabel(label) => {
                ui.add_space(2.0);
                ui.weak(egui::RichText::new(label).size(11.0));
                last_was_sep = false;
            }
            PaneMenuEntry::Item {
                action,
                label,
                enabled,
                danger,
                shortcut,
                tooltip,
            } => {
                if danger && !last_was_sep {
                    ui.separator();
                }
                last_was_sep = false;
                let text = if danger {
                    egui::RichText::new(label).color(ui.visuals().error_fg_color)
                } else {
                    egui::RichText::new(label)
                };
                let mut button = egui::Button::new(text);
                if let Some(shortcut) = shortcut {
                    if action == PaneMenuAction::ChooseModel {
                        let mut job = egui::text::LayoutJob::single_section(
                            shortcut,
                            egui::TextFormat {
                                font_id: egui::FontId::proportional(11.0),
                                color: ui.visuals().weak_text_color(),
                                italics: state.model_manual,
                                ..Default::default()
                            },
                        );
                        job.wrap = egui::text::TextWrapping {
                            max_width: 150.0,
                            max_rows: 1,
                            break_anywhere: true,
                            overflow_character: Some('…'),
                        };
                        button = button.shortcut_text(ui.fonts_mut(|fonts| fonts.layout_job(job)));
                    } else {
                        button = button.shortcut_text(shortcut);
                    }
                }
                let response = ui.add_enabled(enabled, button);
                let response = if let Some(tooltip) = tooltip {
                    if enabled {
                        response.on_hover_text(tooltip)
                    } else {
                        response.on_disabled_hover_text(tooltip)
                    }
                } else {
                    response
                };
                if response.clicked() {
                    selected = Some(action);
                    ui.close();
                }
            }
            _ => {}
        }
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_menu_only_exposes_actions_supported_by_the_pane() {
        let mut state = MenuState {
            title: "Chat",
            language: PaneMenuLang::Fr,
            ..Default::default()
        };
        let actions = |state: &MenuState<'_>| {
            entries(state)
                .into_iter()
                .filter_map(|e| match e {
                    PaneMenuEntry::Item { action, .. } => Some(action),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            actions(&state),
            vec![
                PaneMenuAction::ToggleMaximize,
                PaneMenuAction::OpenHere,
                PaneMenuAction::SplitRight,
                PaneMenuAction::SplitDown,
                PaneMenuAction::ClosePane
            ]
        );
        state.can_rename = true;
        state.session_id = Some("12345678-1234-4234-8234-123456789abc");
        assert!(actions(&state).contains(&PaneMenuAction::CopySessionId));
        assert!(entries(&state).iter().any(|entry| matches!(entry, PaneMenuEntry::Item { action: PaneMenuAction::CopySessionId, label, shortcut: Some(shortcut), .. } if label == "Copier l'ID de session" && shortcut == "12345678")));
        assert!(actions(&state).contains(&PaneMenuAction::Rename));
        state.multiple_panes = true;
        state.terminal_output = true;
        state.voice_typing = true;
        state.has_selection = true;
        state.is_agent = true;
        state.other_panes = vec![pane_menu_model::OtherPaneTarget {
            id: "2".into(),
            title: "Bohr".into(),
            where_hint: String::new(),
            is_agent: true,
        }];
        let (facts, _) = build_filtered(&state);
        assert!(
            facts
                .iter()
                .any(|f| f.kind == pane_menu_model::PaneFactKind::State)
        );
        let menu = actions(&state);
        assert!(menu.contains(&PaneMenuAction::Copy));
        assert!(menu.contains(&PaneMenuAction::Paste));
        assert!(menu.contains(&PaneMenuAction::CopyOutput));
        assert!(menu.contains(&PaneMenuAction::Clear));
        assert!(menu.contains(&PaneMenuAction::ResetTerminal));
        assert!(menu.contains(&PaneMenuAction::Find));
        assert!(menu.contains(&PaneMenuAction::VoiceTyping));
        assert!(menu.contains(&PaneMenuAction::SendSelectionTo("2".into())));
        assert!(menu.contains(&PaneMenuAction::ArrangeGrid));
        assert!(!menu.contains(&PaneMenuAction::SwitchYolo));
        assert!(!menu.contains(&PaneMenuAction::Restart));
        state.can_restart = true;
        state.launch_stale = true;
        assert!(actions(&state).contains(&PaneMenuAction::Restart));
        assert!(actions(&state).contains(&PaneMenuAction::RestartApplySettings));
        state.model_choice = true;
        state.model_text = Some("synthetic-model".into());
        assert!(entries(&state).iter().any(|entry| matches!(entry, PaneMenuEntry::Item { action: PaneMenuAction::ChooseModel, label, shortcut: Some(shortcut), .. } if label == "Modèle…" && shortcut == "synthetic-model")));
        state.model_pending = true;
        assert!(entries(&state).iter().any(|entry| matches!(
            entry,
            PaneMenuEntry::Item {
                action: PaneMenuAction::ChooseModel,
                enabled: false,
                ..
            }
        )));
    }
}
