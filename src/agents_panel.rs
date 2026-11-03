//! Native Agents settings. Pure display/actions; launch and persistence stay in app.
use eframe::egui::{self, RichText, Ui};

pub struct Row {
    pub preset: &'static crate::agent_catalog::Preset,
    pub available: bool,
    pub enabled: bool,
}

pub struct Input<'a> {
    pub rows: &'a [Row],
    pub loaded: bool,
    pub detecting: bool,
    pub error: Option<&'a str>,
    pub notice: Option<&'a str>,
    pub has_project: bool,
    pub french: bool,
    pub search: &'a str,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Detect,
    SetEnabled(String, bool),
    Terminal(String),
    NativeChat(String),
    Docs(String),
    CopyInstall(String),
}

#[derive(Default)]
pub struct AgentsPanel {
    open: Option<String>,
    missing_open: bool,
    last_installed: Option<usize>,
    escape_held: bool,
}

impl AgentsPanel {
    /// Called before the settings navigation, so Escape folds details first.
    pub fn handle_escape(&mut self, ctx: &egui::Context) -> bool {
        let down = ctx.input(|i| i.key_down(egui::Key::Escape));
        if !down {
            self.escape_held = false;
        }
        if !ctx.input(|i| i.focused) || egui::Popup::is_any_open(ctx) {
            return false;
        }
        if self.open.is_none() && !self.escape_held {
            return false;
        }
        let consumed = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if consumed {
            self.open = None;
            self.escape_held = down;
        }
        consumed
    }

    pub fn show(&mut self, ui: &mut Ui, input: Input<'_>) -> Option<Action> {
        let tr = |en: &'static str, fr: &'static str| if input.french { fr } else { en };
        let mut action = None;
        ui.heading("Agents");
        ui.label(tr(
            "Which agents Tessel offers, how they start and what they may do.",
            "Les agents proposés par Tessel, leur démarrage et leurs autorisations.",
        ));
        ui.add_space(14.0);
        let installed = input.rows.iter().filter(|row| row.available).count();
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!("{} ({installed})", tr("Installed", "Installés"))).strong(),
            );
            if ui
                .add_enabled(
                    !input.detecting,
                    egui::Button::new(if input.detecting {
                        tr("Detecting…", "Détection…")
                    } else {
                        tr("Detect again", "Détecter à nouveau")
                    }),
                )
                .clicked()
            {
                action = Some(Action::Detect);
            }
            if input.detecting {
                ui.spinner();
            }
        });
        if let Some(error) = input.error {
            ui.colored_label(ui.visuals().error_fg_color, crate::i18n::tr(error));
        }
        if let Some(notice) = input.notice {
            ui.label(notice);
        }
        if !input.loaded {
            return action;
        }
        if self.last_installed != Some(installed) {
            if installed == 0 && input.error.is_none() {
                self.missing_open = true;
            }
            self.last_installed = Some(installed);
        }
        let query = input.search.trim().to_lowercase();
        let matches = |row: &&Row| {
            query.is_empty()
                || query == "agents"
                || format!(
                    "{} {} {}",
                    row.preset.id, row.preset.name, row.preset.command
                )
                .to_lowercase()
                .contains(&query)
        };
        if installed == 0 && input.error.is_none() {
            ui.label(tr(
                "No agent found on this computer yet.",
                "Aucun agent trouvé sur cet ordinateur pour l’instant.",
            ));
        }
        ui.add_space(8.0);
        for row in input
            .rows
            .iter()
            .filter(|row| row.available)
            .filter(matches)
        {
            let preset = row.preset;
            ui.push_id(preset.id, |ui| {
                egui::Frame::NONE
                    .inner_margin(8)
                    .corner_radius(6)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let mut enabled = row.enabled;
                            if ui
                                .checkbox(&mut enabled, "")
                                .on_hover_text(tr("Offer in menus", "Afficher dans les menus"))
                                .changed()
                            {
                                action = Some(Action::SetEnabled(preset.id.into(), enabled));
                            }
                            let expanded = self.open.as_deref() == Some(preset.id);
                            let label = RichText::new(preset.name).strong().color(if row.enabled {
                                ui.visuals().text_color()
                            } else {
                                ui.visuals().weak_text_color()
                            });
                            if ui
                                .add_sized(
                                    [ui.available_width(), 32.0],
                                    egui::Button::new(label).selected(expanded).frame(expanded),
                                )
                                .clicked()
                            {
                                self.open = if expanded {
                                    None
                                } else {
                                    Some(preset.id.into())
                                };
                            }
                        });
                        if self.open.as_deref() == Some(preset.id) {
                            egui::Frame::NONE
                                .fill(ui.visuals().faint_bg_color)
                                .inner_margin(10)
                                .corner_radius(6)
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(tr("How it starts", "Comment il démarre"))
                                            .small()
                                            .strong(),
                                    );
                                    ui.add(
                                        egui::Label::new(RichText::new(preset.command).monospace())
                                            .wrap(),
                                    );
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .button(tr(
                                                "Open in terminal",
                                                "Ouvrir dans un terminal",
                                            ))
                                            .clicked()
                                        {
                                            action = Some(Action::Terminal(preset.id.into()));
                                        }
                                        if matches!(preset.id, "codex" | "claude")
                                            && ui
                                                .add_enabled(
                                                    input.has_project,
                                                    egui::Button::new(tr(
                                                        "Native chat",
                                                        "Chat natif",
                                                    )),
                                                )
                                                .on_disabled_hover_text(tr(
                                                    "Select a project for native chat.",
                                                    "Sélectionnez un projet pour le chat natif.",
                                                ))
                                                .clicked()
                                        {
                                            action = Some(Action::NativeChat(preset.id.into()));
                                        }
                                        if preset.docs.is_some()
                                            && ui.button(tr("Docs", "Documentation")).clicked()
                                        {
                                            action = Some(Action::Docs(preset.id.into()));
                                        }
                                    });
                                });
                        }
                    });
                ui.separator();
            });
        }
        let missing = input.rows.iter().filter(|row| !row.available).count();
        if missing > 0 {
            let label = format!("{} ({missing})", tr("Not installed", "Non installés"));
            if ui
                .selectable_label(self.missing_open || !query.is_empty(), label)
                .clicked()
            {
                self.missing_open = !self.missing_open;
            }
            if self.missing_open || !query.is_empty() {
                for row in input
                    .rows
                    .iter()
                    .filter(|row| !row.available)
                    .filter(matches)
                {
                    let preset = row.preset;
                    ui.push_id(preset.id, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(preset.name).strong());
                            if crate::agent_installs::find(preset.id)
                                .is_some_and(|entry| entry.install.is_some())
                                && ui
                                    .button(tr(
                                        "Copy install command",
                                        "Copier la commande d’installation",
                                    ))
                                    .clicked()
                            {
                                action = Some(Action::CopyInstall(preset.id.into()));
                            }
                            if (preset.docs.is_some()
                                || crate::agent_installs::find(preset.id)
                                    .is_some_and(|entry| entry.docs_url.is_some()))
                                && ui
                                    .button(tr("Install page", "Page d’installation"))
                                    .clicked()
                            {
                                action = Some(Action::Docs(preset.id.into()));
                            }
                        });
                        ui.add(
                            egui::Label::new(RichText::new(preset.executable).monospace().weak())
                                .wrap(),
                        );
                        ui.separator();
                    });
                }
            }
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(
        ctx: &egui::Context,
        panel: &mut AgentsPanel,
        events: Vec<egui::Event>,
        project: bool,
        enabled: bool,
        width: f32,
    ) -> (egui::FullOutput, Option<Action>, f32) {
        let rows = vec![
            Row {
                preset: crate::agent_catalog::find("codex").unwrap(),
                available: true,
                enabled: true,
            },
            Row {
                preset: crate::agent_catalog::find("kiro").unwrap(),
                available: false,
                enabled: true,
            },
        ];
        let mut rows = rows;
        rows.push(Row {
            preset: crate::agent_catalog::find("aider").unwrap(),
            available: false,
            enabled: true,
        });
        let mut action = None;
        let mut right = 0.0;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 900.0),
                )),
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| {
                if enabled {
                    panel.handle_escape(ctx);
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        action = panel.show(
                            ui,
                            Input {
                                rows: &rows,
                                loaded: true,
                                detecting: false,
                                error: None,
                                notice: None,
                                has_project: project,
                                french: false,
                                search: "",
                            },
                        );
                        right = ui.min_rect().right();
                    });
                });
            },
        );
        (output, action, right)
    }
    fn text_center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        fn find(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, label))
            .unwrap_or_else(|| panic!("Missing label {label}"))
    }
    fn click(
        ctx: &egui::Context,
        panel: &mut AgentsPanel,
        label: &str,
        project: bool,
        enabled: bool,
    ) -> Option<Action> {
        let (out, _, _) = frame(ctx, panel, vec![], project, enabled, 620.0);
        let pos = text_center(&out, label);
        let pointer = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            ctx,
            panel,
            vec![egui::Event::PointerMoved(pos), pointer(true)],
            project,
            enabled,
            620.0,
        );
        frame(ctx, panel, vec![pointer(false)], project, enabled, 620.0).1
    }
    #[test]
    fn pointer_actions_and_native_chat_project_gate_are_real() {
        let ctx = egui::Context::default();
        let mut panel = AgentsPanel::default();
        frame(&ctx, &mut panel, vec![], false, true, 620.0);
        assert_eq!(
            click(&ctx, &mut panel, "Detect again", false, true),
            Some(Action::Detect)
        );
        assert_eq!(click(&ctx, &mut panel, "Codex CLI", false, true), None);
        assert_eq!(panel.open.as_deref(), Some("codex"));
        assert_eq!(click(&ctx, &mut panel, "Native chat", false, true), None);
        assert_eq!(
            click(&ctx, &mut panel, "Native chat", true, true),
            Some(Action::NativeChat("codex".into()))
        );
        assert_eq!(
            click(&ctx, &mut panel, "Open in terminal", true, true),
            Some(Action::Terminal("codex".into()))
        );
        assert_eq!(
            click(&ctx, &mut panel, "Docs", true, true),
            Some(Action::Docs("codex".into()))
        );
        assert_eq!(click(&ctx, &mut panel, "Detect again", true, false), None);
    }
    #[test]
    fn missing_agents_offer_docs_or_copy_without_launching() {
        let ctx = egui::Context::default();
        let mut panel = AgentsPanel::default();
        frame(&ctx, &mut panel, vec![], false, true, 620.0);
        assert!(click(&ctx, &mut panel, "Not installed (2)", false, true).is_none());
        assert_eq!(
            click(&ctx, &mut panel, "Install page", false, true),
            Some(Action::Docs("kiro".into()))
        );
        assert_eq!(
            click(&ctx, &mut panel, "Copy install command", false, true),
            Some(Action::CopyInstall("aider".into()))
        );
        let (_, action, right) = frame(&ctx, &mut panel, vec![], false, true, 300.0);
        assert!(action.is_none());
        assert!(right <= 300.0, "narrow panel overflow: {right}");
    }

    #[test]
    fn escape_folds_details_and_repeat_does_not_escape_to_parent() {
        let ctx = egui::Context::default();
        let mut panel = AgentsPanel {
            open: Some("codex".into()),
            ..Default::default()
        };
        frame(&ctx, &mut panel, vec![], true, true, 300.0);
        let key = |repeat| egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: egui::Modifiers::NONE,
        };
        let (_, action, right) = frame(&ctx, &mut panel, vec![key(false)], true, true, 300.0);
        assert!(panel.open.is_none());
        assert!(action.is_none());
        assert!(right <= 300.0);
        frame(&ctx, &mut panel, vec![key(true)], true, true, 300.0);
        assert!(!ctx.input(|i| i.key_pressed(egui::Key::Escape)));
    }
}
