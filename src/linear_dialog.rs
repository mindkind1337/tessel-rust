//! Compatibility renderer for the owner-local Linear dialog.
use crate::linear_model::{
    LinearFilter,
    panel::{Command, Panel, issue_text},
};
use eframe::egui::{self, RichText};
use zeroize::Zeroizing;

// Egui reads only stars (including its undo, accessibility and output caches).
// Generic undo replacement is intentionally ignored; it contains masks, not keys.
struct KeyBuffer<'a> {
    key: &'a mut Zeroizing<String>,
    mask: String,
    rejected: &'a mut bool,
}
impl egui::TextBuffer for KeyBuffer<'_> {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<KeyBuffer<'static>>()
    }
    fn is_mutable(&self) -> bool {
        true
    }
    fn as_str(&self) -> &str {
        &self.mask
    }
    fn insert_text(&mut self, text: &str, index: usize) -> usize {
        if index > self.key.len()
            || self.key.len().saturating_add(text.len()) > 4096
            || !text.bytes().all(|b| (0x20..=0x7e).contains(&b))
        {
            *self.rejected = true;
            return 0;
        }
        *self.rejected = false;
        let mut next = Zeroizing::new(String::with_capacity(self.key.len() + text.len()));
        next.push_str(&self.key[..index]);
        next.push_str(text);
        next.push_str(&self.key[index..]);
        *self.key = next;
        self.mask = "*".repeat(self.key.len());
        text.len()
    }
    fn delete_char_range(&mut self, range: std::ops::Range<usize>) {
        if range.start > range.end || range.end > self.key.len() {
            return;
        }
        let mut next = Zeroizing::new(String::with_capacity(self.key.len() - range.len()));
        next.push_str(&self.key[..range.start]);
        next.push_str(&self.key[range.end..]);
        *self.key = next;
        self.mask = "*".repeat(self.key.len());
    }
    fn replace_with(&mut self, _: &str) {}
}

pub fn show(ctx: &egui::Context, panel: &mut Panel, french: bool) {
    if !panel.view.open {
        return;
    }
    let view = panel.view.clone();
    let epoch = view.epoch;
    let t = |fr, en| if french { fr } else { en };
    let mut command = None;
    let mut connect = false;
    let response = egui::Modal::new(egui::Id::new("linear-dialog")).show(ctx, |ui| {
        ui.set_width(
            720.0_f32
                .min(ui.ctx().content_rect().width() - 48.0)
                .max(240.0),
        );
        ui.horizontal(|ui| {
            ui.heading("Linear");
            if ui
                .add_enabled(!view.busy, egui::Button::new(t("Fermer", "Close")))
                .clicked()
            {
                command = Some(Command::Close);
            }
        });
        if !view.error.is_empty() {
            ui.colored_label(ui.visuals().error_fg_color, &view.error);
            if ui
                .add_enabled(!view.busy, egui::Button::new(t("Réessayer", "Retry")))
                .clicked()
            {
                command = Some(Command::Retry);
            }
        }
        if view.loading_account {
            ui.label(t("Lecture de la connexion…", "Reading connection…"));
        } else if let Some(identity) = &view.identity {
            ui.add_enabled_ui(!view.busy, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        identity
                            .viewer
                            .display_name
                            .as_ref()
                            .unwrap_or(&identity.viewer.name),
                    );
                    if ui
                        .add_enabled(
                            !view.loading_issues,
                            egui::Button::new(t("Actualiser", "Refresh")),
                        )
                        .clicked()
                    {
                        command = Some(Command::Refresh);
                    }
                    if ui.button(t("Déconnecter", "Disconnect")).clicked() {
                        command = Some(Command::ConfirmDisconnect(true));
                    }
                });
                if view.confirm_disconnect {
                    ui.label(t(
                        "Supprimer la clé enregistrée ? Vos issues restent dans Linear.",
                        "Remove the saved key? Your issues stay in Linear.",
                    ));
                    ui.horizontal(|ui| {
                        if ui
                            .button(t("Garder la connexion", "Keep connection"))
                            .clicked()
                        {
                            command = Some(Command::ConfirmDisconnect(false));
                        }
                        if ui.button(t("Déconnecter", "Disconnect")).clicked() {
                            command = Some(Command::Disconnect);
                        }
                    });
                }
                if let Some(issue) = &view.selected {
                    ui.horizontal(|ui| {
                        if ui.button(t("← Issues", "← Issues")).clicked() {
                            command = Some(Command::Back);
                        }
                        if ui.button(t("Ouvrir", "Open")).clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(&issue.url));
                        }
                        if ui.button(t("Copier le lien", "Copy link")).clicked() {
                            ui.ctx().copy_text(issue.url.clone());
                        }
                        if ui.button(t("Copier l’issue", "Copy issue")).clicked() {
                            ui.ctx().copy_text(issue_text(issue));
                        }
                    });
                    ui.heading(format!("{} {}", issue.identifier, issue.title));
                    ui.label(format!("{} · {}", issue.state.name, issue.team.name));
                    egui::ScrollArea::vertical()
                        .max_height(360.)
                        .show(ui, |ui| {
                            ui.label(RichText::new(&issue.description).monospace());
                        });
                    ui.label(t(
                        "La préparation d’une tâche depuis Linear reste à raccorder.",
                        "Preparing a task from Linear is not connected yet.",
                    ));
                } else {
                    let mut filter = view.selection.filter;
                    let mut team = view.selection.team_id.clone();
                    let mut state = view.selection.state_id.clone();
                    ui.horizontal_wrapped(|ui| {
                        egui::ComboBox::from_id_salt("linear-filter")
                            .selected_text(filter.label(french))
                            .show_ui(ui, |ui| {
                                for item in LinearFilter::ALL {
                                    ui.selectable_value(&mut filter, item, item.label(french));
                                }
                            });
                        egui::ComboBox::from_id_salt("linear-team")
                            .selected_text(
                                view.teams
                                    .iter()
                                    .find(|v| Some(&v.id) == team.as_ref())
                                    .map(|v| v.name.as_str())
                                    .unwrap_or(t("Toutes les équipes", "All teams")),
                            )
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut team,
                                    None,
                                    t("Toutes les équipes", "All teams"),
                                );
                                for item in &view.teams {
                                    ui.selectable_value(
                                        &mut team,
                                        Some(item.id.clone()),
                                        &item.name,
                                    );
                                }
                            });
                        ui.add_enabled_ui(team.is_some(), |ui| {
                            egui::ComboBox::from_id_salt("linear-state")
                                .selected_text(
                                    view.states
                                        .iter()
                                        .find(|v| Some(&v.id) == state.as_ref())
                                        .map(|v| v.name.as_str())
                                        .unwrap_or(t("Tous les états", "All states")),
                                )
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut state,
                                        None,
                                        t("Tous les états", "All states"),
                                    );
                                    for item in &view.states {
                                        ui.selectable_value(
                                            &mut state,
                                            Some(item.id.clone()),
                                            &item.name,
                                        );
                                    }
                                });
                        });
                    });
                    if filter != view.selection.filter {
                        command = Some(Command::Filter(filter));
                    } else if team != view.selection.team_id {
                        command = Some(Command::Team(team));
                    } else if state != view.selection.state_id {
                        command = Some(Command::State(state));
                    }
                    if view.loading_issues {
                        ui.spinner();
                    } else if view.issues.is_empty() {
                        ui.label(t(
                            "Aucune issue pour ces filtres.",
                            "No issues match these filters.",
                        ));
                    }
                    if view.has_more {
                        ui.label(t(
                            "100 premières issues. Précisez les filtres.",
                            "First 100 issues. Narrow the filters.",
                        ));
                    }
                    egui::ScrollArea::vertical()
                        .max_height(400.)
                        .show(ui, |ui| {
                            for issue in &view.issues {
                                if ui
                                    .add_enabled(
                                        !view.loading_issues,
                                        egui::Button::new(format!(
                                            "{} {}\n{} · {}",
                                            issue.identifier,
                                            issue.title,
                                            issue.state.name,
                                            issue.team.name
                                        )),
                                    )
                                    .clicked()
                                {
                                    command = Some(Command::Select(issue.id.clone()));
                                }
                            }
                        });
                }
            });
        } else {
            ui.label(t(
                "Créez une clé dans les paramètres Linear → Sécurité et accès.",
                "Create a key in Linear settings → Security & access.",
            ));
            if view.secure == Some(false) {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    t(
                        "Le stockage chiffré est indisponible sur cet ordinateur.",
                        "Encrypted storage is unavailable on this computer.",
                    ),
                );
            }
            ui.label(t("Clé API personnelle", "Personal API key"));
            let mut buffer = KeyBuffer {
                mask: "*".repeat(panel.key.len()),
                key: &mut panel.key,
                rejected: &mut panel.rejected_key,
            };
            let mut enter = false;
            ui.add_enabled_ui(!view.busy, |ui| {
                let mut output = egui::TextEdit::singleline(&mut buffer)
                    .password(true)
                    .id_salt(("linear-private-key", epoch))
                    .show(ui);
                enter =
                    output.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                output.state.clear_undoer();
                output.state.store(ui.ctx(), output.response.id);
            });
            if panel.rejected_key {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    t(
                        "Clé refusée : utilisez au plus 4096 caractères ASCII.",
                        "Key rejected: use at most 4096 ASCII characters.",
                    ),
                );
            }
            ui.hyperlink_to(
                t("Ouvrir les paramètres Linear", "Open Linear settings"),
                "https://linear.app/settings/api",
            );
            let enabled = !view.busy
                && !panel.rejected_key
                && view.secure == Some(true)
                && !panel.key.trim().is_empty();
            connect = ui
                .add_enabled(
                    enabled,
                    egui::Button::new(t("Tester et enregistrer", "Test and save")),
                )
                .clicked()
                || (enter && enabled);
        }
    });
    if response.should_close() && !view.busy {
        command = Some(Command::Close);
    }
    if let Some(command) = command {
        panel.command(epoch, command);
    }
    if connect {
        let key = std::mem::replace(&mut panel.key, Zeroizing::new(String::new()));
        panel.connect(epoch, key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::TextBuffer;
    #[test]
    fn compatibility_password_undo_and_render_never_receive_key() {
        let ctx = egui::Context::default();
        let mut key = Zeroizing::new("synthetic-key".to_owned());
        let mut rejected = false;
        let mut buffer = KeyBuffer {
            mask: "*".repeat(key.len()),
            key: &mut key,
            rejected: &mut rejected,
        };
        buffer.delete_char_range(0..4);
        assert_eq!(buffer.insert_text("fake", 0), 4);
        assert_eq!(buffer.insert_text("é", 0), 0);
        buffer.replace_with("********"); // generic undo must not replace key with its mask
        assert_eq!(buffer.key.as_str(), "fakehetic-key");
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let output = egui::TextEdit::singleline(&mut buffer)
                    .password(true)
                    .show(ui);
                assert!(!output.galley.text().contains("fakehetic"));
                assert!(!format!("{:?}", output.state.undoer()).contains("fakehetic"));
            });
        });
    }
}
