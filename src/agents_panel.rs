//! Native Agents settings. Pure display/actions; launch and persistence stay in app.
use eframe::egui::{self, RichText, Ui};

pub struct Row {
    pub preset: &'static crate::agent_catalog::Preset,
    pub available: bool,
    pub enabled: bool,
    pub prefs: crate::agent_preferences::Preferences,
    pub model_defaults: crate::agent_model_defaults::Defaults,
}

pub struct Input<'a> {
    pub discovery: Option<&'a crate::agent_model_discovery::Discovery>,
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
    RefreshModels(String),
    Detect,
    SetEnabled(String, bool),
    SetModelDefaults(String, crate::agent_model_defaults::Defaults),
    Terminal(String),
    NativeChat(String),
    Docs(String),
    CopyInstall(String),
}

#[derive(Default)]
pub struct AgentsPanel {
    model_ui: crate::agent_model_defaults_ui::AgentModelDefaultsUi,
    open: Option<String>,
    missing_open: bool,
    last_installed: Option<usize>,
    escape_held: bool,
    open_row: Option<egui::Id>,
    detail_focus: Vec<egui::Id>,
    drafts: std::collections::BTreeMap<String, crate::agent_preferences::Preferences>,
    pending_save: Option<(String, crate::agent_preferences::Preferences)>,
}

impl AgentsPanel {
    #[cfg(feature = "visual-test")]
    pub fn model_picker_fixture(agent: &str) -> Self {
        Self {
            open: Some(agent.into()),
            ..Self::default()
        }
    }
    pub fn take_save(&mut self) -> Option<(String, crate::agent_preferences::Preferences)> {
        self.pending_save.take()
    }
    pub fn save_open_draft(&mut self) {
        self.model_ui.close_popup();
        if let Some(id) = &self.open
            && let Some(draft) = self.drafts.get(id)
        {
            self.pending_save = Some((id.clone(), draft.clone()));
        }
    }
    /// Called before the settings navigation, so Escape folds details first.
    pub fn handle_escape(&mut self, ctx: &egui::Context) -> bool {
        if self.model_ui.handle_escape(ctx) {
            return true;
        }
        let down = ctx.input(|i| i.key_down(egui::Key::Escape));
        if !down {
            self.escape_held = false;
        }
        if !ctx.input(|i| i.focused) || egui::Popup::is_any_open(ctx) {
            return false;
        }
        if !self.escape_held
            && (self.open.is_none()
                || !ctx.memory(|memory| {
                    match memory.focused() {
                        Some(id) => self.detail_focus.contains(&id),
                        // egui clears focus on Escape before this handler. A newly
                        // focused widget cannot install its event filter until
                        // the following frame, so retain that first-key boundary.
                        None if down => self
                            .detail_focus
                            .iter()
                            .any(|id| memory.had_focus_last_frame(*id)),
                        None => false,
                    }
                }))
        {
            return false;
        }
        let consumed = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if consumed {
            self.save_open_draft();
            self.open = None;
            if let Some(id) = self.open_row {
                ctx.memory_mut(|memory| memory.request_focus(id));
            }
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
        self.detail_focus.clear();
        for row in input
            .rows
            .iter()
            .filter(|row| row.available)
            .filter(matches)
        {
            let preset = row.preset;
            ui.push_id(preset.id, |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 8,
                        top: 0,
                        bottom: 0,
                    })
                    .corner_radius(6)
                    .show(ui, |ui| {
                        let expanded = self.open.as_deref() == Some(preset.id);
                        let controls = agent_row(ui, row, expanded, input.french);
                        if controls.toggle.clicked() {
                            action = Some(Action::SetEnabled(preset.id.into(), !row.enabled));
                        }
                        if controls.main.clicked() || controls.chevron.clicked() {
                            self.save_open_draft();
                            controls.main.request_focus();
                            self.open_row = Some(controls.main.id);
                            self.open = if expanded {
                                None
                            } else {
                                Some(preset.id.into())
                            };
                        }
                        if self.open.as_deref() == Some(preset.id) {
                            self.detail_focus.extend([
                                controls.main.id,
                                controls.toggle.id,
                                controls.chevron.id,
                            ]);
                        }
                        if self.open.as_deref() == Some(preset.id) {
                            egui::Frame::NONE
                                .fill(ui.visuals().faint_bg_color)
                                .inner_margin(10)
                                .corner_radius(6)
                                .show(ui, |ui| {
                                    let (model_change, model_focus, refresh) = model_defaults_ui(
                                        &mut self.model_ui,
                                        ui,
                                        row,
                                        input.french,
                                        input.discovery,
                                    );
                                    if refresh {
                                        action = Some(Action::RefreshModels(preset.id.into()));
                                    }
                                    if let Some(defaults) = model_change {
                                        action = Some(Action::SetModelDefaults(
                                            preset.id.into(),
                                            defaults,
                                        ));
                                    }
                                    ui.label(
                                        RichText::new(tr("How it starts", "Comment il démarre"))
                                            .small()
                                            .strong(),
                                    );
                                    let draft = self
                                        .drafts
                                        .entry(preset.id.into())
                                        .or_insert_with(|| row.prefs.clone());
                                    draft.enabled = row.enabled;
                                    let reset = ui.add_enabled(
                                        draft.customized(),
                                        egui::Button::new(tr("Reset", "Réinitialiser")),
                                    );
                                    self.detail_focus.push(reset.id);
                                    if reset.clicked() {
                                        draft.reset();
                                        self.pending_save = Some((preset.id.into(), draft.clone()));
                                    }
                                    let mut commit = false;
                                    for (label, value, hint, multiline) in [
                                        (
                                            tr("Command", "Commande"),
                                            &mut draft.command,
                                            preset.command,
                                            false,
                                        ),
                                        (tr("Arguments", "Arguments"), &mut draft.args, "", false),
                                        (
                                            tr(
                                                "Variables, one NAME=value per line",
                                                "Variables, un NOM=valeur par ligne",
                                            ),
                                            &mut draft.env,
                                            "",
                                            true,
                                        ),
                                    ] {
                                        let label = ui.label(label);
                                        let field = if multiline {
                                            egui::TextEdit::multiline(value).desired_rows(2)
                                        } else {
                                            egui::TextEdit::singleline(value)
                                        }
                                        .hint_text(hint)
                                        .desired_width(f32::INFINITY);
                                        let response = ui.add(field).labelled_by(label.id);
                                        self.detail_focus.push(response.id);
                                        commit |= response.lost_focus();
                                    }
                                    if let Err(error) = crate::agent_env::parse(&draft.env) {
                                        ui.colored_label(ui.visuals().error_fg_color, error);
                                        ui.label(tr(
                                            "Not used until fixed.",
                                            "Non utilisées tant que l’erreur n’est pas corrigée.",
                                        ));
                                    }
                                    ui.weak(tr(
                                        "Applies to panes you start from now on",
                                        "S’applique aux volets démarrés à partir de maintenant",
                                    ));
                                    if commit && *draft != row.prefs {
                                        self.pending_save = Some((preset.id.into(), draft.clone()));
                                    }
                                    ui.horizontal_wrapped(|ui| {
                                        let terminal = ui.button(tr(
                                            "Open in terminal",
                                            "Ouvrir dans un terminal",
                                        ));
                                        self.detail_focus.push(terminal.id);
                                        if terminal.clicked() {
                                            action = Some(Action::Terminal(preset.id.into()));
                                        }
                                        if matches!(preset.id, "codex" | "claude") {
                                            let native = ui
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
                                                ));
                                            self.detail_focus.push(native.id);
                                            if native.clicked() {
                                                action = Some(Action::NativeChat(preset.id.into()));
                                            }
                                        }
                                        if preset.docs.is_some() {
                                            let docs = ui.button(tr("Docs", "Documentation"));
                                            self.detail_focus.push(docs.id);
                                            if docs.clicked() {
                                                action = Some(Action::Docs(preset.id.into()));
                                            }
                                        }
                                    });
                                    self.detail_focus.extend(model_focus);
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
                            let (icon, _) = ui
                                .allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                            crate::agent_brand_icons::paint_preset(
                                ui.painter(),
                                icon,
                                preset,
                                false,
                            );
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
        if let Some(id) = ui.memory(|memory| memory.focused())
            && self.detail_focus.contains(&id)
        {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        escape: true,
                        ..Default::default()
                    },
                )
            });
        }
        action
    }
}

fn model_defaults_ui(
    picker: &mut crate::agent_model_defaults_ui::AgentModelDefaultsUi,
    ui: &mut Ui,
    row: &Row,
    french: bool,
    discovery: Option<&crate::agent_model_discovery::Discovery>,
) -> (
    Option<crate::agent_model_defaults::Defaults>,
    Vec<egui::Id>,
    bool,
) {
    use crate::agent_model_defaults_ui::{Capabilities, Input, ListedMeta, ProbeError, ProbeState};
    use crate::agent_model_probe::Failure;
    use crate::agent_model_process::Error;
    let id = row.preset.id;
    let models = discovery.and_then(|d| d.models(id));
    let entry = discovery
        .filter(|_| models.is_some())
        .and_then(|d| d.lists().get(id));
    let date = entry
        .and_then(|e| crate::regional_date::local(e.fetched_at, french))
        .unwrap_or_else(|| "—".into());
    let probe = if discovery.is_some_and(|d| d.busy(id)) {
        ProbeState::Busy
    } else if let Some(error) = discovery.and_then(|d| d.error(id)) {
        ProbeState::Error(match error {
            Failure::NotFound => ProbeError::NotFound,
            Failure::Empty => ProbeError::Empty,
            Failure::Unsupported | Failure::ProtocolRequired => ProbeError::Unsupported,
            Failure::Process(Error::Timeout) => ProbeError::Timeout,
            Failure::Process(Error::TooMuch) => ProbeError::TooMuch,
            Failure::Failed(detail) => ProbeError::Failed {
                detail: Some(detail),
            },
            _ => ProbeError::Failed { detail: None },
        })
    } else {
        ProbeState::Idle
    };
    let result = picker.show(
        ui,
        Input {
            agent_id: row.preset.id,
            agent_name: row.preset.name,
            defaults: &row.model_defaults,
            discovered_models: models,
            listed_meta: entry.map(|e| ListedMeta {
                count: e.models.len(),
                date_label: &date,
            }),
            probe,
            capabilities: Capabilities {
                can_refresh_models: discovery.is_some()
                    && crate::agent_model_discovery::Discovery::can_refresh(id),
                ..Default::default()
            },
            french,
            show_section_header: true,
        },
    );
    if discovery.is_some_and(|d| d.storage_error()) {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            if french {
                "La liste reste en mémoire; sa sauvegarde a échoué."
            } else {
                "The list remains in memory; saving it failed."
            },
        );
    }
    (result.changed, result.focus_ids, result.refresh_models)
}

struct RowControls {
    main: egui::Response,
    toggle: egui::Response,
    chevron: egui::Response,
}
fn agent_row(ui: &mut Ui, row: &Row, expanded: bool, french: bool) -> RowControls {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), egui::Sense::hover());
    let main_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2((rect.right() - 60.0).max(rect.left()), rect.bottom()),
    );
    let toggle_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 35.0, rect.center().y),
        egui::vec2(30.0, 18.0),
    );
    let chevron_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 7.0, rect.center().y),
        egui::vec2(14.0, 24.0),
    );
    let main = ui.interact(
        main_rect,
        ui.make_persistent_id("agent-row"),
        egui::Sense::click(),
    );
    let toggle = ui.interact(
        toggle_rect,
        ui.make_persistent_id("agent-visible"),
        egui::Sense::click(),
    );
    let chevron = ui.interact(
        chevron_rect,
        ui.make_persistent_id("agent-expand"),
        egui::Sense::click(),
    );
    main.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::CollapsingHeader,
            ui.is_enabled(),
            expanded,
            row.preset.name,
        )
    });
    let label = if french {
        format!("Afficher {} dans les menus", row.preset.name)
    } else {
        format!("Offer {} in menus", row.preset.name)
    };
    toggle.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            ui.is_enabled(),
            row.enabled,
            &label,
        )
    });
    chevron.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), row.preset.name)
    });
    if expanded || main.hovered() {
        ui.painter()
            .rect_filled(rect, 6, ui.visuals().faint_bg_color);
    }
    if main.has_focus() {
        ui.painter().rect_stroke(
            main_rect,
            4,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    let text_color = if row.enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().text_color().gamma_multiply(0.55)
    };
    let icon = egui::Rect::from_center_size(
        egui::pos2(main_rect.left() + 12.0, main_rect.center().y),
        egui::vec2(20.0, 20.0),
    );
    crate::agent_brand_icons::paint_preset(ui.painter(), icon, row.preset, !row.enabled);
    let title_rect = egui::Rect::from_min_max(
        main_rect.min + egui::vec2(32.0, 4.0),
        egui::pos2(main_rect.right() - 4.0, main_rect.top() + 23.0),
    );
    paint_line(
        ui,
        title_rect,
        row.preset.name,
        egui::FontId::proportional(12.5),
        text_color,
    );
    let desc = egui::Rect::from_min_max(
        egui::pos2(title_rect.left(), main_rect.top() + 24.0),
        egui::pos2(title_rect.right(), main_rect.bottom() - 3.0),
    );
    paint_line(
        ui,
        desc,
        if crate::agent_env::trim(&row.prefs.command).is_empty() {
            row.preset.command
        } else {
            &row.prefs.command
        },
        egui::FontId::monospace(11.0),
        ui.visuals().weak_text_color(),
    );
    let fill = if row.enabled {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    ui.painter().rect_filled(toggle_rect, 9, fill);
    ui.painter().circle_filled(
        egui::pos2(
            if row.enabled {
                toggle_rect.right() - 9.0
            } else {
                toggle_rect.left() + 9.0
            },
            toggle_rect.center().y,
        ),
        6.0,
        ui.visuals().text_color(),
    );
    if toggle.has_focus() {
        ui.painter().rect_stroke(
            toggle_rect.expand(2.0),
            10,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    let c = chevron_rect.center();
    let points = if expanded {
        [
            c + egui::vec2(-4.0, -2.0),
            c + egui::vec2(0.0, 2.5),
            c + egui::vec2(4.0, -2.0),
        ]
    } else {
        [
            c + egui::vec2(-2.0, -4.0),
            c + egui::vec2(2.5, 0.0),
            c + egui::vec2(-2.0, 4.0),
        ]
    };
    ui.painter().add(egui::Shape::line(
        points.to_vec(),
        egui::Stroke::new(1.5_f32, ui.visuals().weak_text_color()),
    ));
    RowControls {
        main: main.on_hover_text(row.preset.name),
        toggle: toggle.on_hover_text(label),
        chevron,
    }
}
fn paint_line(ui: &Ui, rect: egui::Rect, text: &str, font: egui::FontId, color: egui::Color32) {
    if rect.width() <= 1.0 {
        return;
    }
    let mut job = egui::text::LayoutJob::simple(text.into(), font, color, rect.width());
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.painter().layout_job(job);
    ui.painter().with_clip_rect(rect).galley(
        egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5),
        galley,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_models_are_selectable_and_refresh_emits_an_action_without_starting_a_process() {
        let dir = tempfile::tempdir().unwrap();
        crate::agent_model_cache::Cache::open(dir.path()).keep("codex", &serde_json::json!([{"id":"dynamic-model", "label":"Synthetic discovery", "effortLevels":["high"]}]), 1_700_000_000_000.0).unwrap();
        let discovery = crate::agent_model_discovery::Discovery::open(dir.path());
        let ctx = egui::Context::default();
        let mut panel = AgentsPanel {
            open: Some("codex".into()),
            ..Default::default()
        };
        let rows = [Row {
            preset: crate::agent_catalog::find("codex").unwrap(),
            available: true,
            enabled: true,
            prefs: Default::default(),
            model_defaults: Default::default(),
        }];
        let mut render = |events| {
            let mut action = None;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 900.0),
                    )),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        action = panel.show(
                            ui,
                            Input {
                                discovery: Some(&discovery),
                                rows: &rows,
                                loaded: true,
                                detecting: false,
                                error: None,
                                notice: None,
                                has_project: true,
                                french: false,
                                search: "",
                            },
                        );
                    });
                },
            );
            (output, action)
        };
        render(vec![]);
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        let (out, _) = render(vec![]);
        let pos = text_center(&out, "Refresh models");
        render(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
        assert_eq!(
            render(vec![pointer(pos, false)]).1,
            Some(Action::RefreshModels("codex".into()))
        );
        assert!(!discovery.active());
        let (out, _) = render(vec![]);
        let pos = text_center(&out, "Agent's own default");
        render(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
        render(vec![pointer(pos, false)]);
        let (out, _) = render(vec![]);
        let pos = text_center(&out, "Synthetic discovery");
        render(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
        let Some(Action::SetModelDefaults(id, defaults)) = render(vec![pointer(pos, false)]).1
        else {
            panic!("Expected selecting the cached model");
        };
        assert_eq!(id, "codex");
        assert_eq!(defaults.model.as_deref(), Some("dynamic-model"));
    }
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
                prefs: Default::default(),
                model_defaults: Default::default(),
            },
            Row {
                preset: crate::agent_catalog::find("kiro").unwrap(),
                available: false,
                enabled: true,
                prefs: Default::default(),
                model_defaults: Default::default(),
            },
        ];
        let mut rows = rows;
        rows.push(Row {
            preset: crate::agent_catalog::find("aider").unwrap(),
            available: false,
            enabled: true,
            prefs: Default::default(),
            model_defaults: Default::default(),
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
                                discovery: None,
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
    fn installed_brand_geometry_keeps_the_source_gap_before_text() {
        let ctx = egui::Context::default();
        let mut rect = egui::Rect::NOTHING;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 100.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let row = Row {
                        preset: crate::agent_catalog::find("codex").unwrap(),
                        available: true,
                        enabled: true,
                        prefs: Default::default(),
                        model_defaults: Default::default(),
                    };
                    rect = agent_row(ui, &row, false, false).main.rect;
                });
            },
        );
        let mut meshes = 0;
        let mut labels = 0;
        for clipped in output.shapes {
            match clipped.shape {
                egui::Shape::Mesh(mesh) => {
                    meshes += 1;
                    assert!(!mesh.indices.is_empty());
                    assert!(
                        mesh.vertices
                            .iter()
                            .all(|v| v.pos.x >= rect.left() + 1.9 && v.pos.x <= rect.left() + 22.1)
                    );
                }
                egui::Shape::Text(text) if text.galley.job.text == "Codex CLI" => {
                    labels += 1;
                    assert!(text.pos.x >= rect.left() + 32.0);
                }
                _ => {}
            }
        }
        assert!(meshes > 0, "brand painter must be connected to the row");
        assert_eq!(labels, 1);
    }

    #[test]
    fn model_popup_escape_precedes_detail_folding_and_held_repeats() {
        let ctx = egui::Context::default();
        let mut panel = AgentsPanel::default();
        frame(&ctx, &mut panel, vec![], true, true, 620.0);
        click(&ctx, &mut panel, "Codex CLI", true, true);
        click(&ctx, &mut panel, "Agent's own default", true, true);
        for _ in 0..3 {
            frame(&ctx, &mut panel, vec![], true, true, 620.0);
            assert!(panel.model_ui.is_popup_open());
        }
        let key = |pressed, repeat| egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut panel, vec![key(true, false)], true, true, 620.0);
        assert!(!panel.model_ui.is_popup_open());
        assert_eq!(panel.open.as_deref(), Some("codex"));
        frame(&ctx, &mut panel, vec![key(true, true)], true, true, 620.0);
        assert_eq!(panel.open.as_deref(), Some("codex"));
        frame(&ctx, &mut panel, vec![key(false, false)], true, true, 620.0);
        frame(&ctx, &mut panel, vec![key(true, false)], true, true, 620.0);
        assert!(panel.open.is_none());
    }

    #[test]
    fn model_picker_emits_only_an_explicit_model_choice() {
        let ctx = egui::Context::default();
        let mut panel = AgentsPanel::default();
        frame(&ctx, &mut panel, vec![], true, true, 620.0);
        click(&ctx, &mut panel, "Codex CLI", true, true);
        assert!(click(&ctx, &mut panel, "Agent's own default", true, true).is_none());
        let action = click(&ctx, &mut panel, "GPT-5.6 Sol", true, true);
        assert_eq!(
            action,
            Some(Action::SetModelDefaults(
                "codex".into(),
                crate::agent_model_defaults::Defaults {
                    model: Some("gpt-5.6-sol".into()),
                    ..Default::default()
                }
            ))
        );
    }

    #[test]
    fn edited_command_is_committed_before_launch_and_reset_clears_it() {
        let ctx = egui::Context::default();
        let mut panel = AgentsPanel::default();
        frame(&ctx, &mut panel, vec![], true, true, 620.0);
        click(&ctx, &mut panel, "Codex CLI", true, true);
        let command_id = panel.detail_focus[4];
        ctx.memory_mut(|m| m.request_focus(command_id));
        frame(
            &ctx,
            &mut panel,
            vec![egui::Event::Text("echo synthetic".into())],
            true,
            true,
            620.0,
        );
        assert!(
            panel.take_save().is_none(),
            "typing is not a commit before blur"
        );
        assert_eq!(
            click(&ctx, &mut panel, "Open in terminal", true, true),
            Some(Action::Terminal("codex".into()))
        );
        let (id, prefs) = panel
            .take_save()
            .expect("blur must commit before launch action is handled");
        assert_eq!(id, "codex");
        assert_eq!(prefs.command, "echo synthetic");
        click(&ctx, &mut panel, "Reset", true, true);
        assert!(!panel.take_save().unwrap().1.customized());
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
    fn escape_leaves_external_focus_alone_and_space_operates_the_visibility_switch() {
        let ctx = egui::Context::default();
        let mut panel = AgentsPanel::default();
        frame(&ctx, &mut panel, vec![], true, true, 620.0);
        click(&ctx, &mut panel, "Codex CLI", true, true);
        let toggle_id = panel.detail_focus[1];
        ctx.memory_mut(|memory| memory.request_focus(toggle_id));
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let (_, action, _) = frame(
            &ctx,
            &mut panel,
            vec![key(egui::Key::Space)],
            true,
            true,
            620.0,
        );
        assert_eq!(action, Some(Action::SetEnabled("codex".into(), false)));
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("external-settings-search")));
        frame(
            &ctx,
            &mut panel,
            vec![key(egui::Key::Escape)],
            true,
            true,
            620.0,
        );
        assert_eq!(panel.open.as_deref(), Some("codex"));
        assert!(ctx.input(|input| input.key_pressed(egui::Key::Escape)));
    }

    #[test]
    fn escape_folds_details_and_repeat_does_not_escape_to_parent() {
        let ctx = egui::Context::default();
        let mut panel = AgentsPanel::default();
        frame(&ctx, &mut panel, vec![], true, true, 620.0);
        click(&ctx, &mut panel, "Codex CLI", true, true);
        let row_id = panel.open_row.unwrap();
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(row_id),
            "pointer should focus row"
        );
        assert!(panel.detail_focus.contains(&row_id));
        let key = |repeat| egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: egui::Modifiers::NONE,
        };
        let (_, action, right) = frame(&ctx, &mut panel, vec![key(false)], true, true, 300.0);
        assert!(panel.open.is_none());
        assert!(ctx.memory(|memory| memory.has_focus(row_id)));
        assert!(action.is_none());
        assert!(right <= 300.0);
        frame(&ctx, &mut panel, vec![key(true)], true, true, 300.0);
        assert!(!ctx.input(|i| i.key_pressed(egui::Key::Escape)));
    }
}
