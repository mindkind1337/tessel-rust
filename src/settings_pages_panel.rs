//! Native `egui` settings pages (`General`, `Terminal`, `Editor`, `Browser`, `Git`,
//! `QuickCommands`, `Stats`, and `Updates`) with full EN/FR parity and zero direct I/O.

use eframe::egui::{self, CornerRadius, FontId, Margin, RichText, Stroke};

use crate::{
    settings_model::{SettingsLang, SettingsPageId},
    theme,
    workspace::{
        BranchPrefixMode, BrowserPreferences, EditorPreferences, GeneralPreferences,
        GitPreferences, OpenLinksTarget, QuickCommand, ScmOrderSetting, ScmViewModeSetting,
        StatsPreferences, TerminalCursorStyle, TerminalPreferences, UsagePercentDisplay,
    },
};

/// Validates a single Git branch prefix segment (no leading/trailing `/`, spaces, or invalid
/// `git check-ref-format` characters).
pub fn validate_git_prefix_segment(raw: &str) -> Result<String, &'static str> {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Err("Enter a non-empty branch prefix.");
    }
    if trimmed.chars().count() > 60 {
        return Err("Branch prefix must be 60 characters or fewer.");
    }
    if trimmed.starts_with('-') || trimmed.starts_with('.') || trimmed.ends_with('.') {
        return Err("Branch prefix cannot start with '-' or '.' or end with '.'.");
    }
    if trimmed.contains("..") || trimmed.contains("//") || trimmed.contains("@{") {
        return Err("Branch prefix cannot contain '..', '//', or '@{'.");
    }
    for part in trimmed.split('/') {
        if part.is_empty()
            || part.starts_with('.')
            || part.ends_with('.')
            || part.ends_with(".lock")
        {
            return Err("Branch prefix contains an invalid path component.");
        }
    }
    if trimmed.chars().any(|c| {
        c.is_control()
            || c.is_whitespace()
            || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\' | '\x7f')
    }) {
        return Err("Branch prefix contains characters not allowed in Git branch names.");
    }
    Ok(trimmed.to_owned())
}

/// Sanitizes an arbitrary Git `user.name` into a safe lowercase ref prefix segment.
#[must_use]
pub fn sanitize_git_username_prefix(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut prev_sep = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_sep = false;
        } else if matches!(ch, '-' | '_' | ' ' | '.') && !out.is_empty() && !prev_sep {
            out.push('-');
            prev_sep = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    let cleaned = out.trim_matches('-').to_owned();
    if cleaned.is_empty() {
        None
    } else {
        validate_git_prefix_segment(&cleaned).ok()
    }
}

/// Resolves the effective Git branch prefix (`Some("agent")`, `Some("alice")`, or `None`).
#[must_use]
pub fn resolve_branch_prefix(
    mode: BranchPrefixMode,
    custom: &str,
    git_username: Option<&str>,
) -> Option<String> {
    match mode {
        BranchPrefixMode::None => None,
        BranchPrefixMode::Custom => validate_git_prefix_segment(custom)
            .ok()
            .or_else(|| Some("agent".to_owned())),
        BranchPrefixMode::GitUsername => git_username
            .and_then(sanitize_git_username_prefix)
            .or_else(|| Some("agent".to_owned())),
    }
}

/// Builds a live branch name preview (`<prefix>/<slug>` or `<slug>`).
#[must_use]
pub fn preview_branch_name(
    mode: BranchPrefixMode,
    custom: &str,
    git_username: Option<&str>,
    slug: &str,
) -> String {
    match resolve_branch_prefix(mode, custom, git_username) {
        Some(prefix) => format!("{prefix}/{slug}"),
        None => slug.to_owned(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsPageAction {
    ExportSettings,
    ImportSettings,
    OpenDataFolder,
    CopyDiagnostics,
    ResetAllSettings,
    SelectPreferredShell(Option<String>),
    SetFontSize(f32),
    OpenPasswordsVault,
    AddQuickCommand {
        name: String,
        text: String,
        press_enter: bool,
    },
    UpdateQuickCommand(QuickCommand),
    RemoveQuickCommand(u64),
    RunQuickCommand(u64),
    OpenGeneralPage,
    OpenExternalUrl(String),
    OpenNoticeFile(&'static str),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StatsSnapshot {
    pub project_count: usize,
    pub session_count: usize,
    pub running_agents: usize,
    pub sleeping_sessions: usize,
    pub open_documents: usize,
    pub tasks_todo: usize,
    pub tasks_doing: usize,
    pub tasks_review: usize,
    pub tasks_done: usize,
    pub app_rss_bytes: u64,
    pub panes_rss_bytes: u64,
    pub panes_cpu_pct: f32,
    pub listening_ports: usize,
}

#[derive(Clone, Debug, Default)]
pub struct SettingsPagesState {
    confirm_reset_all: bool,
    diagnostics_copied: bool,
    version_copied: bool,
    expanded_quick_cmd: Option<u64>,
    edit_draft: Option<(u64, String, String, bool, String)>,
    confirm_remove_cmd: Option<u64>,
    new_cmd_name: String,
    new_cmd_text: String,
    new_cmd_press_enter: bool,
    new_cmd_initialized: bool,
    new_cmd_error: String,
}

impl SettingsPagesState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            new_cmd_press_enter: true,
            new_cmd_initialized: true,
            ..Default::default()
        }
    }

    pub fn reset_transient(&mut self) {
        self.confirm_reset_all = false;
        self.diagnostics_copied = false;
        self.version_copied = false;
        self.confirm_remove_cmd = None;
        self.new_cmd_error.clear();
    }

    fn ensure_init(&mut self) {
        if !self.new_cmd_initialized {
            self.new_cmd_press_enter = true;
            self.new_cmd_initialized = true;
        }
    }
}

fn lang_of(french: bool) -> SettingsLang {
    if french {
        SettingsLang::Fr
    } else {
        SettingsLang::En
    }
}

fn paint_page_header(ui: &mut egui::Ui, page: SettingsPageId, french: bool) {
    let lang = lang_of(french);
    ui.label(
        RichText::new(page.title(lang))
            .font(FontId::proportional(19.0))
            .strong(),
    );
    ui.add_space(3.0);
    ui.label(
        RichText::new(page.description(lang))
            .font(FontId::proportional(12.5))
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(14.0);
}

fn section_heading(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(title)
            .font(FontId::proportional(13.0))
            .strong(),
    );
    ui.add_space(6.0);
}

fn settings_card<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let spec = theme::active(ui.ctx()).spec();
    let (fill, border) = if ui.visuals().dark_mode {
        (spec.ui.surface, spec.ui.border)
    } else {
        (
            ui.visuals().widgets.noninteractive.bg_fill,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        )
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width().min(680.0));
            ui.set_max_width(680.0);
            add_contents(ui)
        })
        .inner
}

fn toggle_row(ui: &mut egui::Ui, value: &mut bool, label: &str, hint: &str) -> bool {
    let changed = ui
        .checkbox(value, RichText::new(label).font(FontId::proportional(13.0)))
        .changed();
    if !hint.is_empty() {
        ui.add_space(1.0);
        ui.label(
            RichText::new(hint)
                .font(FontId::proportional(11.5))
                .color(ui.visuals().weak_text_color()),
        );
    }
    changed
}

fn format_bytes_human(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.0} MiB", b / MIB)
    } else if bytes > 0 {
        format!("{} KiB", (bytes / 1024).max(1))
    } else {
        "0 MiB".to_owned()
    }
}

impl SettingsPagesState {
    pub fn show_general(
        &mut self,
        ui: &mut egui::Ui,
        prefs: &mut GeneralPreferences,
        backup_status: &str,
        has_store: bool,
        french: bool,
    ) -> (bool, Option<SettingsPageAction>) {
        let mut changed = false;
        let mut action = None;
        paint_page_header(ui, SettingsPageId::General, french);

        section_heading(
            ui,
            if french {
                "Démarrage et fermeture"
            } else {
                "Startup & closing"
            },
        );
        settings_card(ui, |ui| {
            changed |= toggle_row(
                ui,
                &mut prefs.restore_workspaces,
                if french {
                    "Restaurer les espaces de travail et panneaux au lancement"
                } else {
                    "Restore workspaces and panes on launch"
                },
                if french {
                    "Les panneaux restaurés restent en veille jusqu'à ce que vous cliquiez dessus."
                } else {
                    "Restored panes stay asleep until you click to start them."
                },
            );
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            changed |= toggle_row(
                ui,
                &mut prefs.confirm_close_agent,
                if french {
                    "Confirmer avant de fermer un agent actif"
                } else {
                    "Ask before closing a running agent pane"
                },
                if french {
                    "Évite d'interrompre une session d'agent en cours par inadvertance."
                } else {
                    "Prevents accidentally stopping an active agent conversation or CLI."
                },
            );
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            changed |= toggle_row(
                ui,
                &mut prefs.confirm_quit_running,
                if french {
                    "Confirmer avant de quitter quand des sessions tournent"
                } else {
                    "Ask before quitting while sessions are open"
                },
                if french {
                    "Les fichiers modifiés non enregistrés demandent toujours confirmation."
                } else {
                    "Unsaved file edits always ask for confirmation before closing."
                },
            );
        });

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Espaces de travail & worktrees"
            } else {
                "Workspaces & worktrees"
            },
        );
        settings_card(ui, |ui| {
            ui.label(
                RichText::new(if french {
                    "Dossier des copies de travail Git (worktrees)"
                } else {
                    "Git worktrees directory override"
                })
                .font(FontId::proportional(13.0)),
            );
            ui.label(
                RichText::new(if french {
                    "Laissez vide pour créer <projet>.worktrees à côté du dépôt."
                } else {
                    "Leave empty to create <project>.worktrees next to each repository."
                })
                .font(FontId::proportional(11.5))
                .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let mut draft = prefs.workspace_dir.clone();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut draft)
                        .char_limit(1024)
                        .desired_width((ui.available_width() - 90.0).max(140.0))
                        .hint_text("<project>.worktrees"),
                );
                if resp.changed() && !draft.chars().any(char::is_control) {
                    prefs.workspace_dir = draft;
                    changed = true;
                }
                if ui
                    .add_enabled(
                        !prefs.workspace_dir.is_empty(),
                        egui::Button::new(if french { "Réinitialiser" } else { "Reset" }),
                    )
                    .clicked()
                {
                    prefs.workspace_dir.clear();
                    changed = true;
                }
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            changed |= toggle_row(
                ui,
                &mut prefs.confirm_delete_workspace,
                if french {
                    "Confirmer avant de retirer un projet de la barre latérale"
                } else {
                    "Confirm before removing a workspace from the sidebar"
                },
                if french {
                    "Retirer un projet ferme ses panneaux mais ne supprime aucun fichier sur le disque."
                } else {
                    "Removing a workspace closes its panes without deleting files on disk."
                },
            );
        });

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Sauvegarde des paramètres"
            } else {
                "Settings backup"
            },
        );
        settings_card(ui, |ui| {
            ui.label(
                RichText::new(if french {
                    "Exporte les chemins de projets, tâches, hôtes SSH et préférences. Aucun mot de passe, jeton, commande shell ou profil navigateur n'est inclus."
                } else {
                    "Exports project paths, tasks, SSH aliases, and preferences. Passwords, tokens, shell commands, and browser profiles are never exported or imported."
                })
                .font(FontId::proportional(11.5))
                .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(if french {
                        "Exporter les paramètres…"
                    } else {
                        "Export settings…"
                    })
                    .clicked()
                {
                    action = Some(SettingsPageAction::ExportSettings);
                }
                if ui
                    .button(if french {
                        "Importer des paramètres…"
                    } else {
                        "Import settings…"
                    })
                    .clicked()
                {
                    action = Some(SettingsPageAction::ImportSettings);
                }
            });
            if !backup_status.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(backup_status).font(FontId::proportional(12.0)));
            }
        });

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Dépannage"
            } else {
                "Troubleshooting"
            },
        );
        settings_card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        has_store,
                        egui::Button::new(if french {
                            "Ouvrir le dossier de données"
                        } else {
                            "Open data folder"
                        }),
                    )
                    .clicked()
                {
                    action = Some(SettingsPageAction::OpenDataFolder);
                }
                if ui
                    .button(if french {
                        "Copier le diagnostic"
                    } else {
                        "Copy diagnostics"
                    })
                    .clicked()
                {
                    self.diagnostics_copied = true;
                    action = Some(SettingsPageAction::CopyDiagnostics);
                }
                if !self.confirm_reset_all {
                    if ui
                        .button(if french {
                            "Réinitialiser les préférences…"
                        } else {
                            "Reset preferences…"
                        })
                        .clicked()
                    {
                        self.confirm_reset_all = true;
                    }
                } else {
                    ui.label(
                        RichText::new(if french {
                            "Réinitialiser toutes les préférences ?"
                        } else {
                            "Reset all preferences to defaults?"
                        })
                        .color(ui.visuals().warn_fg_color),
                    );
                    if ui
                        .button(if french {
                            "Confirmer la réinitialisation"
                        } else {
                            "Confirm reset"
                        })
                        .clicked()
                    {
                        self.confirm_reset_all = false;
                        action = Some(SettingsPageAction::ResetAllSettings);
                    }
                    if ui
                        .button(if french { "Annuler" } else { "Cancel" })
                        .clicked()
                    {
                        self.confirm_reset_all = false;
                    }
                }
            });
            if self.diagnostics_copied {
                ui.add_space(4.0);
                ui.small(if french {
                    "Diagnostic copié dans le presse-papiers."
                } else {
                    "Diagnostics copied to clipboard."
                });
            }
        });

        (changed, action)
    }

    pub fn show_terminal(
        &mut self,
        ui: &mut egui::Ui,
        prefs: &mut TerminalPreferences,
        preferred_shell: Option<&str>,
        font_size: f32,
        wsl_available: bool,
        french: bool,
    ) -> (bool, Option<SettingsPageAction>) {
        let mut changed = false;
        let mut action = None;
        paint_page_header(ui, SettingsPageId::Terminal, french);

        section_heading(ui, "Shell");
        settings_card(ui, |ui| {
            ui.label(
                RichText::new(if french {
                    "Shell par défaut pour les nouveaux panneaux"
                } else {
                    "Default shell for new terminal panes"
                })
                .font(FontId::proportional(13.0)),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if cfg!(windows) {
                    let current = preferred_shell.unwrap_or("powershell");
                    for (id, label, enabled) in [
                        ("powershell", "PowerShell", true),
                        ("cmd", "CMD", true),
                        ("wsl", "WSL", wsl_available),
                    ] {
                        if ui
                            .add_enabled(enabled, egui::Button::selectable(current == id, label))
                            .clicked()
                            && current != id
                        {
                            action = Some(SettingsPageAction::SelectPreferredShell(Some(
                                id.to_owned(),
                            )));
                        }
                    }
                } else {
                    let _ = wsl_available;
                    let _ = ui.selectable_label(true, preferred_shell.unwrap_or("sh"));
                }
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if french {
                        "Historique de défilement (lignes)"
                    } else {
                        "Scrollback history (lines)"
                    })
                    .font(FontId::proportional(13.0)),
                );
                let mut sb = prefs.scrollback.clamp(500, 100_000);
                if ui
                    .add(
                        egui::DragValue::new(&mut sb)
                            .range(500..=100_000)
                            .speed(100.0),
                    )
                    .changed()
                {
                    prefs.scrollback = sb.clamp(500, 100_000);
                    changed = true;
                }
            });
            ui.small(if french {
                "S'applique aux prochains terminaux ouverts (500 à 100 000 lignes)."
            } else {
                "Applies to newly opened terminals (500 to 100,000 lines)."
            });
        });

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Police et curseur"
            } else {
                "Font & cursor"
            },
        );
        settings_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if french {
                        "Taille de police (px)"
                    } else {
                        "Font size (px)"
                    })
                    .font(FontId::proportional(13.0)),
                );
                let mut sz = font_size.clamp(10.0, 28.0);
                if ui
                    .add(egui::Slider::new(&mut sz, 10.0_f32..=28.0_f32).step_by(0.5))
                    .changed()
                {
                    action = Some(SettingsPageAction::SetFontSize(sz.clamp(10.0, 28.0)));
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if french {
                        "Hauteur de ligne"
                    } else {
                        "Line height"
                    })
                    .font(FontId::proportional(13.0)),
                );
                let mut lh = prefs.line_height.clamp(1.0, 3.0);
                if ui
                    .add(egui::Slider::new(&mut lh, 1.0_f32..=2.0_f32).step_by(0.05))
                    .changed()
                {
                    prefs.line_height = (lh * 20.0).round() / 20.0;
                    changed = true;
                }
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(if french {
                        "Style du curseur"
                    } else {
                        "Cursor style"
                    })
                    .font(FontId::proportional(13.0)),
                );
                for (style, en, fr) in [
                    (TerminalCursorStyle::Block, "Block", "Bloc"),
                    (TerminalCursorStyle::Bar, "Bar", "Barre"),
                    (TerminalCursorStyle::Underline, "Underline", "Souligné"),
                ] {
                    if ui
                        .selectable_label(prefs.cursor_style == style, if french { fr } else { en })
                        .clicked()
                        && prefs.cursor_style != style
                    {
                        prefs.cursor_style = style;
                        changed = true;
                    }
                }
            });
            ui.add_space(6.0);
            changed |= toggle_row(
                ui,
                &mut prefs.cursor_blink,
                if french {
                    "Curseur clignotant"
                } else {
                    "Blinking cursor"
                },
                "",
            );
        });

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Disposition, presse-papiers et souris"
            } else {
                "Layout, clipboard & mouse"
            },
        );
        settings_card(ui, |ui| {
            changed |= toggle_row(
                ui,
                &mut prefs.tile_new_panes,
                if french {
                    "Disposer automatiquement les nouveaux panneaux en mosaïque"
                } else {
                    "Automatically tile newly opened panes"
                },
                if french {
                    "Rééquilibre la grille lorsque vous ajoutez un panneau sans division explicite."
                } else {
                    "Rebalances the workspace grid when opening a pane without an explicit split."
                },
            );
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            changed |= toggle_row(
                ui,
                &mut prefs.copy_on_select,
                if french {
                    "Copier automatiquement la sélection"
                } else {
                    "Copy selection to clipboard automatically"
                },
                "",
            );
            ui.add_space(6.0);
            changed |= toggle_row(
                ui,
                &mut prefs.right_click_paste,
                if french {
                    "Clic droit pour copier/coller immédiatement"
                } else {
                    "Right-click to copy selection or paste immediately"
                },
                if french {
                    "Si désactivé, le clic droit ouvre le menu contextuel du terminal."
                } else {
                    "When off, right-clicking opens the terminal context menu instead."
                },
            );
            ui.add_space(6.0);
            changed |= toggle_row(
                ui,
                &mut prefs.confirm_multiline_paste,
                if french {
                    "Confirmer avant de coller plusieurs lignes"
                } else {
                    "Warn when pasting multiple lines"
                },
                "",
            );
            ui.add_space(6.0);
            changed |= toggle_row(
                ui,
                &mut prefs.focus_follows_mouse,
                if french {
                    "Le focus suit la souris"
                } else {
                    "Focus terminal pane on mouse hover"
                },
                "",
            );
        });

        (changed, action)
    }

    pub fn show_editor(
        &mut self,
        ui: &mut egui::Ui,
        prefs: &mut EditorPreferences,
        french: bool,
    ) -> bool {
        let mut changed = false;
        paint_page_header(ui, SettingsPageId::Editor, french);

        section_heading(
            ui,
            if french {
                "Édition du code"
            } else {
                "Code editing"
            },
        );
        settings_card(ui, |ui| {
            changed |= toggle_row(
                ui,
                &mut prefs.auto_save,
                if french {
                    "Enregistrement automatique"
                } else {
                    "Auto-save modified files"
                },
                if french {
                    "Enregistre les fichiers modifiés après un court délai d'inactivité."
                } else {
                    "Automatically writes modified UTF-8 files to disk after an idle delay."
                },
            );
            ui.horizontal(|ui| {
                ui.label(if french {
                    "Taille de police de l’éditeur"
                } else {
                    "Editor font size"
                });
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut prefs.font_size)
                            .range(6..=100)
                            .suffix(" px"),
                    )
                    .changed();
            });
            if prefs.auto_save {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if french { "Délai (ms)" } else { "Delay (ms)" })
                            .font(FontId::proportional(12.5)),
                    );
                    let mut delay = prefs.auto_save_delay_ms.clamp(250, 10_000);
                    if ui
                        .add(
                            egui::DragValue::new(&mut delay)
                                .range(250..=10_000)
                                .speed(50.0),
                        )
                        .changed()
                    {
                        prefs.auto_save_delay_ms = delay.clamp(250, 10_000);
                        changed = true;
                    }
                });
            }
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            changed |= toggle_row(
                ui,
                &mut prefs.syntax_highlighting,
                if french {
                    "Coloration syntaxique"
                } else {
                    "Syntax highlighting"
                },
                if french {
                    "Coloration lexicale adaptée au thème actif (jusqu'à 512 Kio par fichier)."
                } else {
                    "Theme-adaptive lexical highlighting (up to 512 KiB per file)."
                },
            );
            ui.add_space(6.0);
            changed |= toggle_row(
                ui,
                &mut prefs.line_numbers,
                if french {
                    "Numéros de ligne et gouttière de modifications"
                } else {
                    "Line numbers and modified-line gutter"
                },
                "",
            );
            ui.add_space(6.0);
            changed |= toggle_row(
                ui,
                &mut prefs.word_wrap,
                if french {
                    "Retour à la ligne par défaut"
                } else {
                    "Word wrap by default"
                },
                "",
            );
            ui.add_space(6.0);
            changed |= toggle_row(
                ui,
                &mut prefs.preview_tabs,
                if french {
                    "Onglets d'aperçu réutilisables (clic simple)"
                } else {
                    "Enable reusable preview tabs on single click"
                },
                if french {
                    "Double-cliquez ou modifiez un fichier pour épingler son onglet."
                } else {
                    "Double-click or edit a file to keep its tab open permanently."
                },
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if french {
                        "Taille d'indentation (espaces)"
                    } else {
                        "Tab size (spaces)"
                    })
                    .font(FontId::proportional(13.0)),
                );
                for sz in [2_u8, 4, 8] {
                    if ui
                        .selectable_label(prefs.tab_size == sz, sz.to_string())
                        .clicked()
                        && prefs.tab_size != sz
                    {
                        prefs.tab_size = sz;
                        changed = true;
                    }
                }
            });
        });

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Panneau Fichiers et Diffs"
            } else {
                "Files panel & diffs"
            },
        );
        settings_card(ui, |ui| {
            changed |= toggle_row(
                ui,
                &mut prefs.show_git_ignored,
                if french {
                    "Afficher les fichiers cachés (.dotfiles)"
                } else {
                    "Show dotfiles and hidden entries in Explorer"
                },
                "",
            );
            ui.add_space(6.0);
            changed |= toggle_row(
                ui,
                &mut prefs.compact_folders,
                if french {
                    "Compacter les dossiers à enfant unique (ex. src/bin)"
                } else {
                    "Compact single-child folders (e.g. src/bin)"
                },
                "",
            );
            ui.add_space(6.0);
            changed |= toggle_row(
                ui,
                &mut prefs.diff_word_wrap,
                if french {
                    "Retour à la ligne dans les vues de diff"
                } else {
                    "Wrap long lines in Git diff views"
                },
                "",
            );
        });

        changed
    }

    pub fn show_browser(
        &mut self,
        ui: &mut egui::Ui,
        prefs: &mut BrowserPreferences,
        has_vault: bool,
        french: bool,
    ) -> (bool, Option<SettingsPageAction>) {
        let mut changed = false;
        let mut action = None;
        paint_page_header(ui, SettingsPageId::Browser, french);

        section_heading(ui, if french { "Liens" } else { "Links" });
        settings_card(ui, |ui| {
            ui.label(
                RichText::new(if french {
                    "Ouvrir les liens externes dans"
                } else {
                    "Open HTTP/HTTPS links in"
                })
                .font(FontId::proportional(13.0)),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (target, en, fr) in [
                    (
                        OpenLinksTarget::Builtin,
                        "Built-in browser",
                        "Navigateur intégré",
                    ),
                    (
                        OpenLinksTarget::System,
                        "System browser",
                        "Navigateur du système",
                    ),
                ] {
                    if ui
                        .selectable_label(
                            prefs.open_links_in == target,
                            if french { fr } else { en },
                        )
                        .clicked()
                        && prefs.open_links_in != target
                    {
                        prefs.open_links_in = target;
                        changed = true;
                    }
                }
            });
        });

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Connexions et isolation des agents"
            } else {
                "Sign-ins & agent session isolation"
            },
        );
        settings_card(ui, |ui| {
            changed |= toggle_row(
                ui,
                &mut prefs.separate_agent_session,
                if french {
                    "Session navigateur séparée et isolée par projet pour les agents"
                } else {
                    "Isolate agent browser sessions from your personal browser profile"
                },
                if french {
                    "Chaque projet utilise un profil WebView2 distinct des cookies personnels du propriétaire."
                } else {
                    "Agents browse inside an isolated per-project WebView2 profile separate from your owner sign-ins."
                },
            );
            if has_vault {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if french {
                            "Coffre-fort de mots de passe chiffré (DPAPI)"
                        } else {
                            "Encrypted password vault (Windows DPAPI)"
                        })
                        .font(FontId::proportional(13.0)),
                    );
                    if ui
                        .button(if french {
                            "Ouvrir Mots de passe…"
                        } else {
                            "Open Passwords…"
                        })
                        .clicked()
                    {
                        action = Some(SettingsPageAction::OpenPasswordsVault);
                    }
                });
            }
        });

        (changed, action)
    }

    pub fn show_git(
        &mut self,
        ui: &mut egui::Ui,
        prefs: &mut GitPreferences,
        git_username: Option<&str>,
        french: bool,
    ) -> bool {
        let mut changed = false;
        paint_page_header(ui, SettingsPageId::Git, french);

        section_heading(
            ui,
            if french {
                "Nommage des branches de worktree"
            } else {
                "Worktree branch naming"
            },
        );
        settings_card(ui, |ui| {
            ui.label(
                RichText::new(if french {
                    "Préfixe des nouvelles branches créées pour les copies de travail"
                } else {
                    "Prefix for new branches created in Git worktrees"
                })
                .font(FontId::proportional(13.0)),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (mode, en, fr) in [
                    (
                        BranchPrefixMode::GitUsername,
                        "Git username",
                        "Nom d'utilisateur Git",
                    ),
                    (
                        BranchPrefixMode::Custom,
                        "Custom prefix",
                        "Préfixe personnalisé",
                    ),
                    (BranchPrefixMode::None, "No prefix", "Aucun préfixe"),
                ] {
                    if ui
                        .selectable_label(
                            prefs.branch_prefix_mode == mode,
                            if french { fr } else { en },
                        )
                        .clicked()
                        && prefs.branch_prefix_mode != mode
                    {
                        prefs.branch_prefix_mode = mode;
                        changed = true;
                    }
                }
            });

            if prefs.branch_prefix_mode == BranchPrefixMode::Custom {
                ui.add_space(6.0);
                let mut draft = prefs.branch_prefix_custom.clone();
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut draft)
                        .char_limit(60)
                        .desired_width(240.0)
                        .hint_text("agent"),
                );
                if edit.changed() && !draft.chars().any(char::is_control) {
                    prefs.branch_prefix_custom = draft;
                    changed = true;
                }
                if let Err(err) = validate_git_prefix_segment(&prefs.branch_prefix_custom) {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        if french {
                            "Préfixe invalide — « agent » sera utilisé par défaut."
                        } else {
                            err
                        },
                    );
                }
            }

            ui.add_space(6.0);
            let sample = preview_branch_name(
                prefs.branch_prefix_mode,
                &prefs.branch_prefix_custom,
                git_username,
                "my-feature",
            );
            ui.horizontal(|ui| {
                ui.small(if french { "Aperçu :" } else { "Preview:" });
                ui.code(sample);
            });
        });

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Gestion de code source (Modifications)"
            } else {
                "Source Control panel"
            },
        );
        settings_card(ui, |ui| {
            ui.label(
                RichText::new(if french {
                    "Ordre des sections"
                } else {
                    "Section order"
                })
                .font(FontId::proportional(13.0)),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (order, en, fr) in [
                    (
                        ScmOrderSetting::ChangesFirst,
                        "Changes first",
                        "Modifications d'abord",
                    ),
                    (
                        ScmOrderSetting::StagedFirst,
                        "Staged first",
                        "Indexées d'abord",
                    ),
                    (
                        ScmOrderSetting::UntrackedFirst,
                        "Untracked first",
                        "Non suivis d'abord",
                    ),
                ] {
                    if ui
                        .selectable_label(
                            prefs.scm_group_order == order,
                            if french { fr } else { en },
                        )
                        .clicked()
                        && prefs.scm_group_order != order
                    {
                        prefs.scm_group_order = order;
                        changed = true;
                    }
                }
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(
                RichText::new(if french {
                    "Mode d'affichage des fichiers"
                } else {
                    "Default file presentation"
                })
                .font(FontId::proportional(13.0)),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (mode, en, fr) in [
                    (ScmViewModeSetting::Tree, "Folder tree", "Arborescence"),
                    (ScmViewModeSetting::List, "Flat list", "Liste plate"),
                ] {
                    if ui
                        .selectable_label(prefs.scm_view_mode == mode, if french { fr } else { en })
                        .clicked()
                        && prefs.scm_view_mode != mode
                    {
                        prefs.scm_view_mode = mode;
                        changed = true;
                    }
                }
            });
        });

        changed
    }

    pub fn show_quick_commands(
        &mut self,
        ui: &mut egui::Ui,
        commands: &[QuickCommand],
        can_run_in_terminal: bool,
        french: bool,
    ) -> Option<SettingsPageAction> {
        self.ensure_init();
        let mut action = None;
        paint_page_header(ui, SettingsPageId::QuickCommands, french);

        section_heading(
            ui,
            if french {
                "Commandes enregistrées"
            } else {
                "Saved commands"
            },
        );
        if commands.is_empty() {
            settings_card(ui, |ui| {
                ui.weak(if french {
                    "Aucune commande rapide enregistrée. Ajoutez vos commandes fréquentes ci-dessous pour les lancer depuis ce panneau ou la palette de commandes (Ctrl+Maj+P)."
                } else {
                    "No quick commands saved yet. Add frequent commands below to run them from here or from the Command Palette (Ctrl+Shift+P)."
                });
            });
        } else {
            for cmd in commands {
                ui.push_id(("quick-cmd", cmd.id), |ui| {
                    settings_card(ui, |ui| {
                        let is_editing = self
                            .edit_draft
                            .as_ref()
                            .is_some_and(|(id, ..)| *id == cmd.id);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(&cmd.name)
                                    .font(FontId::proportional(13.5))
                                    .strong(),
                            );
                            let mode_badge = match (cmd.press_enter, french) {
                                (true, false) => "Runs right away",
                                (true, true) => "Lancée tout de suite",
                                (false, false) => "Placed in the terminal",
                                (false, true) => "Placée dans le terminal",
                            };
                            ui.small(format!("· {mode_badge}"));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if self.confirm_remove_cmd == Some(cmd.id) {
                                        if ui
                                            .button(if french { "Annuler" } else { "Cancel" })
                                            .clicked()
                                        {
                                            self.confirm_remove_cmd = None;
                                        }
                                        if ui
                                            .button(
                                                RichText::new(if french {
                                                    "Confirmer"
                                                } else {
                                                    "Confirm remove"
                                                })
                                                .color(ui.visuals().error_fg_color),
                                            )
                                            .clicked()
                                        {
                                            self.confirm_remove_cmd = None;
                                            if self.expanded_quick_cmd == Some(cmd.id) {
                                                self.expanded_quick_cmd = None;
                                            }
                                            action = Some(SettingsPageAction::RemoveQuickCommand(
                                                cmd.id,
                                            ));
                                        }
                                    } else {
                                        if ui
                                            .button(if french { "Supprimer" } else { "Remove" })
                                            .clicked()
                                        {
                                            self.confirm_remove_cmd = Some(cmd.id);
                                        }
                                        if ui
                                            .button(if is_editing {
                                                if french { "Fermer" } else { "Close" }
                                            } else if french {
                                                "Modifier"
                                            } else {
                                                "Edit"
                                            })
                                            .clicked()
                                        {
                                            if is_editing {
                                                self.edit_draft = None;
                                                self.expanded_quick_cmd = None;
                                            } else {
                                                self.expanded_quick_cmd = Some(cmd.id);
                                                self.edit_draft = Some((
                                                    cmd.id,
                                                    cmd.name.clone(),
                                                    cmd.text.clone(),
                                                    cmd.press_enter,
                                                    String::new(),
                                                ));
                                            }
                                        }
                                        if ui
                                            .add_enabled(
                                                can_run_in_terminal,
                                                egui::Button::new(if french {
                                                    "Exécuter"
                                                } else {
                                                    "Run"
                                                }),
                                            )
                                            .on_hover_text(if french {
                                                "Envoyer au terminal actif"
                                            } else {
                                                "Send to active terminal"
                                            })
                                            .clicked()
                                        {
                                            action =
                                                Some(SettingsPageAction::RunQuickCommand(cmd.id));
                                        }
                                    }
                                },
                            );
                        });
                        if !is_editing {
                            ui.add_space(3.0);
                            let preview: String = cmd.text.chars().take(120).collect();
                            ui.code(preview);
                        } else if let Some((_, name, text, press_enter, err)) = &mut self.edit_draft
                        {
                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);
                            ui.label(if french { "Nom" } else { "Name" });
                            ui.add(
                                egui::TextEdit::singleline(name)
                                    .char_limit(120)
                                    .desired_width(ui.available_width()),
                            );
                            ui.add_space(4.0);
                            ui.label(if french { "Commande" } else { "Command" });
                            ui.add(
                                egui::TextEdit::multiline(text)
                                    .code_editor()
                                    .desired_rows(3)
                                    .desired_width(ui.available_width()),
                            );
                            ui.add_space(4.0);
                            ui.checkbox(
                                press_enter,
                                if french {
                                    "Appuyer sur Entrée automatiquement (exécuter tout de suite)"
                                } else {
                                    "Press Enter automatically (run right away)"
                                },
                            );
                            if !err.is_empty() {
                                ui.colored_label(ui.visuals().error_fg_color, err.as_str());
                            }
                            ui.add_space(4.0);
                            let mut close_edit = false;
                            let mut saved_cmd = None;
                            ui.horizontal(|ui| {
                                if ui
                                    .button(if french { "Enregistrer" } else { "Save" })
                                    .clicked()
                                {
                                    let clean_name = name.trim().to_owned();
                                    if clean_name.is_empty()
                                        || clean_name.chars().any(char::is_control)
                                    {
                                        *err = if french {
                                            "Saisissez un nom valide.".to_owned()
                                        } else {
                                            "Enter a valid command name.".to_owned()
                                        };
                                    } else if text.trim().is_empty()
                                        || text.chars().count() > 20_000
                                        || text.contains('\0')
                                    {
                                        *err = if french {
                                            "Saisissez un texte de commande non vide.".to_owned()
                                        } else {
                                            "Enter non-empty command text.".to_owned()
                                        };
                                    } else {
                                        saved_cmd = Some(QuickCommand {
                                            id: cmd.id,
                                            name: clean_name,
                                            text: text.clone(),
                                            press_enter: *press_enter,
                                        });
                                        close_edit = true;
                                    }
                                }
                                if ui
                                    .button(if french { "Annuler" } else { "Cancel" })
                                    .clicked()
                                {
                                    close_edit = true;
                                }
                            });
                            if let Some(updated) = saved_cmd {
                                action = Some(SettingsPageAction::UpdateQuickCommand(updated));
                            }
                            if close_edit {
                                self.edit_draft = None;
                                self.expanded_quick_cmd = None;
                            }
                        }
                    });
                });
                ui.add_space(6.0);
            }
        }

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Nouvelle commande"
            } else {
                "New command"
            },
        );
        settings_card(ui, |ui| {
            ui.label(if french { "Nom" } else { "Name" });
            ui.add(
                egui::TextEdit::singleline(&mut self.new_cmd_name)
                    .char_limit(120)
                    .desired_width(ui.available_width())
                    .hint_text(if french {
                        "Ex. Lancer les tests"
                    } else {
                        "e.g. Run test suite"
                    }),
            );
            ui.add_space(6.0);
            ui.label(if french { "Commande" } else { "Command" });
            ui.add(
                egui::TextEdit::multiline(&mut self.new_cmd_text)
                    .code_editor()
                    .desired_rows(3)
                    .desired_width(ui.available_width())
                    .hint_text("cargo test"),
            );
            ui.add_space(6.0);
            ui.checkbox(
                &mut self.new_cmd_press_enter,
                if french {
                    "Appuyer sur Entrée automatiquement (exécuter tout de suite)"
                } else {
                    "Press Enter automatically (run right away)"
                },
            );
            if !self.new_cmd_error.is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, &self.new_cmd_error);
            }
            ui.add_space(6.0);
            let at_cap = commands.len() >= 200;
            if ui
                .add_enabled(
                    !at_cap,
                    egui::Button::new(if french {
                        "Ajouter la commande"
                    } else {
                        "Add command"
                    }),
                )
                .clicked()
            {
                let clean_name = self.new_cmd_name.trim().to_owned();
                if clean_name.is_empty() || clean_name.chars().any(char::is_control) {
                    self.new_cmd_error = if french {
                        "Saisissez un nom valide.".to_owned()
                    } else {
                        "Enter a valid command name.".to_owned()
                    };
                } else if self.new_cmd_text.trim().is_empty()
                    || self.new_cmd_text.chars().count() > 20_000
                    || self.new_cmd_text.contains('\0')
                {
                    self.new_cmd_error = if french {
                        "Saisissez un texte de commande non vide.".to_owned()
                    } else {
                        "Enter non-empty command text.".to_owned()
                    };
                } else {
                    let text = std::mem::take(&mut self.new_cmd_text);
                    let press_enter = self.new_cmd_press_enter;
                    self.new_cmd_name.clear();
                    self.new_cmd_press_enter = true;
                    self.new_cmd_error.clear();
                    action = Some(SettingsPageAction::AddQuickCommand {
                        name: clean_name,
                        text,
                        press_enter,
                    });
                }
            }
        });

        action
    }

    pub fn show_stats(
        &mut self,
        ui: &mut egui::Ui,
        prefs: &mut StatsPreferences,
        snapshot: &StatsSnapshot,
        french: bool,
    ) -> bool {
        let mut changed = false;
        paint_page_header(ui, SettingsPageId::Stats, french);

        section_heading(
            ui,
            if french {
                "Affichage de l'utilisation"
            } else {
                "Usage display"
            },
        );
        settings_card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(if french {
                        "Afficher les quotas en"
                    } else {
                        "Display quota percentages as"
                    })
                    .font(FontId::proportional(13.0)),
                );
                for (mode, en, fr) in [
                    (UsagePercentDisplay::Used, "% used", "% utilisé"),
                    (UsagePercentDisplay::Remaining, "% remaining", "% restant"),
                ] {
                    if ui
                        .selectable_label(
                            prefs.usage_percentage_display == mode,
                            if french { fr } else { en },
                        )
                        .clicked()
                        && prefs.usage_percentage_display != mode
                    {
                        prefs.usage_percentage_display = mode;
                        changed = true;
                    }
                }
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(if french {
                        "Actualisation automatique"
                    } else {
                        "Automatic refresh interval"
                    })
                    .font(FontId::proportional(13.0)),
                );
                for mins in [0_u16, 2, 5, 15, 30, 60] {
                    let label = if mins == 0 {
                        if french {
                            "Manuel".to_owned()
                        } else {
                            "Manual".to_owned()
                        }
                    } else {
                        format!("{mins} min")
                    };
                    if ui
                        .selectable_label(prefs.usage_refresh_minutes == mins, label)
                        .clicked()
                        && prefs.usage_refresh_minutes != mins
                    {
                        prefs.usage_refresh_minutes = mins;
                        changed = true;
                    }
                }
            });
        });

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Activité locale de Tessel"
            } else {
                "Local Tessel activity"
            },
        );
        settings_card(ui, |ui| {
            egui::Grid::new("stats-activity-grid")
                .num_columns(2)
                .spacing([28.0, 6.0])
                .show(ui, |ui| {
                    ui.weak(if french { "Projets" } else { "Workspaces" });
                    ui.strong(snapshot.project_count.to_string());
                    ui.end_row();

                    ui.weak(if french {
                        "Panneaux ouverts"
                    } else {
                        "Open panes"
                    });
                    ui.strong(format!(
                        "{} ({} {}, {} {})",
                        snapshot.session_count,
                        snapshot.running_agents,
                        if french {
                            "agents actifs"
                        } else {
                            "running agents"
                        },
                        snapshot.sleeping_sessions,
                        if french { "en veille" } else { "sleeping" }
                    ));
                    ui.end_row();

                    ui.weak(if french {
                        "Fichiers ouverts"
                    } else {
                        "Open editors"
                    });
                    ui.strong(snapshot.open_documents.to_string());
                    ui.end_row();

                    ui.weak(if french {
                        "Tableau des tâches"
                    } else {
                        "Task board"
                    });
                    let total_tasks = snapshot.tasks_todo
                        + snapshot.tasks_doing
                        + snapshot.tasks_review
                        + snapshot.tasks_done;
                    ui.strong(format!(
                        "{total_tasks} ({}: {}, {}: {}, {}: {}, {}: {})",
                        if french { "À faire" } else { "Todo" },
                        snapshot.tasks_todo,
                        if french { "En cours" } else { "Doing" },
                        snapshot.tasks_doing,
                        if french { "Revue" } else { "Review" },
                        snapshot.tasks_review,
                        if french { "Terminé" } else { "Done" },
                        snapshot.tasks_done,
                    ));
                    ui.end_row();
                });
        });

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Ressources système en direct"
            } else {
                "Live system resources"
            },
        );
        settings_card(ui, |ui| {
            egui::Grid::new("stats-resources-grid")
                .num_columns(2)
                .spacing([28.0, 6.0])
                .show(ui, |ui| {
                    ui.weak(if french {
                        "Mémoire de l'application"
                    } else {
                        "App memory (RSS)"
                    });
                    ui.strong(format_bytes_human(snapshot.app_rss_bytes));
                    ui.end_row();

                    ui.weak(if french {
                        "Processus des panneaux"
                    } else {
                        "Pane processes (RAM / CPU)"
                    });
                    ui.strong(format!(
                        "{} · {:.0}% CPU",
                        format_bytes_human(snapshot.panes_rss_bytes),
                        snapshot.panes_cpu_pct
                    ));
                    ui.end_row();

                    ui.weak(if french {
                        "Ports locaux en écoute"
                    } else {
                        "Listening local ports"
                    });
                    ui.strong(snapshot.listening_ports.to_string());
                    ui.end_row();
                });
        });

        changed
    }

    pub fn show_updates(
        &mut self,
        ui: &mut egui::Ui,
        has_store: bool,
        french: bool,
    ) -> Option<SettingsPageAction> {
        let mut action = None;
        paint_page_header(ui, SettingsPageId::Updates, french);

        section_heading(ui, "Tessel Rust");
        settings_card(ui, |ui| {
            let version_line = format!("v{} · {}", env!("CARGO_PKG_VERSION"), env!("TESSEL_BUILD"));
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(&version_line)
                        .font(FontId::monospace(14.0))
                        .strong(),
                );
                if ui
                    .button(if french {
                        "Copier la version"
                    } else {
                        "Copy version"
                    })
                    .clicked()
                {
                    ui.ctx().copy_text(version_line.clone());
                    self.version_copied = true;
                }
                if ui
                    .button(if french {
                        "Notes de version"
                    } else {
                        "Release notes"
                    })
                    .clicked()
                {
                    action = Some(SettingsPageAction::OpenExternalUrl(
                        "https://github.com/mindkind1337/tessel-rust/commits/main".to_owned(),
                    ));
                }
            });
            if self.version_copied {
                ui.small(if french {
                    "Version copiée."
                } else {
                    "Version copied."
                });
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(if french {
                    "Exécutable natif Rust autonome (egui / ConPTY / WebView2 isolé), sans serveur local ni auto-exécuteur distant."
                } else {
                    "Standalone native Rust executable (egui / ConPTY / isolated WebView2) with zero local HTTP server or background auto-updater."
                })
                .font(FontId::proportional(12.0))
                .color(ui.visuals().weak_text_color()),
            );
        });

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Dépannage et retours"
            } else {
                "Troubleshooting & feedback"
            },
        );
        settings_card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(if french {
                        "Sauvegarde et diagnostic (Général)"
                    } else {
                        "Backup & diagnostics (General)"
                    })
                    .clicked()
                {
                    action = Some(SettingsPageAction::OpenGeneralPage);
                }
                if ui
                    .add_enabled(
                        has_store,
                        egui::Button::new(if french {
                            "Ouvrir le dossier de données"
                        } else {
                            "Open data folder"
                        }),
                    )
                    .clicked()
                {
                    action = Some(SettingsPageAction::OpenDataFolder);
                }
                if ui
                    .button(if french {
                        "Signaler un problème"
                    } else {
                        "Report a problem"
                    })
                    .clicked()
                {
                    action = Some(SettingsPageAction::OpenExternalUrl(
                        "https://github.com/mindkind1337/tessel-rust/issues".to_owned(),
                    ));
                }
            });
        });

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Licences open source"
            } else {
                "Open-source licenses"
            },
        );
        settings_card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(if french { "Licence MIT" } else { "MIT License" })
                    .clicked()
                {
                    action = Some(SettingsPageAction::OpenNoticeFile("LICENSE"));
                }
                if ui
                    .button(if french {
                        "Avis relatifs aux tiers"
                    } else {
                        "Third-party notices"
                    })
                    .clicked()
                {
                    action = Some(SettingsPageAction::OpenNoticeFile("THIRD_PARTY_NOTICES.md"));
                }
            });
        });

        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_prefix_validation_and_preview_cover_all_modes() {
        assert_eq!(
            validate_git_prefix_segment("  feat/sub/ ").unwrap(),
            "feat/sub"
        );
        assert!(validate_git_prefix_segment("").is_err());
        assert!(validate_git_prefix_segment("-bad").is_err());
        assert!(validate_git_prefix_segment("a..b").is_err());
        assert!(validate_git_prefix_segment("bad space").is_err());
        assert!(validate_git_prefix_segment("foo.lock").is_err());

        assert_eq!(
            preview_branch_name(
                BranchPrefixMode::Custom,
                "team/ai",
                Some("Alice"),
                "fix-login"
            ),
            "team/ai/fix-login"
        );
        assert_eq!(
            preview_branch_name(
                BranchPrefixMode::GitUsername,
                "ignored",
                Some("Jean C."),
                "fix-login"
            ),
            "jean-c/fix-login"
        );
        assert_eq!(
            preview_branch_name(BranchPrefixMode::None, "agent", Some("Jean"), "fix-login"),
            "fix-login"
        );
    }
}
