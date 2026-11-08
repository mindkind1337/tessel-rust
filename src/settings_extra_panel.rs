//! Native `egui` UI renderers for the remaining 10 Settings pages (`Accounts`,
//! `Skills`, `Mcp`, `Connections`, `Context`, `Orchestration`, `Alerts`, `Voice`,
//! `Devtools`, `Automations`) and Custom CLI Agents (`CustomAgent` form in `Agents`).

use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, RichText, Stroke};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::automations::{
    Automation, AutomationAgent, AutomationIsolation, AutomationSchedule, AutomationsFile,
    MAX_AUTOMATIONS, STARTER_TEMPLATES, generate_automation_id, weekday_label,
};
use crate::settings_extra_model::{
    AlertSound, CUSTOM_AGENT_SWATCHES, CliSignInProbe, ContextModelFamily, ContextPreferences,
    ContextScanReport, CustomAgent, DEVTOOLS_CATALOG, DevToolStatus, InstalledMcpServer,
    MAX_CUSTOM_AGENTS, MCP_CATALOG, ManagedAccountsRoster, McpCategory, McpTransport,
    MinimaxEndpoint, NotificationPreferences, OrchestrationPreferences, PROVIDER_SECRET_KEYS,
    ProviderAccountsPreferences, SkillEntry, SkillScope, TEAM_TOOLS_REFERENCE, TESSEL_CONNECTORS,
    VoicePreferences, WorkerMode, ZcodePlanSite, generate_custom_agent_id,
    is_valid_mcp_server_name, parse_hex_rgb, supported_voice_languages,
    validate_custom_agent_fields, winget_install_command,
};
use crate::settings_model::{SettingsLang, SettingsPageId};
use crate::theme;

#[derive(Clone, Debug)]
pub enum SettingsExtraAction {
    // Accounts
    AddManagedAccount {
        provider: String,
        label: String,
    },
    SelectManagedAccount {
        provider: String,
        account_id: Option<String>,
    },
    RemoveManagedAccount {
        provider: String,
        account_id: String,
    },
    LaunchManagedAccountSignIn {
        provider: String,
        account_id: String,
    },
    SaveProviderSecret {
        key: String,
        value: String,
    },
    ClearProviderSecret {
        key: String,
    },
    // Skills
    RefreshSkills,
    PickSkillFolder {
        project_scope: bool,
    },
    CloneSkillFromGit {
        repo_url: String,
        project_scope: bool,
    },
    OpenSkillMarkdown(PathBuf),
    RemoveSkillDir(PathBuf),
    // MCP
    RefreshMcp,
    InstallCatalogMcp {
        catalog_id: &'static str,
        target_agent_idx: usize,
        env_values: Vec<(String, String)>,
    },
    AddCustomMcp {
        target_agent_idx: usize,
        name: String,
        transport: McpTransport,
        endpoint: String,
        args: Vec<String>,
        env_values: Vec<(String, String)>,
    },
    RemoveInstalledMcp {
        config_path: PathBuf,
        agent_id: String,
        name: String,
    },
    TestMcpServer {
        name: String,
        transport: McpTransport,
        endpoint: String,
    },
    SignInMcpAgent {
        name: String,
        agent_id: String,
        agent_label: String,
    },
    SignInAllMcpServer {
        name: String,
    },
    // Connections
    OpenTesselAccountSettings,
    QuickAddProductivityMcp(&'static str),
    // Context
    RefreshContext,
    // Notifications
    SendTestNotification,
    // Voice
    TriggerWindowsVoiceTyping,
    // Devtools
    RefreshDevtools,
    RunTerminalSetupCommand {
        title: String,
        command: String,
    },
    OpenExternalUrl(String),
    // Automations
    UpsertAutomation(Automation),
    DeleteAutomation(String),
    ConfirmAndEnableAutomation(String),
    RunAutomationNow(String),
    SetAutomationsMaxConcurrent(u8),
    // Custom CLI Agents
    UpsertCustomAgent(CustomAgent),
    DeleteCustomAgent(String),
    ToggleCustomAgent {
        id: String,
        enabled: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SkillsTab {
    #[default]
    Installed,
    Add,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum McpTab {
    #[default]
    Installed,
    Catalog,
    Custom,
}

#[derive(Clone, Debug)]
pub struct AutomationDraft {
    pub id: Option<String>,
    pub name: String,
    pub project_id: u64,
    pub agent: AutomationAgent,
    pub isolation: AutomationIsolation,
    pub schedule_kind: usize, // 0 Hourly, 1 Daily, 2 Weekdays, 3 Weekly, 4 Cron
    pub hour: u8,
    pub minute: u8,
    pub weekday: u8,
    pub cron_expr: String,
    pub prompt: String,
    pub error: String,
}

impl Default for AutomationDraft {
    fn default() -> Self {
        Self {
            id: None,
            name: String::new(),
            project_id: 0,
            agent: AutomationAgent::Claude,
            isolation: AutomationIsolation::Worktree,
            schedule_kind: 1,
            hour: 9,
            minute: 0,
            weekday: 1,
            cron_expr: "0 9 * * 1-5".into(),
            prompt: String::new(),
            error: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SettingsExtraState {
    pub new_claude_label: String,
    pub new_codex_label: String,
    pub secret_drafts: BTreeMap<String, String>,
    pub accounts_notice: String,
    pub skills_tab: SkillsTab,
    pub skills_filter: String,
    pub add_skill_project_scope: bool,
    pub add_skill_git_url: String,
    pub confirm_remove_skill: Option<PathBuf>,
    pub skills_status: String,
    pub mcp_tab: McpTab,
    pub mcp_target_idx: usize,
    pub mcp_catalog_filter: String,
    pub mcp_env_drafts: BTreeMap<String, String>,
    pub custom_mcp_name: String,
    pub custom_mcp_http: bool,
    pub custom_mcp_endpoint: String,
    pub custom_mcp_args: String,
    pub custom_mcp_env: String,
    pub mcp_error: String,
    pub mcp_test_status: String,
    pub mcp_probe_outcomes: BTreeMap<String, crate::settings_extra_model::McpProbeOutcome>,
    pub mcp_card_messages: BTreeMap<String, String>,
    pub automation_draft: Option<AutomationDraft>,
    pub pending_confirm_automation: Option<String>,
    pub automations_status: String,
    pub custom_agent_form_open: bool,
    pub editing_custom_id: Option<String>,
    pub custom_name: String,
    pub custom_command: String,
    pub custom_accent: String,
    pub custom_error: String,
}

impl SettingsExtraState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            custom_accent: CUSTOM_AGENT_SWATCHES[0].to_owned(),
            ..Default::default()
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
            add_contents(ui)
        })
        .inner
}

pub struct AccountsViewInput<'a> {
    pub claude_roster: &'a ManagedAccountsRoster,
    pub codex_roster: &'a ManagedAccountsRoster,
    pub claude_system_email: Option<&'a str>,
    pub codex_system_email: Option<&'a str>,
    pub secret_saved_flags: &'a BTreeMap<String, bool>,
    pub cli_sign_ins: &'a CliSignInProbe,
    pub ssh_host_aliases: &'a [String],
    pub french: bool,
}

impl SettingsExtraState {
    pub fn show_accounts(
        &mut self,
        ui: &mut egui::Ui,
        prefs: &mut ProviderAccountsPreferences,
        input: &AccountsViewInput<'_>,
    ) -> (bool, Option<SettingsExtraAction>) {
        let mut changed = false;
        let mut action = None;
        let fr = input.french;

        paint_page_header(ui, SettingsPageId::Accounts, fr);

        section_heading(
            ui,
            if fr {
                "Comptes Claude Code gérés"
            } else {
                "Managed Claude Code accounts"
            },
        );
        settings_card(ui, |ui| {
            render_managed_roster_card(
                ui,
                "claude",
                "CLAUDE_CONFIG_DIR",
                input.claude_roster,
                input.claude_system_email,
                &mut self.new_claude_label,
                fr,
                &mut action,
            );
        });

        ui.add_space(10.0);

        section_heading(
            ui,
            if fr {
                "Comptes OpenAI Codex gérés"
            } else {
                "Managed OpenAI Codex accounts"
            },
        );
        settings_card(ui, |ui| {
            render_managed_roster_card(
                ui,
                "codex",
                "CODEX_HOME",
                input.codex_roster,
                input.codex_system_email,
                &mut self.new_codex_label,
                fr,
                &mut action,
            );
        });

        ui.add_space(10.0);

        section_heading(
            ui,
            if fr {
                "Fournisseurs & jetons d'utilisation (chiffrés localement)"
            } else {
                "Provider quotas & encrypted credentials"
            },
        );
        settings_card(ui, |ui| {
            if ui
                .checkbox(
                    &mut prefs.gemini_cli_oauth,
                    if fr {
                        "Lire le quota OAuth local de Gemini CLI"
                    } else {
                        "Read local Gemini CLI OAuth quota"
                    },
                )
                .changed()
            {
                changed = true;
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(if fr {
                    "ID espace OpenCode :"
                } else {
                    "OpenCode workspace ID:"
                });
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut prefs.opencode_workspace_id)
                            .desired_width(220.0),
                    )
                    .changed()
                {
                    changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label(if fr {
                    "Point de terminaison MiniMax :"
                } else {
                    "MiniMax endpoint:"
                });
                if ui
                    .selectable_value(
                        &mut prefs.minimax_endpoint,
                        MinimaxEndpoint::Overseas,
                        "Overseas (Global)",
                    )
                    .changed()
                    || ui
                        .selectable_value(
                            &mut prefs.minimax_endpoint,
                            MinimaxEndpoint::Cn,
                            "Mainland China (CN)",
                        )
                        .changed()
                {
                    changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("MiniMax Group ID:");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut prefs.minimax_group_id)
                            .desired_width(180.0),
                    )
                    .changed()
                {
                    changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label(if fr {
                    "Site Z-Code / GLM :"
                } else {
                    "Z-Code plan site:"
                });
                if ui
                    .selectable_value(&mut prefs.zcode_plan_site, ZcodePlanSite::Zai, "Z.ai")
                    .changed()
                    || ui
                        .selectable_value(
                            &mut prefs.zcode_plan_site,
                            ZcodePlanSite::Bigmodel,
                            "BigModel.cn",
                        )
                        .changed()
                {
                    changed = true;
                }
            });

            ui.separator();
            ui.small(if fr {
                "Les clés API et cookies sont scellés via Windows DPAPI et ne sont jamais ré-affichés ni exportés."
            } else {
                "API keys and cookies are sealed with Windows DPAPI and never echoed back or exported."
            });
            ui.add_space(4.0);
            for key in PROVIDER_SECRET_KEYS {
                let is_saved = input.secret_saved_flags.get(key).copied().unwrap_or(false);
                let draft = self.secret_drafts.entry(key.to_owned()).or_default();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(key).monospace());
                    ui.small(if is_saved {
                        if fr { "(Enregistré)" } else { "(Saved)" }
                    } else if fr {
                        "(Non configuré)"
                    } else {
                        "(Not set)"
                    });
                    ui.add(
                        egui::TextEdit::singleline(draft)
                            .password(true)
                            .hint_text("••••••••")
                            .desired_width(180.0),
                    );
                    if ui
                        .add_enabled(
                            !draft.trim().is_empty(),
                            egui::Button::new(if fr { "Enregistrer" } else { "Save" }),
                        )
                        .clicked()
                    {
                        let val = std::mem::take(draft);
                        action = Some(SettingsExtraAction::SaveProviderSecret {
                            key: key.to_owned(),
                            value: val,
                        });
                    }
                    if is_saved && ui.button(if fr { "Effacer" } else { "Clear" }).clicked() {
                        action = Some(SettingsExtraAction::ClearProviderSecret {
                            key: key.to_owned(),
                        });
                    }
                });
            }
        });

        ui.add_space(10.0);

        section_heading(
            ui,
            if fr {
                "Connexions CLI détectées & hôtes par défaut"
            } else {
                "Detected CLI sign-ins & host defaults"
            },
        );
        settings_card(ui, |ui| {
            for (name, ok) in [
                (
                    "Grok CLI (~/.grok/auth.json)",
                    input.cli_sign_ins.grok_signed_in,
                ),
                (
                    "Muse CLI (~/.config/muse/auth.json)",
                    input.cli_sign_ins.muse_signed_in,
                ),
                ("Cursor CLI", input.cli_sign_ins.cursor_signed_in),
            ] {
                ui.horizontal(|ui| {
                    ui.label(name);
                    ui.small(if ok {
                        if fr { "Connecté" } else { "Signed in" }
                    } else if fr {
                        "Non connecté"
                    } else {
                        "Not signed in"
                    });
                });
            }

            if !input.ssh_host_aliases.is_empty() && !input.claude_roster.accounts.is_empty() {
                ui.separator();
                ui.small(if fr {
                    "Compte Claude par défaut par hôte :"
                } else {
                    "Default Claude account per host:"
                });
                for host in input.ssh_host_aliases {
                    let current = prefs
                        .host_account_defaults
                        .get(host)
                        .and_then(|m| m.get("claude"))
                        .cloned()
                        .unwrap_or_default();
                    ui.horizontal(|ui| {
                        ui.label(host);
                        egui::ComboBox::from_id_salt(("host_acct", host))
                            .selected_text(if current.is_empty() {
                                if fr {
                                    "Par défaut système"
                                } else {
                                    "System default"
                                }
                            } else {
                                input
                                    .claude_roster
                                    .accounts
                                    .iter()
                                    .find(|a| a.id == current)
                                    .map_or(current.as_str(), |a| a.label.as_str())
                            })
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        current.is_empty(),
                                        if fr {
                                            "Par défaut système"
                                        } else {
                                            "System default"
                                        },
                                    )
                                    .clicked()
                                {
                                    if let Some(m) = prefs.host_account_defaults.get_mut(host) {
                                        m.remove("claude");
                                    }
                                    changed = true;
                                }
                                for acct in &input.claude_roster.accounts {
                                    if ui
                                        .selectable_label(current == acct.id, &acct.label)
                                        .clicked()
                                    {
                                        prefs
                                            .host_account_defaults
                                            .entry(host.clone())
                                            .or_default()
                                            .insert("claude".into(), acct.id.clone());
                                        changed = true;
                                    }
                                }
                            });
                    });
                }
            }
        });

        if !self.accounts_notice.is_empty() {
            ui.add_space(6.0);
            ui.small(&self.accounts_notice);
        }

        (changed, action)
    }

    pub fn show_skills(
        &mut self,
        ui: &mut egui::Ui,
        skills: &[SkillEntry],
        has_active_project: bool,
        french: bool,
    ) -> Option<SettingsExtraAction> {
        let mut action = None;
        paint_page_header(ui, SettingsPageId::Skills, french);

        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.skills_tab,
                SkillsTab::Installed,
                if french {
                    format!("Installées ({})", skills.len())
                } else {
                    format!("Installed ({})", skills.len())
                },
            );
            ui.selectable_value(
                &mut self.skills_tab,
                SkillsTab::Add,
                if french { "Ajouter" } else { "Add" },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(if french { "Actualiser" } else { "Refresh" })
                    .clicked()
                {
                    action = Some(SettingsExtraAction::RefreshSkills);
                }
            });
        });
        ui.add_space(8.0);

        match self.skills_tab {
            SkillsTab::Installed => {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.skills_filter)
                            .hint_text(if french {
                                "Filtrer les compétences…"
                            } else {
                                "Filter skills…"
                            })
                            .desired_width(280.0),
                    );
                });
                ui.add_space(6.0);
                let q = self.skills_filter.trim().to_lowercase();
                let filtered: Vec<&SkillEntry> = skills
                    .iter()
                    .filter(|s| {
                        q.is_empty()
                            || s.name.to_lowercase().contains(&q)
                            || s.description.to_lowercase().contains(&q)
                            || s.agent_label.to_lowercase().contains(&q)
                    })
                    .collect();

                if filtered.is_empty() {
                    settings_card(ui, |ui| {
                        ui.label(if french {
                            "Aucune compétence SKILL.md détectée."
                        } else {
                            "No SKILL.md skills detected."
                        });
                    });
                } else {
                    for skill in filtered {
                        settings_card(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.strong(&skill.name);
                                ui.small(format!(
                                    "[{} · {}]",
                                    skill.agent_label,
                                    match (skill.scope, french) {
                                        (SkillScope::User, false) => "User",
                                        (SkillScope::User, true) => "Utilisateur",
                                        (SkillScope::Project, false) => "Project",
                                        (SkillScope::Project, true) => "Projet",
                                    }
                                ));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if self.confirm_remove_skill.as_ref()
                                            == Some(&skill.dir_path)
                                        {
                                            if ui
                                                .button(if french {
                                                    "Confirmer la suppression"
                                                } else {
                                                    "Confirm delete"
                                                })
                                                .clicked()
                                            {
                                                self.confirm_remove_skill = None;
                                                action = Some(SettingsExtraAction::RemoveSkillDir(
                                                    skill.dir_path.clone(),
                                                ));
                                            }
                                            if ui
                                                .button(if french { "Annuler" } else { "Cancel" })
                                                .clicked()
                                            {
                                                self.confirm_remove_skill = None;
                                            }
                                        } else {
                                            if ui
                                                .button(if french { "Supprimer" } else { "Remove" })
                                                .clicked()
                                            {
                                                self.confirm_remove_skill =
                                                    Some(skill.dir_path.clone());
                                            }
                                            if ui.button("SKILL.md").clicked() {
                                                action =
                                                    Some(SettingsExtraAction::OpenSkillMarkdown(
                                                        skill.skill_md_path.clone(),
                                                    ));
                                            }
                                        }
                                    },
                                );
                            });
                            if !skill.description.is_empty() {
                                ui.small(&skill.description);
                            }
                        });
                        ui.add_space(6.0);
                    }
                }
            }
            SkillsTab::Add => {
                settings_card(ui, |ui| {
                    ui.strong(if french {
                        "Installer une compétence locale ou GitHub"
                    } else {
                        "Install a skill from folder or GitHub"
                    });
                    ui.add_space(6.0);
                    if has_active_project {
                        ui.checkbox(
                            &mut self.add_skill_project_scope,
                            if french {
                                "Installer uniquement dans le projet actif (.agents/skills)"
                            } else {
                                "Install into active project only (.agents/skills)"
                            },
                        );
                    } else {
                        self.add_skill_project_scope = false;
                    }
                    ui.add_space(6.0);
                    if ui
                        .button(if french {
                            "Importer un dossier local contenant SKILL.md…"
                        } else {
                            "Import local folder containing SKILL.md…"
                        })
                        .clicked()
                    {
                        action = Some(SettingsExtraAction::PickSkillFolder {
                            project_scope: self.add_skill_project_scope,
                        });
                    }
                    ui.separator();
                    ui.label(if french {
                        "Cloner depuis une URL de dépôt Git / GitHub :"
                    } else {
                        "Clone from a Git / GitHub repository URL:"
                    });
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.add_skill_git_url)
                                .hint_text("https://github.com/owner/skill-repo.git")
                                .desired_width(340.0),
                        );
                        if ui
                            .add_enabled(
                                !self.add_skill_git_url.trim().is_empty(),
                                egui::Button::new(if french { "Cloner" } else { "Clone" }),
                            )
                            .clicked()
                        {
                            action = Some(SettingsExtraAction::CloneSkillFromGit {
                                repo_url: self.add_skill_git_url.trim().to_owned(),
                                project_scope: self.add_skill_project_scope,
                            });
                        }
                    });
                });
            }
        }

        if !self.skills_status.is_empty() {
            ui.add_space(6.0);
            ui.small(&self.skills_status);
        }
        action
    }

    pub fn show_mcp(
        &mut self,
        ui: &mut egui::Ui,
        installed: &[InstalledMcpServer],
        target_labels: &[String],
        french: bool,
    ) -> Option<SettingsExtraAction> {
        let mut action = None;
        paint_page_header(ui, SettingsPageId::Mcp, french);

        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.mcp_tab,
                McpTab::Installed,
                if french {
                    format!("Installés ({})", installed.len())
                } else {
                    format!("Installed ({})", installed.len())
                },
            );
            ui.selectable_value(
                &mut self.mcp_tab,
                McpTab::Catalog,
                if french {
                    format!("Catalogue ({})", MCP_CATALOG.len())
                } else {
                    format!("Catalog ({})", MCP_CATALOG.len())
                },
            );
            ui.selectable_value(
                &mut self.mcp_tab,
                McpTab::Custom,
                if french { "Personnalisé" } else { "Custom" },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(if french { "Actualiser" } else { "Refresh" })
                    .clicked()
                {
                    action = Some(SettingsExtraAction::RefreshMcp);
                }
            });
        });
        ui.add_space(8.0);

        if !target_labels.is_empty() && self.mcp_tab != McpTab::Installed {
            self.mcp_target_idx = self.mcp_target_idx.min(target_labels.len() - 1);
            ui.horizontal(|ui| {
                ui.label(if french {
                    "Fichier de configuration cible :"
                } else {
                    "Target agent configuration:"
                });
                egui::ComboBox::from_id_salt("mcp_target_agent")
                    .selected_text(&target_labels[self.mcp_target_idx])
                    .show_ui(ui, |ui| {
                        for (idx, label) in target_labels.iter().enumerate() {
                            ui.selectable_value(&mut self.mcp_target_idx, idx, label);
                        }
                    });
            });
            ui.add_space(6.0);
        }

        match self.mcp_tab {
            McpTab::Installed => {
                if installed.is_empty() {
                    settings_card(ui, |ui| {
                        ui.label(if french {
                            "Aucun serveur MCP configuré. Parcourez l'onglet Catalogue pour en ajouter."
                        } else {
                            "No MCP servers configured yet. Browse the Catalog tab to add one."
                        });
                    });
                } else {
                    let mut shown_bulk_for = std::collections::HashSet::new();
                    for srv in installed {
                        let remote_count = if srv.transport == McpTransport::Http {
                            installed
                                .iter()
                                .filter(|s| s.transport == McpTransport::Http && s.name == srv.name)
                                .count()
                        } else {
                            0
                        };
                        let show_bulk_here =
                            remote_count > 1 && shown_bulk_for.insert(srv.name.clone());
                        let card_key = format!("{}:{}", srv.agent_id, srv.name);
                        let bulk_key = format!("all:{}", srv.name);

                        settings_card(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.strong(&srv.name);
                                ui.small(format!(
                                    "[{} · {}]",
                                    srv.agent_label,
                                    match srv.transport {
                                        McpTransport::Stdio => "stdio",
                                        McpTransport::Http => "http",
                                    }
                                ));
                                if self.mcp_probe_outcomes.get(&srv.name)
                                    == Some(
                                        &crate::settings_extra_model::McpProbeOutcome::SignsInFromAgent,
                                    )
                                {
                                    ui.label(
                                        RichText::new(
                                            crate::settings_extra_model::McpProbeOutcome::signs_in_badge(
                                                french,
                                            ),
                                        )
                                        .small()
                                        .color(ui.visuals().warn_fg_color),
                                    )
                                    .on_hover_text(
                                        crate::settings_extra_model::McpProbeOutcome::signs_in_tooltip(
                                            french,
                                        ),
                                    );
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if srv.agent_id != "codex"
                                            && ui
                                                .button(if french { "Retirer" } else { "Remove" })
                                                .clicked()
                                        {
                                            self.mcp_card_messages.remove(&card_key);
                                            action =
                                                Some(SettingsExtraAction::RemoveInstalledMcp {
                                                    config_path: srv.config_path.clone(),
                                                    agent_id: srv.agent_id.clone(),
                                                    name: srv.name.clone(),
                                                });
                                        }
                                        if ui
                                            .button(if french { "Tester" } else { "Test" })
                                            .clicked()
                                        {
                                            self.mcp_card_messages.remove(&card_key);
                                            action = Some(SettingsExtraAction::TestMcpServer {
                                                name: srv.name.clone(),
                                                transport: srv.transport,
                                                endpoint: srv.endpoint_summary.clone(),
                                            });
                                        }
                                        if srv.transport == McpTransport::Http
                                            && ui
                                                .button(if french {
                                                    "Se connecter"
                                                } else {
                                                    "Sign in"
                                                })
                                                .clicked()
                                        {
                                            self.mcp_card_messages.remove(&card_key);
                                            self.mcp_card_messages.remove(&bulk_key);
                                            action = Some(SettingsExtraAction::SignInMcpAgent {
                                                name: srv.name.clone(),
                                                agent_id: srv.agent_id.clone(),
                                                agent_label: srv.agent_label.clone(),
                                            });
                                        }
                                        if show_bulk_here
                                            && ui
                                                .button(if french {
                                                    "Tout connecter"
                                                } else {
                                                    "Sign in all"
                                                })
                                                .clicked()
                                        {
                                            self.mcp_card_messages.remove(&card_key);
                                            self.mcp_card_messages.remove(&bulk_key);
                                            action = Some(SettingsExtraAction::SignInAllMcpServer {
                                                name: srv.name.clone(),
                                            });
                                        }
                                    },
                                );
                            });
                            ui.small(RichText::new(&srv.endpoint_summary).monospace());
                            if !srv.env_keys.is_empty() {
                                ui.small(format!("env: {} (masked)", srv.env_keys.join(", ")));
                            }
                            for msg_key in [
                                card_key.as_str(),
                                if show_bulk_here {
                                    bulk_key.as_str()
                                } else {
                                    ""
                                },
                            ] {
                                if msg_key.is_empty() {
                                    continue;
                                }
                                if let Some(msg) = self.mcp_card_messages.get(msg_key).cloned() {
                                    let mut dismiss = false;
                                    ui.horizontal_wrapped(|ui| {
                                        ui.small(&msg);
                                        if ui
                                            .small_button("×")
                                            .on_hover_text(if french {
                                                "Fermer"
                                            } else {
                                                "Dismiss"
                                            })
                                            .clicked()
                                        {
                                            dismiss = true;
                                        }
                                    });
                                    if dismiss {
                                        self.mcp_card_messages.remove(msg_key);
                                    }
                                }
                            }
                        });
                        ui.add_space(6.0);
                    }
                }
            }
            McpTab::Catalog => {
                ui.add(
                    egui::TextEdit::singleline(&mut self.mcp_catalog_filter)
                        .hint_text(if french {
                            "Rechercher parmi les 30 serveurs MCP…"
                        } else {
                            "Search 30 MCP servers…"
                        })
                        .desired_width(280.0),
                );
                ui.add_space(6.0);
                let q = self.mcp_catalog_filter.trim().to_lowercase();
                for entry in &MCP_CATALOG {
                    let desc = if french { entry.desc_fr } else { entry.desc_en };
                    if !q.is_empty()
                        && !entry.name.to_lowercase().contains(&q)
                        && !entry.id.contains(&q)
                        && !desc.to_lowercase().contains(&q)
                    {
                        continue;
                    }
                    let already = installed.iter().any(|i| i.name == entry.id);
                    settings_card(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong(entry.name);
                            ui.small(format!("[{}]", entry.category.label(french)));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let btn_label = if already {
                                        if french { "Réinstaller" } else { "Re-add" }
                                    } else if french {
                                        "Ajouter"
                                    } else {
                                        "Add"
                                    };
                                    if ui.button(btn_label).clicked() {
                                        let mut envs = Vec::new();
                                        for &k in entry.env_keys {
                                            let key_id = format!("{}:{}", entry.id, k);
                                            if let Some(val) = self.mcp_env_drafts.get(&key_id)
                                                && !val.trim().is_empty()
                                            {
                                                envs.push((k.to_owned(), val.trim().to_owned()));
                                            }
                                        }
                                        action = Some(SettingsExtraAction::InstallCatalogMcp {
                                            catalog_id: entry.id,
                                            target_agent_idx: self.mcp_target_idx,
                                            env_values: envs,
                                        });
                                    }
                                },
                            );
                        });
                        ui.small(desc);
                        for &env_key in entry.env_keys {
                            let draft_key = format!("{}:{}", entry.id, env_key);
                            let val = self.mcp_env_drafts.entry(draft_key).or_default();
                            ui.horizontal(|ui| {
                                ui.small(RichText::new(env_key).monospace());
                                ui.add(
                                    egui::TextEdit::singleline(val)
                                        .password(true)
                                        .desired_width(200.0),
                                );
                            });
                        }
                    });
                    ui.add_space(6.0);
                }
            }
            McpTab::Custom => {
                settings_card(ui, |ui| {
                    ui.strong(if french {
                        "Ajouter un serveur MCP personnalisé"
                    } else {
                        "Add custom MCP server"
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(if french { "Nom :" } else { "Name:" });
                        ui.add(
                            egui::TextEdit::singleline(&mut self.custom_mcp_name)
                                .hint_text("my-mcp-server")
                                .desired_width(200.0),
                        );
                        ui.selectable_value(&mut self.custom_mcp_http, false, "stdio");
                        ui.selectable_value(&mut self.custom_mcp_http, true, "HTTP / SSE");
                    });
                    ui.horizontal(|ui| {
                        ui.label(if self.custom_mcp_http {
                            "URL:"
                        } else if french {
                            "Commande :"
                        } else {
                            "Command:"
                        });
                        ui.add(
                            egui::TextEdit::singleline(&mut self.custom_mcp_endpoint)
                                .hint_text(if self.custom_mcp_http {
                                    "https://example.com/mcp"
                                } else {
                                    "npx"
                                })
                                .desired_width(300.0),
                        );
                    });
                    if !self.custom_mcp_http {
                        ui.horizontal(|ui| {
                            ui.label(if french {
                                "Arguments (séparés par espaces) :"
                            } else {
                                "Arguments (space-separated):"
                            });
                            ui.add(
                                egui::TextEdit::singleline(&mut self.custom_mcp_args)
                                    .desired_width(260.0),
                            );
                        });
                    }
                    ui.label(if french {
                        "Variables d'environnement (KEY=VALUE par ligne) :"
                    } else {
                        "Environment variables (KEY=VALUE per line):"
                    });
                    ui.add(
                        egui::TextEdit::multiline(&mut self.custom_mcp_env)
                            .desired_rows(3)
                            .desired_width(420.0),
                    );
                    if !self.mcp_error.is_empty() {
                        ui.colored_label(ui.visuals().error_fg_color, &self.mcp_error);
                    }
                    if ui
                        .button(if french {
                            "Enregistrer le serveur MCP"
                        } else {
                            "Save MCP server"
                        })
                        .clicked()
                    {
                        let name = self.custom_mcp_name.trim().to_owned();
                        if !is_valid_mcp_server_name(&name) {
                            self.mcp_error = if french {
                                "Nom invalide ([A-Za-z0-9_-], max 64).".into()
                            } else {
                                "Invalid server name ([A-Za-z0-9_-], max 64).".into()
                            };
                        } else if self.custom_mcp_endpoint.trim().is_empty() {
                            self.mcp_error = if french {
                                "Indiquez une commande ou URL.".into()
                            } else {
                                "Enter a command or URL.".into()
                            };
                        } else {
                            self.mcp_error.clear();
                            let args: Vec<String> = self
                                .custom_mcp_args
                                .split_whitespace()
                                .map(str::to_owned)
                                .collect();
                            let env_values: Vec<(String, String)> = self
                                .custom_mcp_env
                                .lines()
                                .filter_map(|l| l.split_once('='))
                                .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
                                .filter(|(k, _)| !k.is_empty())
                                .collect();
                            action = Some(SettingsExtraAction::AddCustomMcp {
                                target_agent_idx: self.mcp_target_idx,
                                name,
                                transport: if self.custom_mcp_http {
                                    McpTransport::Http
                                } else {
                                    McpTransport::Stdio
                                },
                                endpoint: self.custom_mcp_endpoint.trim().to_owned(),
                                args,
                                env_values,
                            });
                            self.custom_mcp_name.clear();
                            self.custom_mcp_endpoint.clear();
                            self.custom_mcp_args.clear();
                            self.custom_mcp_env.clear();
                        }
                    }
                });
            }
        }

        if !self.mcp_test_status.is_empty() {
            ui.add_space(6.0);
            ui.small(&self.mcp_test_status);
        }
        action
    }

    pub fn show_connections(
        &mut self,
        ui: &mut egui::Ui,
        tessel_signed_in: bool,
        installed_mcp: &[InstalledMcpServer],
        french: bool,
    ) -> Option<SettingsExtraAction> {
        let mut action = None;
        paint_page_header(ui, SettingsPageId::Connections, french);

        section_heading(
            ui,
            if french {
                "Connecteurs Tessel"
            } else {
                "Tessel connectors"
            },
        );
        if !tessel_signed_in {
            settings_card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(if french {
                        "La connexion d'un connecteur nécessite un compte Tessel."
                    } else {
                        "Connecting needs a Tessel account."
                    });
                    if ui
                        .button(if french {
                            "Ouvrir Compte Tessel"
                        } else {
                            "Open Tessel account"
                        })
                        .clicked()
                    {
                        action = Some(SettingsExtraAction::OpenTesselAccountSettings);
                    }
                });
            });
            ui.add_space(6.0);
        }

        for conn in &TESSEL_CONNECTORS {
            settings_card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(conn.name);
                    if conn.read_only {
                        ui.small(if french {
                            "(Lecture seule)"
                        } else {
                            "(Read-only)"
                        });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let status = if !conn.available {
                            if french {
                                "Bientôt disponible"
                            } else {
                                "Not available yet"
                            }
                        } else if !tessel_signed_in {
                            if french {
                                "Compte Tessel requis"
                            } else {
                                "Requires Tessel account"
                            }
                        } else if french {
                            "Prêt via Compte Tessel"
                        } else {
                            "Managed via Tessel account"
                        };
                        ui.small(status);
                    });
                });
                ui.small(if french {
                    conn.summary_fr
                } else {
                    conn.summary_en
                });
            });
            ui.add_space(5.0);
        }

        ui.add_space(8.0);
        section_heading(
            ui,
            if french {
                "Services de productivité (MCP)"
            } else {
                "Productivity services (MCP)"
            },
        );
        for entry in MCP_CATALOG
            .iter()
            .filter(|e| e.category == McpCategory::Productivity)
        {
            let added = installed_mcp.iter().any(|m| m.name == entry.id);
            settings_card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(entry.name);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if added {
                            ui.small(if french { "Ajouté" } else { "Added" });
                        } else if ui.button(if french { "Ajouter" } else { "Add" }).clicked() {
                            action = Some(SettingsExtraAction::QuickAddProductivityMcp(entry.id));
                        }
                    });
                });
                ui.small(if french { entry.desc_fr } else { entry.desc_en });
            });
            ui.add_space(5.0);
        }

        action
    }

    pub fn show_context(
        &mut self,
        ui: &mut egui::Ui,
        prefs: &mut ContextPreferences,
        report: &ContextScanReport,
        french: bool,
    ) -> (bool, Option<SettingsExtraAction>) {
        let mut changed = false;
        let mut action = None;
        paint_page_header(ui, SettingsPageId::Context, french);

        settings_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(if french {
                    "Famille de tokeniseur :"
                } else {
                    "Tokenizer model family:"
                });
                egui::ComboBox::from_id_salt("context_model_family")
                    .selected_text(prefs.model_family.label())
                    .show_ui(ui, |ui| {
                        for fam in ContextModelFamily::ALL {
                            if ui
                                .selectable_value(&mut prefs.model_family, fam, fam.label())
                                .changed()
                            {
                                changed = true;
                                action = Some(SettingsExtraAction::RefreshContext);
                            }
                        }
                    });
                if ui
                    .button(if french { "Recalculer" } else { "Rescan" })
                    .clicked()
                {
                    action = Some(SettingsExtraAction::RefreshContext);
                }
            });
            ui.add_space(6.0);
            ui.strong(if french {
                format!(
                    "Coût initial estimé au démarrage d'une session : ~{} jetons",
                    report.total_tokens
                )
            } else {
                format!(
                    "Estimated upfront session context: ~{} tokens",
                    report.total_tokens
                )
            });
        });

        ui.add_space(8.0);
        render_context_group(
            ui,
            if french {
                "1. Fichiers de règles et mémoire (CLAUDE.md, AGENTS.md, GEMINI.md, .cursor/rules…)"
            } else {
                "1. Rules and memory files (CLAUDE.md, AGENTS.md, GEMINI.md, .cursor/rules…)"
            },
            &report.rules_files,
            french,
        );
        render_context_group(
            ui,
            if french {
                "2. Compétences listées au démarrage"
            } else {
                "2. Skills listed up front"
            },
            &report.skills_items,
            french,
        );
        render_context_group(
            ui,
            if french {
                "3. Serveurs MCP et schémas d'outils"
            } else {
                "3. MCP servers and tool definitions"
            },
            &report.mcp_items,
            french,
        );
        settings_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(if french {
                    "4. Outils d'équipe Tessel et règle du tableau des tâches"
                } else {
                    "4. Tessel's team tools and task board rule"
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small(format!("~{} tok", report.team_rule_tokens));
                });
            });
        });

        (changed, action)
    }

    pub fn show_orchestration(
        &mut self,
        ui: &mut egui::Ui,
        prefs: &mut OrchestrationPreferences,
        french: bool,
    ) -> bool {
        let mut changed = false;
        paint_page_header(ui, SettingsPageId::Orchestration, french);

        section_heading(
            ui,
            if french {
                "Réveils d'équipe & garde-fous des workers"
            } else {
                "Team wake-ups & worker safeguards"
            },
        );
        settings_card(ui, |ui| {
            changed |= ui
                .checkbox(
                    &mut prefs.team_wake_ups,
                    if french {
                        "Réveiller automatiquement les agents inactifs lorsqu'un message d'équipe arrive"
                    } else {
                        "Wake idle agents automatically when a team message arrives"
                    },
                )
                .changed();
            changed |= ui
                .checkbox(
                    &mut prefs.team_wake_unconfirmed,
                    if french {
                        "Autoriser les réveils sur les panneaux non confirmés"
                    } else {
                        "Allow wake-ups on unconfirmed panes"
                    },
                )
                .changed();
            changed |= ui
                .checkbox(
                    &mut prefs.confirm_workers,
                    if french {
                        "Demander confirmation avant qu'un agent démarre un worker"
                    } else {
                        "Ask confirmation before an agent spawns a worker"
                    },
                )
                .changed();
            changed |= ui
                .checkbox(
                    &mut prefs.cursor_status_hooks,
                    if french {
                        "Activer les hooks d'état en direct pour Cursor"
                    } else {
                        "Enable live status hooks for Cursor"
                    },
                )
                .changed();
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(if french {
                    "Workers simultanés max (1–8) :"
                } else {
                    "Max concurrent workers (1–8):"
                });
                let mut w = i32::from(prefs.max_workers);
                if ui.add(egui::Slider::new(&mut w, 1..=8)).changed() {
                    prefs.max_workers = w as u8;
                    changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label(if french {
                    "Profondeur de délégation max (1–3) :"
                } else {
                    "Max delegation depth (1–3):"
                });
                let mut d = i32::from(prefs.max_depth);
                if ui.add(egui::Slider::new(&mut d, 1..=3)).changed() {
                    prefs.max_depth = d as u8;
                    changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label(if french {
                    "Mode d'exécution des workers :"
                } else {
                    "Default worker mode:"
                });
                changed |= ui
                    .selectable_value(&mut prefs.worker_mode, WorkerMode::Terminal, "Terminal")
                    .changed();
                changed |= ui
                    .selectable_value(
                        &mut prefs.worker_mode,
                        WorkerMode::Chat,
                        if french { "Chat natif" } else { "Native chat" },
                    )
                    .changed();
            });
        });

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Référence des 14 outils d'orchestration (tessel_team)"
            } else {
                "Reference: 14 orchestration tools (tessel_team)"
            },
        );
        settings_card(ui, |ui| {
            let prompt = if french {
                "Utilise team_create, team_task_add et team_worker_start pour répartir le travail en parallèle."
            } else {
                "Use team_create, team_task_add, and team_worker_start to coordinate parallel workers."
            };
            ui.horizontal(|ui| {
                ui.small(prompt);
                if ui
                    .button(if french {
                        "Copier l'invite"
                    } else {
                        "Copy prompt"
                    })
                    .clicked()
                {
                    ui.ctx().copy_text(prompt.to_owned());
                }
            });
            ui.separator();
            for (tool, en, fr_desc) in TEAM_TOOLS_REFERENCE {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tool).monospace().strong());
                    ui.small(if french { fr_desc } else { en });
                });
            }
        });

        changed
    }

    pub fn show_alerts(
        &mut self,
        ui: &mut egui::Ui,
        prefs: &mut NotificationPreferences,
        french: bool,
    ) -> (bool, Option<SettingsExtraAction>) {
        let mut changed = false;
        let mut action = None;
        paint_page_header(ui, SettingsPageId::Alerts, french);

        settings_card(ui, |ui| {
            changed |= ui
                .checkbox(
                    &mut prefs.notifications_enabled,
                    if french {
                        "Activer les notifications d'agents (tâche terminée, approbation requise, erreur)"
                    } else {
                        "Enable agent notifications (task finished, approval needed, error)"
                    },
                )
                .changed();
            ui.add_enabled_ui(prefs.notifications_enabled, |ui| {
                changed |= ui
                    .checkbox(
                        &mut prefs.desktop_notifications,
                        if french {
                            "Notifications système sur le bureau"
                        } else {
                            "Desktop system notifications"
                        },
                    )
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut prefs.notify_terminal_bell,
                        if french {
                            "Notifier lors d'une cloche de terminal (BEL)"
                        } else {
                            "Notify on terminal bell (BEL)"
                        },
                    )
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut prefs.notify_suppress_when_focused,
                        if french {
                            "Masquer les bannières externes quand Tessel a le focus"
                        } else {
                            "Suppress desktop banners while Tessel is focused"
                        },
                    )
                    .changed();
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(if french {
                        "Son d'alerte :"
                    } else {
                        "Alert sound:"
                    });
                    changed |= ui
                        .selectable_value(
                            &mut prefs.alert_sound,
                            AlertSound::None,
                            if french { "Aucun" } else { "None" },
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut prefs.alert_sound,
                            AlertSound::Chime,
                            if french { "Carillon" } else { "Chime" },
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(&mut prefs.alert_sound, AlertSound::Ping, "Ping")
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label(if french { "Volume :" } else { "Volume:" });
                    let mut vol = i32::from(prefs.notification_volume);
                    if ui
                        .add(
                            egui::Slider::new(&mut vol, 0..=100)
                                .step_by(5.0)
                                .suffix("%"),
                        )
                        .changed()
                    {
                        prefs.notification_volume = vol as u8;
                        changed = true;
                    }
                    if ui
                        .button(if french {
                            "Envoyer un test"
                        } else {
                            "Send a test"
                        })
                        .clicked()
                    {
                        action = Some(SettingsExtraAction::SendTestNotification);
                    }
                });
            });
            ui.separator();
            changed |= ui
                .checkbox(
                    &mut prefs.in_app_alerts,
                    if french {
                        "Afficher les alertes et pastilles dans l'application"
                    } else {
                        "Show in-app alert badges and parked approval pills"
                    },
                )
                .changed();
        });

        (changed, action)
    }

    pub fn show_voice(
        &mut self,
        ui: &mut egui::Ui,
        prefs: &mut VoicePreferences,
        french: bool,
    ) -> (bool, Option<SettingsExtraAction>) {
        let mut changed = false;
        let mut action = None;
        paint_page_header(ui, SettingsPageId::Voice, french);

        let langs = supported_voice_languages(french);
        let current_label = langs
            .iter()
            .find(|(code, _)| *code == prefs.voice_tip)
            .map_or(langs[0].1, |(_, label)| *label);

        settings_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(if french {
                    "Langue de dictée préférée :"
                } else {
                    "Preferred dictation language:"
                });
                egui::ComboBox::from_id_salt("voice_typing_lang")
                    .selected_text(current_label)
                    .show_ui(ui, |ui| {
                        for (code, label) in &langs {
                            if ui
                                .selectable_label(prefs.voice_tip == *code, *label)
                                .clicked()
                            {
                                prefs.voice_tip = (*code).to_owned();
                                prefs.voice_tip_chosen = true;
                                changed = true;
                            }
                        }
                    });
            });
            ui.add_space(6.0);
            ui.small(if french {
                "Sur Windows, appuyez sur Win+H dans n'importe quel terminal, éditeur ou chat pour dicter avec la saisie vocale native."
            } else {
                "On Windows, press Win+H inside any terminal, editor, or native chat to dictate with system voice typing."
            });
            ui.add_space(6.0);
            if cfg!(windows)
                && ui
                    .button(if french {
                        "Ouvrir la dictée vocale Windows (Win+H)"
                    } else {
                        "Launch Windows voice typing (Win+H)"
                    })
                    .clicked()
            {
                action = Some(SettingsExtraAction::TriggerWindowsVoiceTyping);
            }
        });

        (changed, action)
    }

    pub fn show_devtools(
        &mut self,
        ui: &mut egui::Ui,
        statuses: &[DevToolStatus],
        probing: bool,
        french: bool,
    ) -> Option<SettingsExtraAction> {
        let mut action = None;
        paint_page_header(ui, SettingsPageId::Devtools, french);

        ui.horizontal(|ui| {
            if probing {
                ui.spinner();
                ui.small(if french {
                    "Vérification des outils sur le PATH…"
                } else {
                    "Checking developer tools on PATH…"
                });
            } else if ui
                .button(if french { "Actualiser" } else { "Refresh" })
                .clicked()
            {
                action = Some(SettingsExtraAction::RefreshDevtools);
            }
        });
        ui.add_space(8.0);

        for spec in &DEVTOOLS_CATALOG {
            let st = statuses.iter().find(|s| s.id == spec.id);
            let installed = st.is_some_and(|s| s.installed);
            settings_card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(spec.name);
                    if let Some(s) = st
                        && s.installed
                        && !s.version.is_empty()
                    {
                        ui.small(RichText::new(&s.version).monospace());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if installed {
                            ui.small(if french { "Installé" } else { "Installed" });
                            if let Some(s) = st {
                                if spec.id == "git" && s.extra_note == "identity-missing" {
                                    if ui
                                        .button(if french {
                                            "Configurer l'identité Git"
                                        } else {
                                            "Configure Git identity"
                                        })
                                        .clicked()
                                    {
                                        action =
                                            Some(SettingsExtraAction::RunTerminalSetupCommand {
                                                title: "Git identity".into(),
                                                command: "git config --global --list".into(),
                                            });
                                    }
                                } else if spec.id == "gh"
                                    && s.extra_note == "auth-missing"
                                    && ui
                                        .button(if french {
                                            "Se connecter (gh auth login)"
                                        } else {
                                            "Sign in (gh auth login)"
                                        })
                                        .clicked()
                                {
                                    action = Some(SettingsExtraAction::RunTerminalSetupCommand {
                                        title: "GitHub CLI Sign-In".into(),
                                        command: "gh auth login".into(),
                                    });
                                }
                            }
                        } else if cfg!(windows) {
                            if ui
                                .button(if french {
                                    "Installer (winget)"
                                } else {
                                    "Install (winget)"
                                })
                                .clicked()
                            {
                                action = Some(SettingsExtraAction::RunTerminalSetupCommand {
                                    title: format!("Install {}", spec.name),
                                    command: winget_install_command(spec.winget_id),
                                });
                            }
                        } else if ui
                            .button(if french { "Site officiel" } else { "Website" })
                            .clicked()
                        {
                            action = Some(SettingsExtraAction::OpenExternalUrl(
                                spec.docs_url.to_owned(),
                            ));
                        }
                    });
                });
                ui.small(if french { spec.desc_fr } else { spec.desc_en });
            });
            ui.add_space(6.0);
        }

        action
    }

    pub fn show_automations(
        &mut self,
        ui: &mut egui::Ui,
        store: &AutomationsFile,
        projects: &[(u64, String, PathBuf)],
        french: bool,
    ) -> Option<SettingsExtraAction> {
        let mut action = None;
        paint_page_header(ui, SettingsPageId::Automations, french);

        settings_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(if french {
                    "Exécutions simultanées max :"
                } else {
                    "Max concurrent automation runs:"
                });
                let mut mc = i32::from(store.max_concurrent);
                if ui.add(egui::Slider::new(&mut mc, 1..=8)).changed() {
                    action = Some(SettingsExtraAction::SetAutomationsMaxConcurrent(mc as u8));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.automation_draft.is_none()
                        && store.automations.len() < MAX_AUTOMATIONS
                        && !projects.is_empty()
                        && ui
                            .button(if french {
                                "+ Nouvelle automatisation"
                            } else {
                                "+ New automation"
                            })
                            .clicked()
                    {
                        self.automation_draft = Some(AutomationDraft {
                            project_id: projects[0].0,
                            ..Default::default()
                        });
                    }
                });
            });
            if projects.is_empty() {
                ui.small(if french {
                    "Ajoutez d'abord un projet local pour planifier des automatisations."
                } else {
                    "Add a local project first to create scheduled automations."
                });
            }
        });

        if !projects.is_empty() && self.automation_draft.is_none() {
            section_heading(
                ui,
                if french {
                    "Modèles de démarrage rapide"
                } else {
                    "Starter templates"
                },
            );
            ui.horizontal_wrapped(|ui| {
                for tpl in &STARTER_TEMPLATES {
                    let title = if french { tpl.title_fr } else { tpl.title_en };
                    if ui
                        .button(title)
                        .on_hover_text(if french { tpl.desc_fr } else { tpl.desc_en })
                        .clicked()
                    {
                        self.automation_draft = Some(AutomationDraft {
                            id: None,
                            name: title.to_owned(),
                            project_id: projects[0].0,
                            agent: AutomationAgent::Claude,
                            isolation: AutomationIsolation::Worktree,
                            schedule_kind: 1,
                            hour: tpl.default_hour,
                            minute: 0,
                            weekday: 1,
                            cron_expr: format!("0 {} * * 1-5", tpl.default_hour),
                            prompt: (if french { tpl.prompt_fr } else { tpl.prompt_en }).to_owned(),
                            error: String::new(),
                        });
                    }
                }
            });
        }

        if let Some(draft) = &mut self.automation_draft {
            ui.add_space(8.0);
            let mut cancel_form = false;
            let mut saved_auto: Option<Automation> = None;
            settings_card(ui, |ui| {
                ui.strong(if draft.id.is_some() {
                    if french {
                        "Modifier l'automatisation"
                    } else {
                        "Edit automation"
                    }
                } else if french {
                    "Nouvelle automatisation"
                } else {
                    "New automation"
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(if french { "Nom :" } else { "Name:" });
                    ui.add(egui::TextEdit::singleline(&mut draft.name).desired_width(260.0));
                });
                ui.horizontal(|ui| {
                    ui.label(if french { "Projet :" } else { "Project:" });
                    let current_proj = projects
                        .iter()
                        .find(|(id, ..)| *id == draft.project_id)
                        .map_or("", |(_, n, _)| n.as_str());
                    egui::ComboBox::from_id_salt("auto_proj_combo")
                        .selected_text(current_proj)
                        .show_ui(ui, |ui| {
                            for (pid, pname, _) in projects {
                                ui.selectable_value(&mut draft.project_id, *pid, pname);
                            }
                        });
                    ui.label("Agent:");
                    egui::ComboBox::from_id_salt("auto_agent_combo")
                        .selected_text(draft.agent.label())
                        .show_ui(ui, |ui| {
                            for ag in AutomationAgent::ALL {
                                ui.selectable_value(&mut draft.agent, ag, ag.label());
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label(if french { "Isolation :" } else { "Isolation:" });
                    ui.selectable_value(
                        &mut draft.isolation,
                        AutomationIsolation::Worktree,
                        AutomationIsolation::Worktree.label(french),
                    );
                    ui.selectable_value(
                        &mut draft.isolation,
                        AutomationIsolation::Project,
                        AutomationIsolation::Project.label(french),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(if french { "Fréquence :" } else { "Schedule:" });
                    for (idx, lbl_en, lbl_fr) in [
                        (0, "Hourly", "Horaire"),
                        (1, "Daily", "Quotidien"),
                        (2, "Weekdays", "Lun–Ven"),
                        (3, "Weekly", "Hebdo"),
                        (4, "Cron", "Cron"),
                    ] {
                        ui.selectable_value(
                            &mut draft.schedule_kind,
                            idx,
                            if french { lbl_fr } else { lbl_en },
                        );
                    }
                });
                ui.horizontal(|ui| match draft.schedule_kind {
                    0 => {
                        ui.label(if french { "Minute :" } else { "Minute:" });
                        let mut m = i32::from(draft.minute);
                        if ui.add(egui::DragValue::new(&mut m).range(0..=59)).changed() {
                            draft.minute = m as u8;
                        }
                    }
                    1 | 2 => {
                        ui.label(if french { "Heure :" } else { "Time (HH:MM):" });
                        let mut h = i32::from(draft.hour);
                        let mut m = i32::from(draft.minute);
                        if ui.add(egui::DragValue::new(&mut h).range(0..=23)).changed() {
                            draft.hour = h as u8;
                        }
                        ui.label(":");
                        if ui.add(egui::DragValue::new(&mut m).range(0..=59)).changed() {
                            draft.minute = m as u8;
                        }
                    }
                    3 => {
                        egui::ComboBox::from_id_salt("auto_weekday")
                            .selected_text(weekday_label(draft.weekday, french))
                            .show_ui(ui, |ui| {
                                for d in 0..=6 {
                                    ui.selectable_value(
                                        &mut draft.weekday,
                                        d,
                                        weekday_label(d, french),
                                    );
                                }
                            });
                        let mut h = i32::from(draft.hour);
                        let mut m = i32::from(draft.minute);
                        if ui.add(egui::DragValue::new(&mut h).range(0..=23)).changed() {
                            draft.hour = h as u8;
                        }
                        ui.label(":");
                        if ui.add(egui::DragValue::new(&mut m).range(0..=59)).changed() {
                            draft.minute = m as u8;
                        }
                    }
                    _ => {
                        ui.label("5-field cron (>=15m):");
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.cron_expr).desired_width(180.0),
                        );
                    }
                });
                ui.label(if french {
                    "Instructions (écrites dans prompt.md) :"
                } else {
                    "Prompt instructions (written safely to prompt.md):"
                });
                ui.add(
                    egui::TextEdit::multiline(&mut draft.prompt)
                        .desired_rows(4)
                        .desired_width(520.0),
                );
                if !draft.error.is_empty() {
                    ui.colored_label(ui.visuals().error_fg_color, &draft.error);
                }
                ui.horizontal(|ui| {
                    if ui
                        .button(if french { "Enregistrer" } else { "Save" })
                        .clicked()
                    {
                        let schedule = match draft.schedule_kind {
                            0 => AutomationSchedule::Hourly {
                                minute: draft.minute,
                            },
                            1 => AutomationSchedule::Daily {
                                hour: draft.hour,
                                minute: draft.minute,
                            },
                            2 => AutomationSchedule::Weekdays {
                                hour: draft.hour,
                                minute: draft.minute,
                            },
                            3 => AutomationSchedule::Weekly {
                                weekday: draft.weekday,
                                hour: draft.hour,
                                minute: draft.minute,
                            },
                            _ => AutomationSchedule::Cron {
                                expr: draft.cron_expr.trim().to_owned(),
                            },
                        };
                        if let Some((_, _, ppath)) =
                            projects.iter().find(|(id, ..)| *id == draft.project_id)
                        {
                            let mut candidate = Automation {
                                id: draft
                                    .id
                                    .clone()
                                    .unwrap_or_else(|| generate_automation_id("auto")),
                                name: draft.name.trim().to_owned(),
                                project_id: draft.project_id,
                                project_path: ppath.clone(),
                                agent: draft.agent,
                                isolation: draft.isolation,
                                schedule,
                                prompt: draft.prompt.clone(),
                                enabled: false,
                                confirmed_sig: None,
                                last_minute_slot: None,
                                runs: Vec::new(),
                            };
                            match candidate.validate() {
                                Ok(()) => {
                                    candidate.mark_confirmed();
                                    candidate.enabled = true;
                                    saved_auto = Some(candidate);
                                }
                                Err(e) => draft.error = e,
                            }
                        }
                    }
                    if ui
                        .button(if french { "Annuler" } else { "Cancel" })
                        .clicked()
                    {
                        cancel_form = true;
                    }
                });
            });
            if let Some(auto) = saved_auto {
                self.automation_draft = None;
                action = Some(SettingsExtraAction::UpsertAutomation(auto));
            } else if cancel_form {
                self.automation_draft = None;
            }
        }

        ui.add_space(10.0);
        section_heading(
            ui,
            if french {
                "Automatisations configurées"
            } else {
                "Configured automations"
            },
        );
        if store.automations.is_empty() {
            settings_card(ui, |ui| {
                ui.label(if french {
                    "Aucune automatisation enregistrée."
                } else {
                    "No automations configured yet."
                });
            });
        } else {
            for auto in &store.automations {
                let confirmed = auto.is_confirmed();
                settings_card(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong(&auto.name);
                        ui.small(format!(
                            "[{} · {}]",
                            auto.agent.label(),
                            auto.schedule.summary(french)
                        ));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(if french { "Supprimer" } else { "Delete" })
                                .clicked()
                            {
                                action =
                                    Some(SettingsExtraAction::DeleteAutomation(auto.id.clone()));
                            }
                            if confirmed {
                                if ui
                                    .button(if french {
                                        "Exécuter maintenant"
                                    } else {
                                        "Run now"
                                    })
                                    .clicked()
                                {
                                    action = Some(SettingsExtraAction::RunAutomationNow(
                                        auto.id.clone(),
                                    ));
                                }
                                let toggle_lbl = if auto.enabled {
                                    "Pause"
                                } else if french {
                                    "Activer"
                                } else {
                                    "Enable"
                                };
                                if ui.button(toggle_lbl).clicked() {
                                    let mut next = auto.clone();
                                    next.enabled = !next.enabled;
                                    action = Some(SettingsExtraAction::UpsertAutomation(next));
                                }
                            } else if self.pending_confirm_automation.as_deref()
                                == Some(auto.id.as_str())
                            {
                                if ui
                                    .button(if french {
                                        "Approuver et activer"
                                    } else {
                                        "Approve & enable"
                                    })
                                    .clicked()
                                {
                                    self.pending_confirm_automation = None;
                                    action = Some(SettingsExtraAction::ConfirmAndEnableAutomation(
                                        auto.id.clone(),
                                    ));
                                }
                            } else if ui
                                .button(if french {
                                    "Vérifier l'autorisation"
                                } else {
                                    "Review permission"
                                })
                                .clicked()
                            {
                                self.pending_confirm_automation = Some(auto.id.clone());
                            }
                        });
                    });
                    ui.small(format!(
                        "{} · {} {}",
                        auto.project_path.display(),
                        auto.runs.len(),
                        if french { "exécutions" } else { "runs" }
                    ));
                });
                ui.add_space(6.0);
            }
        }

        if !self.automations_status.is_empty() {
            ui.add_space(6.0);
            ui.small(&self.automations_status);
        }
        action
    }

    pub fn show_custom_agents_section(
        &mut self,
        ui: &mut egui::Ui,
        custom_agents: &[CustomAgent],
        french: bool,
    ) -> Option<SettingsExtraAction> {
        let mut action = None;
        ui.add_space(16.0);
        section_heading(
            ui,
            if french {
                "Agents CLI personnalisés"
            } else {
                "Custom CLI agents"
            },
        );

        settings_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.small(if french {
                    "Ajoutez vos propres exécutables ou scripts CLI dans le lanceur d'agents (jusqu'à 30)."
                } else {
                    "Add your own CLI executables or wrappers to the agent launcher (up to 30)."
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.custom_agent_form_open
                        && custom_agents.len() < MAX_CUSTOM_AGENTS
                        && ui
                            .button(if french {
                                "+ Ajouter un agent personnalisé"
                            } else {
                                "+ Add custom agent"
                            })
                            .clicked()
                    {
                        self.custom_agent_form_open = true;
                        self.editing_custom_id = None;
                        self.custom_name.clear();
                        self.custom_command.clear();
                        self.custom_accent = CUSTOM_AGENT_SWATCHES[0].to_owned();
                        self.custom_error.clear();
                    }
                });
            });

            if self.custom_agent_form_open {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(if french { "Nom :" } else { "Name:" });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.custom_name)
                            .hint_text("Aider / Custom Agent")
                            .desired_width(220.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(if french { "Commande :" } else { "Command:" });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.custom_command)
                            .hint_text("my-agent --flag")
                            .desired_width(320.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(if french { "Couleur :" } else { "Accent:" });
                    for &hex in &CUSTOM_AGENT_SWATCHES {
                        let col = parse_hex_rgb(hex)
                            .map_or(Color32::GRAY, |(r, g, b)| Color32::from_rgb(r, g, b));
                        let selected = self.custom_accent == hex;
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
                        ui.painter().circle_filled(rect.center(), 7.5, col);
                        if selected {
                            ui.painter().circle_stroke(
                                rect.center(),
                                8.5,
                                Stroke::new(1.5_f32, ui.visuals().text_color()),
                            );
                        }
                        if resp.clicked() {
                            self.custom_accent = hex.to_owned();
                        }
                    }
                });
                if !self.custom_error.is_empty() {
                    ui.colored_label(ui.visuals().error_fg_color, &self.custom_error);
                }
                ui.horizontal(|ui| {
                    if ui
                        .button(if french { "Enregistrer" } else { "Save" })
                        .clicked()
                    {
                        let id = self
                            .editing_custom_id
                            .clone()
                            .unwrap_or_else(generate_custom_agent_id);
                        match validate_custom_agent_fields(
                            &id,
                            &self.custom_name,
                            &self.custom_command,
                            &self.custom_accent,
                        ) {
                            Ok(()) => {
                                let candidate = CustomAgent {
                                    id,
                                    name: self.custom_name.trim().to_owned(),
                                    command: self.custom_command.trim().to_owned(),
                                    accent: self.custom_accent.clone(),
                                    enabled: true,
                                };
                                self.custom_agent_form_open = false;
                                self.editing_custom_id = None;
                                self.custom_error.clear();
                                action = Some(SettingsExtraAction::UpsertCustomAgent(candidate));
                            }
                            Err(e) => self.custom_error = e,
                        }
                    }
                    if ui
                        .button(if french { "Annuler" } else { "Cancel" })
                        .clicked()
                    {
                        self.custom_agent_form_open = false;
                        self.editing_custom_id = None;
                        self.custom_error.clear();
                    }
                });
            }

            if !custom_agents.is_empty() {
                ui.separator();
                for agent in custom_agents {
                    let (r, g, b) = agent.parse_accent_rgb();
                    ui.horizontal(|ui| {
                        let (dot_rect, _) =
                            ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                        ui.painter().circle_filled(
                            dot_rect.center(),
                            5.0,
                            Color32::from_rgb(r, g, b),
                        );
                        let mut en = agent.enabled;
                        if ui.checkbox(&mut en, &agent.name).changed() {
                            action = Some(SettingsExtraAction::ToggleCustomAgent {
                                id: agent.id.clone(),
                                enabled: en,
                            });
                        }
                        ui.small(RichText::new(&agent.command).monospace());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(if french { "Supprimer" } else { "Delete" })
                                .clicked()
                            {
                                action =
                                    Some(SettingsExtraAction::DeleteCustomAgent(agent.id.clone()));
                            }
                            if ui
                                .button(if french { "Modifier" } else { "Edit" })
                                .clicked()
                            {
                                self.custom_agent_form_open = true;
                                self.editing_custom_id = Some(agent.id.clone());
                                self.custom_name = agent.name.clone();
                                self.custom_command = agent.command.clone();
                                self.custom_accent = agent.accent.clone();
                                self.custom_error.clear();
                            }
                        });
                    });
                }
            }
        });

        action
    }
}

#[allow(clippy::too_many_arguments)]
fn render_managed_roster_card(
    ui: &mut egui::Ui,
    provider: &str,
    env_var_name: &str,
    roster: &ManagedAccountsRoster,
    system_email: Option<&str>,
    new_label_draft: &mut String,
    french: bool,
    action: &mut Option<SettingsExtraAction>,
) {
    ui.small(format!(
        "{} ({env_var_name})",
        if french {
            "Choisissez un profil isolé ou le compte système par défaut"
        } else {
            "Select an isolated managed profile or use the system default"
        }
    ));
    ui.add_space(4.0);
    if !crate::settings_extra_model::system_account_covered(system_email, roster) {
        let sys_selected = roster.selected_id.is_none();
        let sys_label = match system_email.map(str::trim).filter(|s| !s.is_empty()) {
            Some(addr) if french => format!("Par défaut système ({addr})"),
            Some(addr) => format!("System default ({addr})"),
            None if french => "Par défaut système".to_owned(),
            None => "System default".to_owned(),
        };
        if ui.radio(sys_selected, sys_label).clicked() && !sys_selected {
            *action = Some(SettingsExtraAction::SelectManagedAccount {
                provider: provider.to_owned(),
                account_id: None,
            });
        }
    }
    for acct in &roster.accounts {
        let selected = roster.selected_id.as_deref() == Some(acct.id.as_str());
        let display_label = if acct.email.trim().is_empty() {
            acct.label.clone()
        } else {
            format!("{} ({})", acct.label, acct.email.trim())
        };
        ui.horizontal(|ui| {
            if ui.radio(selected, &display_label).clicked() {
                *action = Some(SettingsExtraAction::SelectManagedAccount {
                    provider: provider.to_owned(),
                    account_id: if selected {
                        None
                    } else {
                        Some(acct.id.clone())
                    },
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(if french { "Retirer" } else { "Remove" })
                    .clicked()
                {
                    *action = Some(SettingsExtraAction::RemoveManagedAccount {
                        provider: provider.to_owned(),
                        account_id: acct.id.clone(),
                    });
                }
                if ui
                    .button(if french {
                        "Connecter dans un terminal"
                    } else {
                        "Sign in terminal"
                    })
                    .clicked()
                {
                    *action = Some(SettingsExtraAction::LaunchManagedAccountSignIn {
                        provider: provider.to_owned(),
                        account_id: acct.id.clone(),
                    });
                }
            });
        });
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(new_label_draft)
                .hint_text(if french {
                    "Nom du profil (ex. Travail, Personnel)"
                } else {
                    "Profile label (e.g. Work, Personal)"
                })
                .desired_width(220.0),
        );
        if ui
            .add_enabled(
                !new_label_draft.trim().is_empty(),
                egui::Button::new(if french {
                    "Ajouter un profil"
                } else {
                    "Add profile"
                }),
            )
            .clicked()
        {
            let lbl = new_label_draft.trim().to_owned();
            new_label_draft.clear();
            *action = Some(SettingsExtraAction::AddManagedAccount {
                provider: provider.to_owned(),
                label: lbl,
            });
        }
    });
}

fn render_context_group(
    ui: &mut egui::Ui,
    heading: &str,
    items: &[crate::settings_extra_model::ContextItem],
    french: bool,
) {
    let group_tokens: usize = items.iter().map(|i| i.estimated_tokens).sum();
    settings_card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.strong(heading);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.small(format!("~{group_tokens} tok"));
            });
        });
        if items.is_empty() {
            ui.small(if french { "Aucun élément." } else { "None." });
        } else {
            for item in items.iter().take(12) {
                ui.horizontal(|ui| {
                    ui.small(&item.label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.small(format!("~{} tok", item.estimated_tokens));
                    });
                });
            }
        }
    });
    ui.add_space(6.0);
}
