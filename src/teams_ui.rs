//! Native `egui` renderers for Tessel Teams, Worker Orchestration confirmation modals,
//! Inter-Project Relay pair approval modals, unconfirmed-idle wake banners, and
//! pane-header team/worker/unread badges.

use eframe::egui::{self, Color32, FontId, Key, RichText, Stroke, Vec2};

use crate::teams_relay::{
    RELAY_HOURLY_CHOICES, RelayApprovalPrompt, SessionTeamBadge, TeamsRelayHub, WakeBanner,
    WakeChannel, WorkerRecord, WorkerState, dedupe_team_numbers, now_ms,
};
use crate::theme_palettes::UiPalette;
use crate::workspace::Workspace;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TeamsUiAction {
    ApproveWorker(String),
    RefuseWorker(String),
    ApproveRelayPair { pair: (u64, u64), hourly_limit: u32 },
    DeclineRelayPair((u64, u64)),
    SendWakeReminder(u64),
    DismissWakeBanner(u64),
    CreateTeam { project: u64, name: String },
    DeleteTeam(String),
    ToggleMember { team_id: String, pane_id: u64 },
    ToggleLead { team_id: String, pane_id: u64 },
    StopWorker(String),
    ReleaseWorker(String),
    FocusPane(u64),
}

#[derive(Clone, Debug, Default)]
pub struct TeamsUiState {
    pub manager_project: Option<u64>,
    pub new_team_name: String,
}

impl TeamsUiState {
    pub fn open_manager(&mut self, project_id: u64) {
        self.manager_project = Some(project_id);
    }

    pub fn close_manager(&mut self) {
        self.manager_project = None;
    }
}

/// Renders `#<num>`, `.lead` accent badge, `worker of <name>` / `exécutant de <name>`,
/// and `✉ N` unread badge inside a pane header.
pub fn paint_pane_team_pills(
    ui: &mut egui::Ui,
    pal: &UiPalette,
    badge: &SessionTeamBadge,
    french: bool,
) -> bool {
    let mut clicked_team = false;
    if !badge.team_number.is_empty() {
        let label = if badge.is_lead {
            if french {
                format!("#{} · chef", badge.team_number)
            } else {
                format!("#{} · lead", badge.team_number)
            }
        } else {
            format!("#{}", badge.team_number)
        };
        let fg = if badge.is_lead {
            pal.accent
        } else {
            pal.text_dim
        };
        let bg = if badge.is_lead {
            pal.accent.gamma_multiply(0.16)
        } else {
            pal.surface_2
        };
        let resp = egui::Frame::new()
            .fill(bg)
            .corner_radius(4.0_f32)
            .stroke(Stroke::new(1.0_f32, fg.gamma_multiply(0.45)))
            .inner_margin(egui::Margin::symmetric(5, 1))
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(label)
                            .font(FontId::proportional(10.5))
                            .color(fg),
                    )
                    .selectable(false),
                )
            });
        let hover = if badge.team_name.is_empty() {
            String::new()
        } else if french {
            format!("Équipe : {}", badge.team_name)
        } else {
            format!("Team: {}", badge.team_name)
        };
        let interact = resp.response.interact(egui::Sense::click());
        if !hover.is_empty() {
            clicked_team |= interact.on_hover_text(hover).clicked();
        } else {
            clicked_team |= interact.clicked();
        }
    }

    if let Some(coord) = &badge.worker_of {
        let tag = if french {
            format!("exécutant de {coord}")
        } else {
            format!("worker of {coord}")
        };
        egui::Frame::new()
            .fill(pal.surface_2)
            .corner_radius(4.0_f32)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .inner_margin(egui::Margin::symmetric(5, 1))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(tag)
                        .font(FontId::proportional(10.5))
                        .color(pal.text_dim),
                );
            });
    }

    if badge.unread_count > 0 {
        let pill = format!("✉ {}", badge.unread_count);
        let tip = if french {
            format!("{} message(s) non lu(s)", badge.unread_count)
        } else {
            format!("{} unread message(s)", badge.unread_count)
        };
        egui::Frame::new()
            .fill(pal.accent.gamma_multiply(0.18))
            .corner_radius(4.0_f32)
            .stroke(Stroke::new(1.0_f32, pal.accent.gamma_multiply(0.55)))
            .inner_margin(egui::Margin::symmetric(5, 1))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(pill)
                        .font(FontId::proportional(10.5))
                        .color(pal.accent)
                        .strong(),
                );
            })
            .response
            .on_hover_text(tip);
    }

    clicked_team
}

/// Non-intrusive reminder bar shown when an agent's idle state is unconfirmed
/// and `!orchestration_prefs.team_wake_unconfirmed`.
pub fn show_unconfirmed_wake_banner(
    ui: &mut egui::Ui,
    pal: &UiPalette,
    banner: &WakeBanner,
    french: bool,
) -> Option<TeamsUiAction> {
    let mut out = None;
    let msg = match (banner.channel, french) {
        (WakeChannel::Team, false) => "Unread team message waiting (idle state not confirmed)",
        (WakeChannel::Team, true) => "Message d'équipe en attente (inactivité non confirmée)",
        (WakeChannel::Relay, false) => {
            "Unread inter-project message waiting (idle state not confirmed)"
        }
        (WakeChannel::Relay, true) => "Message inter-projets en attente (inactivité non confirmée)",
    };
    let send_lbl = if french {
        "Lui envoyer le rappel"
    } else {
        "Send it the reminder"
    };
    let close_lbl = if french { "Ignorer" } else { "Dismiss" };

    egui::Frame::new()
        .fill(pal.surface_2)
        .stroke(Stroke::new(1.0_f32, pal.border))
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(msg)
                        .font(FontId::proportional(11.5))
                        .color(pal.text),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(close_lbl)
                                    .font(FontId::proportional(11.0))
                                    .color(pal.text_dim),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        out = Some(TeamsUiAction::DismissWakeBanner(banner.pane_id));
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(send_lbl)
                                    .font(FontId::proportional(11.0))
                                    .color(pal.accent)
                                    .strong(),
                            )
                            .fill(pal.accent.gamma_multiply(0.14))
                            .stroke(Stroke::new(1.0_f32, pal.accent.gamma_multiply(0.5))),
                        )
                        .on_hover_text(&banner.reminder_line)
                        .clicked()
                    {
                        out = Some(TeamsUiAction::SendWakeReminder(banner.pane_id));
                    }
                });
            });
        });
    out
}

/// Modal confirmation card when `confirm_workers == true` and a coordinator calls `team_worker_start`.
pub fn show_worker_confirm_modal(
    ctx: &egui::Context,
    pal: &UiPalette,
    workers: &[WorkerRecord],
    french: bool,
) -> Option<TeamsUiAction> {
    let confirming = workers
        .iter()
        .find(|w| w.state == WorkerState::Confirming)?;
    let mut action = None;

    egui::Area::new(egui::Id::new("tessel-worker-confirm-backdrop"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::Pos2::ZERO)
        .show(ctx, |ui| {
            let screen = ctx.content_rect();
            ui.painter()
                .rect_filled(screen, 0.0_f32, Color32::from_black_alpha(150));
            let _ = ui.allocate_rect(screen, egui::Sense::click());
        });

    let title = if french {
        "Confirmer le démarrage d'un agent exécutant"
    } else {
        "Confirm worker agent launch"
    };

    egui::Window::new(title)
        .id(egui::Id::new("tessel-worker-confirm-modal"))
        .order(egui::Order::Tooltip)
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(480.0_f32, 280.0_f32))
        .frame(
            egui::Frame::new()
                .fill(pal.surface)
                .corner_radius(10.0_f32)
                .stroke(Stroke::new(1.0_f32, pal.border_strong))
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                action = Some(TeamsUiAction::RefuseWorker(confirming.id.clone()));
            }
            ui.label(
                RichText::new(title)
                    .font(FontId::proportional(15.0))
                    .color(pal.text_strong)
                    .strong(),
            );
            ui.add_space(8.0_f32);
            let meta = if french {
                format!(
                    "{} souhaite démarrer « {} » ({}) dans {}",
                    confirming.coordinator_name,
                    confirming.worker_name,
                    confirming.agent,
                    confirming.frozen.cwd.display()
                )
            } else {
                format!(
                    "{} wants to start \"{}\" ({}) in {}",
                    confirming.coordinator_name,
                    confirming.worker_name,
                    confirming.agent,
                    confirming.frozen.cwd.display()
                )
            };
            ui.label(
                RichText::new(meta)
                    .font(FontId::proportional(12.5))
                    .color(pal.text),
            );
            ui.add_space(8.0_f32);
            egui::Frame::new()
                .fill(pal.surface_2)
                .corner_radius(6.0_f32)
                .stroke(Stroke::new(1.0_f32, pal.border))
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(120.0_f32)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(&confirming.task_goal)
                                    .font(FontId::monospace(11.5))
                                    .color(pal.text),
                            );
                        });
                });
            ui.add_space(12.0_f32);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let approve_lbl = if french {
                        "Démarrer l'exécutant"
                    } else {
                        "Start worker"
                    };
                    let refuse_lbl = if french { "Refuser" } else { "Refuse" };
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(approve_lbl).color(Color32::WHITE).strong(),
                            )
                            .fill(pal.accent)
                            .min_size(Vec2::new(120.0_f32, 30.0_f32)),
                        )
                        .clicked()
                    {
                        action = Some(TeamsUiAction::ApproveWorker(confirming.id.clone()));
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new(refuse_lbl).color(pal.text))
                                .fill(pal.surface_2)
                                .stroke(Stroke::new(1.0_f32, pal.border))
                                .min_size(Vec2::new(92.0_f32, 30.0_f32)),
                        )
                        .clicked()
                    {
                        action = Some(TeamsUiAction::RefuseWorker(confirming.id.clone()));
                    }
                });
            });
        });

    action
}

/// Modal pair-approval dialog for `project_send` across distinct projects.
pub fn show_relay_approval_modal(
    ctx: &egui::Context,
    pal: &UiPalette,
    prompts: &mut std::collections::VecDeque<RelayApprovalPrompt>,
    french: bool,
) -> Option<TeamsUiAction> {
    let prompt = prompts.front_mut()?;
    let mut action = None;

    egui::Area::new(egui::Id::new("tessel-relay-approve-backdrop"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::Pos2::ZERO)
        .show(ctx, |ui| {
            let screen = ctx.content_rect();
            ui.painter()
                .rect_filled(screen, 0.0_f32, Color32::from_black_alpha(150));
            let _ = ui.allocate_rect(screen, egui::Sense::click());
        });

    let title = if french {
        "Autoriser le relais inter-projets ?"
    } else {
        "Allow inter-project agent relay?"
    };

    egui::Window::new(title)
        .id(egui::Id::new("tessel-relay-approve-modal"))
        .order(egui::Order::Tooltip)
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(512.0_f32, 320.0_f32))
        .frame(
            egui::Frame::new()
                .fill(pal.surface)
                .corner_radius(10.0_f32)
                .stroke(Stroke::new(1.0_f32, pal.border_strong))
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                action = Some(TeamsUiAction::DeclineRelayPair(prompt.pair));
            }
            ui.label(
                RichText::new(title)
                    .font(FontId::proportional(15.0))
                    .color(pal.text_strong)
                    .strong(),
            );
            ui.add_space(6.0_f32);
            let desc = if french {
                format!(
                    "{} ({}) souhaite envoyer un message à {} ({}). L'autorisation est bidirectionnelle entre ces deux projets.",
                    prompt.from_agent_name,
                    prompt.from_project_name,
                    prompt.to_agent_name,
                    prompt.to_project_name
                )
            } else {
                format!(
                    "{} ({}) wants to message {} ({}). Approval applies bidirectionally between both projects.",
                    prompt.from_agent_name,
                    prompt.from_project_name,
                    prompt.to_agent_name,
                    prompt.to_project_name
                )
            };
            ui.label(
                RichText::new(desc)
                    .font(FontId::proportional(12.0))
                    .color(pal.text),
            );
            if prompt.live_share_active {
                ui.add_space(4.0_f32);
                let warn_txt = if french {
                    "Live Share est actif : cette approbation ne sera pas mise en cache."
                } else {
                    "Live Share is active: this approval will apply once and will not be cached."
                };
                ui.label(
                    RichText::new(warn_txt)
                        .font(FontId::proportional(11.5))
                        .color(pal.warn),
                );
            }
            ui.add_space(8.0_f32);
            egui::Frame::new()
                .fill(pal.surface_2)
                .corner_radius(6.0_f32)
                .stroke(Stroke::new(1.0_f32, pal.border))
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(&prompt.preview_text)
                            .font(FontId::monospace(11.5))
                            .color(pal.text),
                    );
                });
            ui.add_space(10.0_f32);
            ui.horizontal(|ui| {
                let rate_lbl = if french {
                    "Plafond horaire :"
                } else {
                    "Hourly cap:"
                };
                ui.label(
                    RichText::new(rate_lbl)
                        .font(FontId::proportional(12.0))
                        .color(pal.text_dim),
                );
                for cap in RELAY_HOURLY_CHOICES {
                    let selected = prompt.selected_hourly_limit == cap;
                    let pill = format!("{cap}/h");
                    if ui.selectable_label(selected, pill).clicked() {
                        prompt.selected_hourly_limit = cap;
                    }
                }
            });
            ui.add_space(12.0_f32);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let allow_lbl = if french { "Autoriser" } else { "Allow pair" };
                    let decline_lbl = if french {
                        "Refuser (10 min)"
                    } else {
                        "Decline (10m)"
                    };
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(allow_lbl).color(Color32::WHITE).strong(),
                            )
                            .fill(pal.accent)
                            .min_size(Vec2::new(112.0_f32, 30.0_f32)),
                        )
                        .clicked()
                    {
                        action = Some(TeamsUiAction::ApproveRelayPair {
                            pair: prompt.pair,
                            hourly_limit: prompt.selected_hourly_limit,
                        });
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new(decline_lbl).color(pal.text))
                                .fill(pal.surface_2)
                                .stroke(Stroke::new(1.0_f32, pal.border))
                                .min_size(Vec2::new(112.0_f32, 30.0_f32)),
                        )
                        .clicked()
                    {
                        action = Some(TeamsUiAction::DeclineRelayPair(prompt.pair));
                    }
                });
            });
        });

    action
}

/// Project-scoped Teams & Workers management dialog.
pub fn show_team_manager_modal(
    ctx: &egui::Context,
    pal: &UiPalette,
    ui_state: &mut TeamsUiState,
    workspace: &Workspace,
    hub: &TeamsRelayHub,
    french: bool,
) -> Option<TeamsUiAction> {
    let project_id = ui_state.manager_project?;
    let project_name = workspace
        .projects
        .iter()
        .find(|p| p.id == project_id)
        .map_or_else(|| format!("Project {project_id}"), |p| p.name.clone());
    let mut action = None;
    let mut close = false;

    egui::Area::new(egui::Id::new("tessel-teams-mgr-backdrop"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::Pos2::ZERO)
        .show(ctx, |ui| {
            let screen = ctx.content_rect();
            ui.painter()
                .rect_filled(screen, 0.0_f32, Color32::from_black_alpha(140));
            if ui.allocate_rect(screen, egui::Sense::click()).clicked() {
                close = true;
            }
        });

    let title = if french {
        format!("Équipes & Exécutants — {project_name}")
    } else {
        format!("Teams & Workers — {project_name}")
    };

    egui::Window::new(&title)
        .id(egui::Id::new("tessel-teams-mgr-modal"))
        .order(egui::Order::Tooltip)
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(620.0_f32, 460.0_f32))
        .frame(
            egui::Frame::new()
                .fill(pal.surface)
                .corner_radius(10.0_f32)
                .stroke(Stroke::new(1.0_f32, pal.border_strong))
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                close = true;
            }
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&title)
                        .font(FontId::proportional(15.0))
                        .color(pal.text_strong)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new("✕").frame(false)).clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(8.0_f32);

            ui.horizontal(|ui| {
                let hint = if french {
                    "Nom d'équipe (ex. Équipe 1)"
                } else {
                    "Team name (e.g. Team 1)"
                };
                ui.add(
                    egui::TextEdit::singleline(&mut ui_state.new_team_name)
                        .hint_text(hint)
                        .desired_width(240.0_f32),
                );
                let add_lbl = if french {
                    "Créer une équipe"
                } else {
                    "Create team"
                };
                if ui
                    .add(
                        egui::Button::new(RichText::new(add_lbl).color(Color32::WHITE))
                            .fill(pal.accent),
                    )
                    .clicked()
                {
                    let name = if ui_state.new_team_name.trim().is_empty() {
                        let idx = workspace
                            .teams
                            .iter()
                            .filter(|t| t.project == project_id)
                            .count()
                            + 1;
                        if french {
                            format!("Équipe {idx}")
                        } else {
                            format!("Team {idx}")
                        }
                    } else {
                        ui_state.new_team_name.trim().to_owned()
                    };
                    ui_state.new_team_name.clear();
                    action = Some(TeamsUiAction::CreateTeam {
                        project: project_id,
                        name,
                    });
                }
            });

            ui.add_space(10.0_f32);
            let numbers = dedupe_team_numbers(&workspace.teams);
            let project_sessions: Vec<_> = workspace
                .sessions
                .iter()
                .filter(|s| s.project == Some(project_id))
                .collect();

            egui::ScrollArea::vertical()
                .max_height(355.0_f32)
                .show(ui, |ui| {
                    let project_teams: Vec<_> = workspace
                        .teams
                        .iter()
                        .filter(|t| t.project == project_id)
                        .collect();
                    if project_teams.is_empty() {
                        let empty = if french {
                            "Aucune équipe active dans ce projet."
                        } else {
                            "No teams created in this project yet."
                        };
                        ui.label(RichText::new(empty).color(pal.text_dim));
                    }
                    for team in project_teams {
                        let num = numbers
                            .get(&team.id)
                            .cloned()
                            .unwrap_or_else(|| "1".to_owned());
                        egui::Frame::new()
                            .fill(pal.surface_2)
                            .corner_radius(6.0_f32)
                            .stroke(Stroke::new(1.0_f32, pal.border))
                            .inner_margin(egui::Margin::same(10))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("#{num} {}", team.name))
                                            .color(pal.text_strong)
                                            .strong(),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let del_lbl =
                                                if french { "Supprimer" } else { "Delete" };
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new(del_lbl).color(pal.danger),
                                                    )
                                                    .frame(false),
                                                )
                                                .clicked()
                                            {
                                                action = Some(TeamsUiAction::DeleteTeam(
                                                    team.id.clone(),
                                                ));
                                            }
                                        },
                                    );
                                });
                                ui.add_space(4.0_f32);
                                for sess in &project_sessions {
                                    let enrolled = team.members.contains(&sess.id);
                                    let is_lead = team.lead_pane == Some(sess.id);
                                    let name =
                                        sess.pane_name.as_deref().unwrap_or(sess.title.as_str());
                                    let unread = hub.unread_team_count(sess.id);
                                    ui.horizontal(|ui| {
                                        let mut checked = enrolled;
                                        if ui.checkbox(&mut checked, name).changed() {
                                            action = Some(TeamsUiAction::ToggleMember {
                                                team_id: team.id.clone(),
                                                pane_id: sess.id,
                                            });
                                        }
                                        if enrolled {
                                            let lead_lbl = if is_lead {
                                                if french { "★ chef" } else { "★ lead" }
                                            } else if french {
                                                "nommer chef"
                                            } else {
                                                "make lead"
                                            };
                                            if ui.selectable_label(is_lead, lead_lbl).clicked() {
                                                action = Some(TeamsUiAction::ToggleLead {
                                                    team_id: team.id.clone(),
                                                    pane_id: sess.id,
                                                });
                                            }
                                            if unread > 0 {
                                                ui.label(
                                                    RichText::new(format!("✉ {unread}"))
                                                        .color(pal.accent),
                                                );
                                            }
                                        }
                                    });
                                }
                            });
                        ui.add_space(6.0_f32);
                    }

                    let project_workers: Vec<_> = hub
                        .workers
                        .iter()
                        .filter(|w| w.frozen.project_id == project_id)
                        .collect();
                    if !project_workers.is_empty() {
                        ui.add_space(6.0_f32);
                        let hdr = if french {
                            "Agents exécutants"
                        } else {
                            "Orchestrated workers"
                        };
                        ui.label(RichText::new(hdr).color(pal.text_strong).strong());
                        let now = now_ms();
                        for w in project_workers {
                            egui::Frame::new()
                                .fill(pal.surface_2)
                                .corner_radius(6.0_f32)
                                .stroke(Stroke::new(1.0_f32, pal.border))
                                .inner_margin(egui::Margin::same(8))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let mut status = w.state.label(french).to_owned();
                                        if w.is_stale(now) {
                                            status.push_str(if french {
                                                " · silencieux"
                                            } else {
                                                " · stale"
                                            });
                                        }
                                        ui.label(
                                            RichText::new(format!(
                                                "{} ({}) — {status}",
                                                w.worker_name, w.agent
                                            ))
                                            .color(pal.text)
                                            .strong(),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if w.state.is_active() {
                                                    let stop_lbl =
                                                        if french { "Arrêter" } else { "Stop" };
                                                    if ui.small_button(stop_lbl).clicked() {
                                                        action = Some(TeamsUiAction::StopWorker(
                                                            w.id.clone(),
                                                        ));
                                                    }
                                                } else if w.state != WorkerState::Released {
                                                    let rel_lbl =
                                                        if french { "Libérer" } else { "Release" };
                                                    if ui.small_button(rel_lbl).clicked() {
                                                        action =
                                                            Some(TeamsUiAction::ReleaseWorker(
                                                                w.id.clone(),
                                                            ));
                                                    }
                                                }
                                                if let Some(pid) = w.worker_pane_id
                                                    && ui
                                                        .small_button(if french {
                                                            "Voir"
                                                        } else {
                                                            "Focus"
                                                        })
                                                        .clicked()
                                                {
                                                    action = Some(TeamsUiAction::FocusPane(pid));
                                                }
                                            },
                                        );
                                    });
                                    if let Some(sum) = &w.summary {
                                        ui.label(
                                            RichText::new(sum)
                                                .font(FontId::proportional(11.5))
                                                .color(pal.text_dim),
                                        );
                                    }
                                });
                            ui.add_space(4.0_f32);
                        }
                    }
                });
        });

    if close {
        ui_state.close_manager();
    }
    action
}
