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

pub struct MenuState<'a> {
    pub title: &'a str,
    pub session_id: Option<&'a str>,
    pub can_restart: bool,
    pub launch_stale: bool,
    pub maximized: bool,
    pub multiple_panes: bool,
    pub terminal_output: bool,
    pub can_rename: bool,
    pub language: PaneMenuLang,
}

fn entries(state: &MenuState<'_>) -> Vec<PaneMenuEntry> {
    pane_menu_model::build_pane_actions_menu(
        &PaneActionsMenuInput {
            pane_title: state.title.into(),
            session_id: state.session_id.map(str::to_owned),
            launch_stale: state.can_restart && state.launch_stale,
            is_maximized: state.maximized,
            can_arrange_panes: state.multiple_panes,
            ..Default::default()
        },
        state.language,
    )
    .entries
    .into_iter()
    .filter(|entry| match entry {
        PaneMenuEntry::Item { action, .. } => match action {
            PaneMenuAction::CopyOutput => state.terminal_output,
            PaneMenuAction::Restart | PaneMenuAction::RestartApplySettings => state.can_restart,
            PaneMenuAction::CopySessionId => state.session_id.is_some_and(|id| !id.is_empty()),
            PaneMenuAction::Rename => state.can_rename,
            PaneMenuAction::ToggleMaximize
            | PaneMenuAction::ArrangeGrid
            | PaneMenuAction::SplitRight
            | PaneMenuAction::SplitDown
            | PaneMenuAction::ClosePane => true,
            _ => false,
        },
        _ => false,
    })
    .collect()
}

pub fn show(ui: &mut egui::Ui, state: &MenuState<'_>) -> Option<PaneMenuAction> {
    ui.strong(state.title);
    ui.separator();
    let mut selected = None;
    for entry in entries(state) {
        if let PaneMenuEntry::Item {
            action,
            label,
            enabled,
            danger,
            shortcut,
            ..
        } = entry
        {
            if danger {
                ui.separator();
            }
            let text = if danger {
                egui::RichText::new(label).color(ui.visuals().error_fg_color)
            } else {
                egui::RichText::new(label)
            };
            let mut button = egui::Button::new(text);
            if let Some(shortcut) = shortcut {
                button = button.shortcut_text(shortcut);
            }
            if ui.add_enabled(enabled, button).clicked() {
                selected = Some(action);
                ui.close();
            }
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
            session_id: None,
            can_restart: false,
            launch_stale: false,
            maximized: false,
            multiple_panes: false,
            terminal_output: false,
            can_rename: false,
            language: PaneMenuLang::Fr,
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
        let menu = actions(&state);
        assert!(menu.contains(&PaneMenuAction::CopyOutput));
        assert!(menu.contains(&PaneMenuAction::ArrangeGrid));
        assert!(!menu.contains(&PaneMenuAction::Paste));
        assert!(!menu.contains(&PaneMenuAction::SwitchYolo));
        assert!(!menu.contains(&PaneMenuAction::Restart));
        state.can_restart = true;
        state.launch_stale = true;
        assert!(actions(&state).contains(&PaneMenuAction::Restart));
        assert!(actions(&state).contains(&PaneMenuAction::RestartApplySettings));
    }
}
