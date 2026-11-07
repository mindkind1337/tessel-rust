//! Pure-UI `egui` modal for scanning and batch-cleaning secondary Git worktrees
//! (`Safe` pre-selected, `Risk` opt-in with force flag, `Blocker` disabled).

use std::{collections::HashSet, path::PathBuf};

use eframe::egui::{self, Color32, CornerRadius, FontId, Key, Margin, Order, RichText, Stroke};

use crate::worktree::{CleanupBlocker, CleanupItem, CleanupRisk};

const SAFE_GREEN: Color32 = Color32::from_rgb(0x22, 0xc5, 0x5e);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorktreeCleanupAction {
    Refresh,
    /// `(worktree_path, branch, force)`
    RemoveSelected(Vec<(PathBuf, String, bool)>),
    Close,
}

#[derive(Clone, Debug)]
pub struct WorktreeCleanupModal {
    pub project_id: u64,
    pub project_name: String,
    pub default_branch: String,
    pub items: Vec<CleanupItem>,
    pub selected: HashSet<PathBuf>,
    pub scanning: bool,
    pub removing: bool,
    pub confirming: bool,
    pub error: Option<String>,
}

impl WorktreeCleanupModal {
    #[must_use]
    pub fn new_scanning(project_id: u64, project_name: impl Into<String>) -> Self {
        Self {
            project_id,
            project_name: project_name.into(),
            default_branch: "main".to_owned(),
            items: Vec::new(),
            selected: HashSet::new(),
            scanning: true,
            removing: false,
            confirming: false,
            error: None,
        }
    }

    pub fn set_scan_results(&mut self, default_branch: String, items: Vec<CleanupItem>) {
        self.default_branch = default_branch;
        self.selected = items
            .iter()
            .filter(|item| item.is_safe())
            .map(|item| item.path.clone())
            .collect();
        self.items = items;
        self.scanning = false;
        self.removing = false;
        self.confirming = false;
        self.error = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.scanning = false;
        self.removing = false;
        self.confirming = false;
        self.error = Some(error);
    }

    pub fn select_safe(&mut self) {
        self.selected = self
            .items
            .iter()
            .filter(|item| item.is_safe())
            .map(|item| item.path.clone())
            .collect();
        self.confirming = false;
    }

    pub fn select_none(&mut self) {
        self.selected.clear();
        self.confirming = false;
    }

    #[must_use]
    pub fn selected_targets(&self) -> Vec<(PathBuf, String, bool)> {
        self.items
            .iter()
            .filter(|item| item.is_selectable() && self.selected.contains(&item.path))
            .map(|item| (item.path.clone(), item.branch.clone(), !item.is_safe()))
            .collect()
    }

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        is_en: bool,
        confirm_before_remove: bool,
    ) -> Option<WorktreeCleanupAction> {
        let mut out_action = None;

        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            if self.confirming {
                self.confirming = false;
            } else {
                return Some(WorktreeCleanupAction::Close);
            }
        }

        let screen = ctx.content_rect();
        let mut backdrop_click_pos: Option<egui::Pos2> = None;
        egui::Area::new(egui::Id::new("worktree-cleanup-backdrop"))
            .order(Order::Middle)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::from_black_alpha(150));
                if resp.clicked() && !self.removing {
                    backdrop_click_pos = resp.interact_pointer_pos();
                }
            });

        let modal_w = (screen.width() - 48.0).clamp(360.0, 660.0);
        let modal_max_h = (screen.height() - 64.0).clamp(300.0, 620.0);
        let modal_pos = egui::pos2(
            screen.center().x - modal_w * 0.5,
            screen.center().y - modal_max_h * 0.5,
        );
        let style_vis = ctx.style().visuals.clone();

        let area_resp = egui::Area::new(egui::Id::new("worktree-cleanup-window"))
            .order(Order::Foreground)
            .fixed_pos(modal_pos)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(style_vis.window_fill)
                    .stroke(Stroke::new(1.0_f32, style_vis.window_stroke.color))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width((modal_w - 32.0).max(200.0));
                        ui.set_max_height((modal_max_h - 32.0).max(200.0));
                        let fg_strong = ui.visuals().strong_text_color();
                        let fg_body = ui.visuals().text_color();
                        let fg_muted = ui.visuals().weak_text_color();
                        let danger = ui.visuals().error_fg_color;
                        let warn = ui.visuals().warn_fg_color;
                        let surface = ui.visuals().panel_fill;

                // Header
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(if is_en {
                                "Clean up worktrees"
                            } else {
                                "Nettoyer les worktrees"
                            })
                            .font(FontId::proportional(15.5))
                            .strong()
                            .color(fg_strong),
                        );
                        let subtitle = if is_en {
                            format!(
                                "{} · compared with {}",
                                self.project_name, self.default_branch
                            )
                        } else {
                            format!(
                                "{} · comparé à {}",
                                self.project_name, self.default_branch
                            )
                        };
                        ui.label(
                            RichText::new(subtitle)
                                .font(FontId::proportional(11.5))
                                .color(fg_muted),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let close_btn = ui.add_enabled(
                            !self.removing,
                            egui::Button::new(RichText::new("×").size(16.0)).frame(false),
                        );
                        if close_btn.clicked() {
                            out_action = Some(WorktreeCleanupAction::Close);
                        }
                    });
                });

                ui.add_space(10.0);

                // Toolbar
                ui.horizontal(|ui| {
                    let controls_enabled = !self.scanning && !self.removing;
                    if ui
                        .add_enabled(
                            controls_enabled,
                            egui::Button::new(if is_en {
                                "Select safe"
                            } else {
                                "Sélectionner les sûrs"
                            }),
                        )
                        .clicked()
                    {
                        self.select_safe();
                    }
                    if ui
                        .add_enabled(
                            controls_enabled,
                            egui::Button::new(if is_en {
                                "Select none"
                            } else {
                                "Tout désélectionner"
                            }),
                        )
                        .clicked()
                    {
                        self.select_none();
                    }
                    if ui
                        .add_enabled(
                            controls_enabled,
                            egui::Button::new(if is_en { "Refresh" } else { "Actualiser" }),
                        )
                        .clicked()
                    {
                        self.scanning = true;
                        self.confirming = false;
                        out_action = Some(WorktreeCleanupAction::Refresh);
                    }
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);

                if let Some(err) = &self.error {
                    egui::Frame::new()
                        .fill(danger.gamma_multiply(0.14))
                        .stroke(Stroke::new(1.0_f32, danger))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(err)
                                    .font(FontId::proportional(12.0))
                                    .color(fg_strong),
                            );
                        });
                    ui.add_space(8.0);
                }

                let scroll_h = (modal_max_h - 190.0).max(120.0);
                egui::ScrollArea::vertical()
                    .max_height(scroll_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.scanning {
                            ui.vertical_centered(|ui| {
                                ui.add_space(32.0);
                                ui.spinner();
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(if is_en {
                                        "Scanning worktrees…"
                                    } else {
                                        "Analyse des worktrees…"
                                    })
                                    .color(fg_body),
                                );
                            });
                        } else if self.items.is_empty() {
                            ui.vertical_centered(|ui| {
                                ui.add_space(36.0);
                                ui.label(
                                    RichText::new(if is_en {
                                        "No secondary worktrees found for this project."
                                    } else {
                                        "Aucun worktree secondaire trouvé pour ce projet."
                                    })
                                    .color(fg_muted),
                                );
                            });
                        } else {
                            for item in &self.items {
                                render_cleanup_card(
                                    ui,
                                    item,
                                    &mut self.selected,
                                    &mut self.confirming,
                                    self.removing,
                                    is_en,
                                );
                                ui.add_space(6.0);
                            }
                        }
                    });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);

                let selected_targets = self.selected_targets();
                let selected_count = selected_targets.len();

                if self.confirming {
                    let prompt = if is_en {
                        format!(
                            "Remove {selected_count} worktree(s)? Their folders and local branches will be deleted."
                        )
                    } else {
                        format!(
                            "Supprimer {selected_count} worktree(s) ? Leurs dossiers et branches locales seront supprimés."
                        )
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(prompt)
                                .font(FontId::proportional(12.0))
                                .color(warn),
                        );
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                !self.removing,
                                egui::Button::new(if is_en { "Back" } else { "Retour" }),
                            )
                            .clicked()
                        {
                            self.confirming = false;
                        }
                        let confirm_btn = egui::Button::new(
                            RichText::new(if is_en {
                                "Confirm delete"
                            } else {
                                "Confirmer la suppression"
                            })
                            .color(Color32::WHITE),
                        )
                        .fill(danger);
                        if ui
                            .add_enabled(!self.removing && selected_count > 0, confirm_btn)
                            .clicked()
                        {
                            self.removing = true;
                            self.confirming = false;
                            out_action =
                                Some(WorktreeCleanupAction::RemoveSelected(selected_targets));
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        let summary_label = if is_en {
                            format!("{selected_count} selected")
                        } else {
                            format!("{selected_count} sélectionné(s)")
                        };
                        ui.label(
                            RichText::new(summary_label)
                                .font(FontId::proportional(12.0))
                                .color(fg_muted),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let remove_text = if self.removing {
                                if is_en {
                                    "Removing…".to_owned()
                                } else {
                                    "Suppression…".to_owned()
                                }
                            } else if is_en {
                                format!("Remove selected ({selected_count})")
                            } else {
                                format!("Supprimer la sélection ({selected_count})")
                            };
                            let btn = egui::Button::new(RichText::new(remove_text).color(
                                if selected_count > 0 {
                                    Color32::WHITE
                                } else {
                                    fg_muted
                                },
                            ))
                            .fill(if selected_count > 0 { danger } else { surface });
                            if ui
                                .add_enabled(
                                    !self.scanning && !self.removing && selected_count > 0,
                                    btn,
                                )
                                .clicked()
                            {
                                if confirm_before_remove {
                                    self.confirming = true;
                                } else {
                                    self.removing = true;
                                    out_action =
                                        Some(WorktreeCleanupAction::RemoveSelected(selected_targets));
                                }
                            }
                            if ui
                                .add_enabled(
                                    !self.removing,
                                    egui::Button::new(if is_en { "Cancel" } else { "Annuler" }),
                                )
                                .clicked()
                            {
                                out_action = Some(WorktreeCleanupAction::Close);
                            }
                        });
                    });
                }
                    });
            });

        if out_action.is_none()
            && let Some(pos) = backdrop_click_pos
            && !area_resp.response.rect.contains(pos)
        {
            out_action = Some(WorktreeCleanupAction::Close);
        }

        out_action
    }
}

fn render_cleanup_card(
    ui: &mut egui::Ui,
    item: &CleanupItem,
    selected: &mut HashSet<PathBuf>,
    confirming: &mut bool,
    removing: bool,
    is_en: bool,
) {
    let selectable = item.is_selectable() && !removing;
    let mut is_checked = item.is_selectable() && selected.contains(&item.path);

    let fg_strong = ui.visuals().strong_text_color();
    let fg_body = ui.visuals().text_color();
    let fg_muted = ui.visuals().weak_text_color();
    let warn = ui.visuals().warn_fg_color;
    let border = if is_checked {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke.color
    };

    egui::Frame::new()
        .fill(ui.visuals().panel_fill)
        .stroke(Stroke::new(1.0_f32, border))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let cb = ui.add_enabled(selectable, egui::Checkbox::without_text(&mut is_checked));
                if cb.changed() {
                    if is_checked {
                        selected.insert(item.path.clone());
                    } else {
                        selected.remove(&item.path);
                    }
                    *confirming = false;
                }

                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        let branch_label = if item.branch.is_empty() {
                            if is_en { "(detached)" } else { "(détaché)" }
                        } else {
                            item.branch.as_str()
                        };
                        ui.label(
                            RichText::new(branch_label)
                                .font(FontId::monospace(12.5))
                                .strong()
                                .color(fg_strong),
                        );
                        if item.ahead > 0 || item.behind > 0 {
                            ui.label(
                                RichText::new(format!("+{} / -{}", item.ahead, item.behind))
                                    .font(FontId::monospace(11.0))
                                    .color(fg_muted),
                            );
                        }
                        if item.is_safe() {
                            badge_pill(
                                ui,
                                if is_en {
                                    "Safe to remove"
                                } else {
                                    "Sans risque"
                                },
                                SAFE_GREEN,
                            );
                        } else if let Some(blocker) = item.blocker {
                            badge_pill(ui, blocker_label(blocker, is_en), fg_muted);
                        } else {
                            for risk in &item.risks {
                                badge_pill(ui, &risk_label(*risk, is_en), warn);
                            }
                        }
                    });

                    ui.label(
                        RichText::new(item.path.display().to_string())
                            .font(FontId::monospace(11.0))
                            .color(fg_muted),
                    );
                    if !item.last_subject.is_empty() {
                        ui.label(
                            RichText::new(&item.last_subject)
                                .font(FontId::proportional(11.5))
                                .color(fg_body),
                        );
                    }
                });
            });
        });
}

fn badge_pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.16))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.55)))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .font(FontId::proportional(10.5))
                    .color(color),
            );
        });
}

fn blocker_label(blocker: CleanupBlocker, is_en: bool) -> &'static str {
    match (blocker, is_en) {
        (CleanupBlocker::Detached, true) => "No branch (detached)",
        (CleanupBlocker::Detached, false) => "Sans branche (HEAD détaché)",
        (CleanupBlocker::DefaultBranch, true) => "On default branch",
        (CleanupBlocker::DefaultBranch, false) => "Sur la branche par défaut",
        (CleanupBlocker::Locked, true) => "Locked in git",
        (CleanupBlocker::Locked, false) => "Verrouillé dans Git",
    }
}

fn risk_label(risk: CleanupRisk, is_en: bool) -> String {
    match (risk, is_en) {
        (CleanupRisk::Dirty(n), true) => format!("{n} uncommitted changes"),
        (CleanupRisk::Dirty(n), false) => format!("{n} modifications non validées"),
        (CleanupRisk::Unmerged(n), true) => format!("{n} unmerged commits"),
        (CleanupRisk::Unmerged(n), false) => format!("{n} commits non fusionnés"),
        (CleanupRisk::OpenPanes(n), true) => format!("{n} open panes: closed first"),
        (CleanupRisk::OpenPanes(n), false) => format!("{n} panneaux ouverts : fermés d'abord"),
        (CleanupRisk::MissingFolder, true) => "Folder gone".to_owned(),
        (CleanupRisk::MissingFolder, false) => "Dossier absent".to_owned(),
    }
}
