//! Local owner form. Its transient scope is never serialized or shared.
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    Title,
    Brief,
    Branch,
}

impl Field {
    pub fn index(self) -> usize {
        match self {
            Self::Title => 0,
            Self::Brief => 1,
            Self::Branch => 2,
        }
    }
}
#[derive(Clone)]
pub(crate) enum Command {
    Edit(Field, String),
    Provider(String),
    Isolated(bool),
    CopyEnv(bool),
    Reviewer(Option<u64>),
    Cancel,
    Start,
}
#[derive(Clone)]
pub(crate) struct Choice {
    pub id: String,
    pub label: String,
    pub enabled: bool,
}
#[derive(Clone, Default)]
pub(crate) struct Panel {
    pub open: bool,
    pub epoch: u64,
    pub project: u64,
    pub source: PathBuf,
    pub name: String,
    pub title: String,
    pub brief: String,
    pub branch: String,
    pub provider: String,
    pub isolated: bool,
    pub copy_env: bool,
    pub reviewer: Option<u64>,
    pub choices: Vec<Choice>,
    pub reviewers: Vec<crate::task_agents_model::Choice>,
    pub isolation: Option<bool>,
    pub error: String,
    invalid: [bool; 3],
}
impl Panel {
    pub fn begin(
        &mut self,
        project: u64,
        source: PathBuf,
        name: String,
        choices: Vec<Choice>,
        reviewers: Vec<crate::task_agents_model::Choice>,
    ) {
        let epoch = self
            .epoch
            .checked_add(1)
            .expect("task dialog identity exhausted");
        let provider = choices
            .iter()
            .find(|c| c.enabled)
            .map(|c| c.id.clone())
            .unwrap_or_default();
        *self = Self {
            open: true,
            epoch,
            project,
            source,
            name,
            provider,
            choices,
            reviewers,
            isolated: true,
            copy_env: true,
            ..Default::default()
        };
    }
    pub fn accepts(&self, epoch: u64) -> bool {
        self.open && self.epoch == epoch
    }
    pub fn edit(&mut self, field: Field, value: String) {
        let invalid = value.len() > 64 * 1024
            || value.contains('\0')
            || match field {
                Field::Title => {
                    value.encode_utf16().count() > 120 || value.chars().any(char::is_control)
                }
                Field::Branch => value.len() > 1024 || value.chars().any(char::is_control),
                Field::Brief => false,
            };
        self.invalid[field.index()] = invalid;
        if value.len() <= 64 * 1024 {
            *match field {
                Field::Title => &mut self.title,
                Field::Brief => &mut self.brief,
                Field::Branch => &mut self.branch,
            } = value;
        }
        self.error = if self.invalid.iter().any(|v| *v) {
            "The title, instructions or branch exceeds its limit or contains invalid characters."
                .into()
        } else {
            String::new()
        };
    }
    pub fn ready(&self) -> bool {
        self.open
            && !self.title.trim().is_empty()
            && !self.invalid[0]
            && !self.invalid[1]
            && (!self.use_copy() || !self.invalid[2])
            && self
                .choices
                .iter()
                .any(|c| c.id == self.provider && c.enabled)
            && (!self.isolated || self.isolation.is_some())
    }
    pub fn use_copy(&self) -> bool {
        self.isolated && self.isolation == Some(true)
    }
}

pub(crate) fn show(
    ctx: &eframe::egui::Context,
    panel: &mut Panel,
    french: bool,
) -> Option<Command> {
    use eframe::egui::{self, TextEdit};
    if !panel.open {
        return None;
    }
    let t = |fr, en| if french { fr } else { en };
    let mut command = None;
    let response = egui::Modal::new(egui::Id::new(("new-task-dialog", panel.epoch))).show(ctx, |ui| {
        ui.set_width((ctx.content_rect().width() - 48.).clamp(240., 620.));
        ui.label(egui::RichText::new(t("Nouvelle tâche", "New task")).size(16.).strong());
        ui.label(format!("{} · {}", panel.name, panel.source.display()));
        egui::ScrollArea::vertical().max_height((ctx.content_rect().height() - 180.).max(140.)).show(ui, |ui| {
            for (field, label, multiline) in [(Field::Title, t("Titre", "Title"), false), (Field::Brief, t("Consignes pour l’agent", "Instructions for the agent"), true)] {
                ui.label(label);
                let mut value = match field { Field::Title => panel.title.clone(), _ => panel.brief.clone() };
                let edit = if multiline { TextEdit::multiline(&mut value).desired_rows(4) } else { TextEdit::singleline(&mut value) };
                if ui.add(edit.desired_width(f32::INFINITY)).changed() { panel.edit(field, value); }
            }
            ui.label(t("Agent", "Agent"));
            for choice in &panel.choices {
                ui.add_enabled_ui(choice.enabled, |ui| { ui.radio_value(&mut panel.provider, choice.id.clone(), format!("{} {}", t("Nouveau", "New"), choice.label)); });
            }
            ui.small(t("Les nouvelles conversations Codex et Claude sont disponibles. Le lancement dans un agent existant ou un terminal n’est pas encore disponible.", "New Codex and Claude chats are supported. Starting tasks in existing agents or terminals is not available yet."));
            ui.separator();
            ui.label(t("Où il travaille", "Where it works"));
            ui.add_enabled_ui(panel.isolation == Some(true), |ui| { ui.radio_value(&mut panel.isolated, true, t("Dans sa propre copie — recommandé", "In its own copy — recommended")); });
            if panel.isolation.is_none() { ui.small(t("Vérification du dépôt Git…", "Checking the Git repository…")); }
            else if panel.isolation == Some(false) { ui.small(t("Aucune copie Git disponible pour ce projet.", "A Git copy is not available for this project.")); }
            if panel.use_copy() {
                ui.label(t("Branche de départ", "Start from branch"));
                let mut branch = panel.branch.clone();
                if ui.add(TextEdit::singleline(&mut branch).hint_text(t("la branche courante", "the current branch"))).changed() { panel.edit(Field::Branch, branch); }
                ui.checkbox(&mut panel.copy_env, t("Copier les fichiers .env du projet", "Copy the project's .env files"));
                let mut unavailable = false;
                ui.add_enabled(false, egui::Checkbox::new(&mut unavailable, t("Exécuter .tessel/setup.ps1 — indisponible pour le moment", "Run .tessel/setup.ps1 — not available yet")));
            }
            ui.radio_value(&mut panel.isolated, false, t("Directement dans le dossier du projet", "Directly in the project folder"));
            if !panel.reviewers.is_empty() {
                ui.add_enabled(false, egui::Button::new(t("Demande automatique de revue — indisponible pour le moment", "Automatic review requests — not available yet")));
            }
            ui.small(t("À la fin, déplacez la carte en Révision manuellement.", "When finished, move the card to Review manually."));
            if !panel.error.is_empty() { ui.colored_label(ui.visuals().error_fg_color, &panel.error); }
        });
        ui.horizontal(|ui| {
            if ui.button(t("Annuler", "Cancel")).clicked() { command = Some(Command::Cancel); }
            if ui.add_enabled(panel.ready(), egui::Button::new(t("Démarrer la tâche", "Start task"))).clicked() { command = Some(Command::Start); }
        });
    });
    if response.should_close() {
        command = Some(Command::Cancel);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    fn panel() -> Panel {
        let mut p = Panel::default();
        p.begin(
            7,
            std::env::temp_dir(),
            "Fixture".into(),
            vec![Choice {
                id: "codex".into(),
                label: "Codex".into(),
                enabled: true,
            }],
            vec![],
        );
        p
    }
    #[test]
    fn new_task_form_defaults_limits_and_explicit_direct_choice() {
        let mut p = panel();
        assert!(p.isolated && p.copy_env);
        p.edit(Field::Title, "Fixture".into());
        assert!(!p.ready()); // Git inspection is not a silent direct-folder launch.
        p.isolated = false;
        assert!(p.ready());
        p.edit(Field::Title, "😀".repeat(61));
        assert!(!p.ready());
        p.edit(Field::Title, "😀".repeat(60));
        assert!(p.ready());
        p.edit(Field::Brief, "x".repeat(65537));
        p.edit(Field::Title, "Still invalid instructions".into());
        assert!(!p.ready());
        assert!(!p.error.is_empty());
        p.edit(Field::Brief, "Fixture instructions".into());
        p.edit(Field::Branch, "hidden\0branch".into());
        assert!(p.ready()); // An unused branch does not affect direct work.
        p.isolated = true;
        p.isolation = Some(true);
        assert!(!p.ready());
        p.edit(Field::Branch, "develop".into());
        assert!(p.ready());
    }
    #[test]
    fn new_task_form_epoch_and_compatibility_render_keep_local_draft() {
        let mut p = panel();
        let old = p.epoch;
        p.begin(
            8,
            std::env::temp_dir(),
            "Other fixture".into(),
            vec![],
            vec![],
        );
        assert!(!p.accepts(old));
        assert!(!p.ready());
        p.edit(Field::Title, "Titre local".into());
        let ctx = eframe::egui::Context::default();
        let output = ctx.run(Default::default(), |ctx| {
            assert!(show(ctx, &mut p, true).is_none());
        });
        assert!(!output.shapes.is_empty());
        assert_eq!(p.title, "Titre local");
        assert!(p.open);
    }
}
