//! Pure data model and exact behavioral parity logic for `PaneActionsMenu.vue`
//! (`C:\Tessel\src\renderer\src\components\PaneActionsMenu.vue`), shared identically
//! between terminal headers and native chat headers.
//!
//! Zero dummy items: every menu action, chip row, and fact line is emitted only when
//! its real backing capability is present, preserving exact EN/FR strings and order.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaneMenuLang {
    #[default]
    En,
    Fr,
}

// ---------------------------------------------------------------------------
// 1. Header Facts Block (`PaneActionsMenu.vue:30-62`, `96-98`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneAgentActivity {
    Idle,
    Busy,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuietLevel {
    Mild,
    Long,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuietFactInput {
    pub minutes: u32,
    pub level: QuietLevel,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneFactKind {
    Branch,
    Team,
    Account,
    StartedYolo,
    State,
    Quiet,
    PromptCache,
    ModelReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneMenuFact {
    pub kind: PaneFactKind,
    pub label: String,
    pub value: String,
    pub tooltip: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentStateFactInput<'a> {
    pub sleeping: bool,
    pub exited: bool,
    pub asks_approval: bool,
    pub limit_reset: Option<Option<&'a str>>,
    pub activity: PaneAgentActivity,
    pub estimated_state: bool,
    pub needs_you: bool,
}

#[must_use]
pub fn format_agent_state_fact(state: &AgentStateFactInput<'_>, lang: PaneMenuLang) -> String {
    if state.sleeping {
        return match lang {
            PaneMenuLang::En => "asleep".to_string(),
            PaneMenuLang::Fr => "en veille".to_string(),
        };
    }
    if state.exited {
        return match lang {
            PaneMenuLang::En => "exited".to_string(),
            PaneMenuLang::Fr => "terminé".to_string(),
        };
    }
    if state.asks_approval {
        return match lang {
            PaneMenuLang::En => "asks you to approve".to_string(),
            PaneMenuLang::Fr => "vous demande d'approuver".to_string(),
        };
    }
    if let Some(reset_opt) = state.limit_reset {
        return match (reset_opt, lang) {
            (Some(when), PaneMenuLang::En) if !when.is_empty() => {
                format!("usage limit · resets {when}")
            }
            (Some(when), PaneMenuLang::Fr) if !when.is_empty() => {
                format!("limite d'utilisation · réinitialisation {when}")
            }
            (_, PaneMenuLang::En) => "usage limit".to_string(),
            (_, PaneMenuLang::Fr) => "limite d'utilisation".to_string(),
        };
    }
    if state.activity == PaneAgentActivity::Busy {
        return match (state.estimated_state, lang) {
            (true, PaneMenuLang::En) => "working (estimated)".to_string(),
            (true, PaneMenuLang::Fr) => "au travail (estimé)".to_string(),
            (false, PaneMenuLang::En) => "working".to_string(),
            (false, PaneMenuLang::Fr) => "au travail".to_string(),
        };
    }
    if state.activity == PaneAgentActivity::Unknown {
        return match lang {
            PaneMenuLang::En => "unknown".to_string(),
            PaneMenuLang::Fr => "inconnu".to_string(),
        };
    }
    if state.needs_you {
        return match lang {
            PaneMenuLang::En => "done, needs you".to_string(),
            PaneMenuLang::Fr => "terminé, vous attend".to_string(),
        };
    }
    match lang {
        PaneMenuLang::En => "idle".to_string(),
        PaneMenuLang::Fr => "inactif".to_string(),
    }
}

// ---------------------------------------------------------------------------
// 2. Menu Actions, Chips & Inputs (`PaneActionsMenu.vue:63-299`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveShareTextSize {
    Fit,
    Width,
    Normal,
}

impl LiveShareTextSize {
    #[must_use]
    pub const fn label(self, lang: PaneMenuLang) -> &'static str {
        match (self, lang) {
            (Self::Fit, PaneMenuLang::En) => "Fit",
            (Self::Fit, PaneMenuLang::Fr) => "Ajustée",
            (Self::Width, PaneMenuLang::En) => "Width",
            (Self::Width, PaneMenuLang::Fr) => "Largeur",
            (Self::Normal, PaneMenuLang::En) => "Normal",
            (Self::Normal, PaneMenuLang::Fr) => "Normale",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceLanguageOption {
    pub tip: String,
    pub tag: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherPaneTarget {
    pub id: String,
    pub title: String,
    pub where_hint: String,
    pub is_agent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRestartChoice {
    pub id: Option<String>,
    pub name: String,
    pub title_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamMembership {
    pub id: String,
    pub name: String,
    pub is_lead: bool,
    pub can_set_lead: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneMenuAction {
    ResolveUnsent,
    RestartApplySettings,
    Copy,
    Paste,
    CopyOutput,
    Clear,
    ResetTerminal,
    CopySessionId,
    Find,
    ChooseModel,
    VoiceTyping,
    SendSelectionTo(String),
    AskReview(String),
    ToggleTeamLead(String),
    LeaveTeam(String),
    Rename,
    ToggleMaximize,
    ArrangeGrid,
    OpenHere,
    SplitRight,
    SplitDown,
    Restart,
    RestartOnAccount(Option<String>),
    SignInOnHost,
    MakeHostDefault,
    CheckHostAccount,
    CopyAccountDetails,
    ToggleTranscript,
    OpenOrSwitchChat,
    ContinueOnHostChat,
    SwitchYolo,
    ToggleYoloFolder,
    ClosePane,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneMenuChip {
    pub id: String,
    pub label: String,
    pub selected: bool,
    pub tooltip: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneMenuEntry {
    Separator,
    SectionLabel(String),
    Item {
        action: PaneMenuAction,
        label: String,
        shortcut: Option<String>,
        tooltip: Option<String>,
        enabled: bool,
        danger: bool,
    },
    LiveShareTextSizeRow {
        label: String,
        chips: Vec<PaneMenuChip>,
    },
    VoiceLanguageRow {
        label: String,
        chips: Vec<PaneMenuChip>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct PaneActionsMenuInput {
    pub pane_title: String,
    pub shell_name: String,
    pub worktree: Option<(String, String)>, // (branch, path)
    pub team: Option<TeamMembership>,
    pub is_agent: bool,
    pub is_chat: bool,
    pub is_floating: bool,
    pub is_maximized: bool,
    pub can_arrange_panes: bool,
    pub account_info: Option<(String, String)>, // (name, tooltip)
    pub launch_yolo: bool,
    pub sleeping: bool,
    pub exited: bool,
    pub asks_approval: bool,
    pub limit_reset: Option<Option<String>>,
    pub activity: Option<PaneAgentActivity>,
    pub estimated_state: bool,
    pub needs_you: bool,
    pub status_title: String,
    pub quiet: Option<QuietFactInput>,
    pub prompt_cache: Option<(String, String)>, // (left_text, tooltip)
    pub unsent: bool,
    pub launch_stale: bool,
    pub has_selection: bool,
    pub session_id: Option<String>,
    pub has_model_choice: bool,
    pub model_text: Option<String>,
    pub model_title: Option<String>,
    pub ls_text_size: Option<(LiveShareTextSize, String)>, // (selected, host_name)
    pub voice_language_name: String,
    pub voice_shortcut_label: Option<String>,
    pub voice_languages: Vec<VoiceLanguageOption>,
    pub selected_voice_tip: String,
    pub other_panes: Vec<OtherPaneTarget>,
    pub account_restart_choices: Vec<AccountRestartChoice>,
    pub host_name: Option<String>,
    pub can_sign_in_on_host: bool,
    pub can_make_host_default: bool,
    pub can_check_host_account: bool,
    pub can_view_transcript: bool,
    pub transcript_open: bool,
    pub can_open_as_chat: bool,
    pub chat_view_available: bool,
    pub chat_view_open: bool,
    pub can_continue_on_host: bool,
    pub continue_on_host_reason: Option<String>,
    pub can_switch_yolo: bool,
    pub yolo_folder: Option<(String, bool)>, // (folder_path, enabled_on)
    pub disabled_reasons: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltPaneActionsMenu {
    pub title: String,
    pub subtitle: String,
    pub facts: Vec<PaneMenuFact>,
    pub entries: Vec<PaneMenuEntry>,
}

fn folder_basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    if name.is_empty() { path } else { name }
}

#[must_use]
pub fn open_or_switch_chat_label(
    is_chat: bool,
    chat_view_available: bool,
    chat_view_open: bool,
    lang: PaneMenuLang,
) -> &'static str {
    if is_chat {
        return match lang {
            PaneMenuLang::En => "Continue in a terminal",
            PaneMenuLang::Fr => "Continuer dans un terminal",
        };
    }
    if chat_view_available {
        return match (chat_view_open, lang) {
            (true, PaneMenuLang::En) => "Switch to terminal view",
            (true, PaneMenuLang::Fr) => "Passer en vue terminal",
            (false, PaneMenuLang::En) => "Switch to chat view",
            (false, PaneMenuLang::Fr) => "Passer en vue chat",
        };
    }
    match lang {
        PaneMenuLang::En => "Open as chat",
        PaneMenuLang::Fr => "Ouvrir en chat",
    }
}

#[must_use]
pub fn build_pane_actions_menu(
    input: &PaneActionsMenuInput,
    lang: PaneMenuLang,
) -> BuiltPaneActionsMenu {
    let mut facts: Vec<PaneMenuFact> = Vec::new();

    if let Some((branch, path)) = &input.worktree {
        let (lbl, tip) = match lang {
            PaneMenuLang::En => (
                "Branch",
                format!("Separate copy on branch {branch}\n{path}"),
            ),
            PaneMenuLang::Fr => (
                "Branche",
                format!("Copie séparée sur la branche {branch}\n{path}"),
            ),
        };
        facts.push(PaneMenuFact {
            kind: PaneFactKind::Branch,
            label: lbl.to_string(),
            value: branch.clone(),
            tooltip: tip,
        });
    }

    if let Some(team) = &input.team {
        let (lbl, lead_word, tip) = match (team.is_lead, lang) {
            (true, PaneMenuLang::En) => (
                "Team",
                " · lead",
                format!(
                    "Team: {} (this agent leads it) (manage in Sessions)",
                    team.name
                ),
            ),
            (false, PaneMenuLang::En) => (
                "Team",
                "",
                format!("Team: {} (manage in Sessions)", team.name),
            ),
            (true, PaneMenuLang::Fr) => (
                "Équipe",
                " · chef",
                format!(
                    "Équipe : {} (cet agent la dirige) (gérez-la dans Sessions)",
                    team.name
                ),
            ),
            (false, PaneMenuLang::Fr) => (
                "Équipe",
                "",
                format!("Équipe : {} (gérez-la dans Sessions)", team.name),
            ),
        };
        facts.push(PaneMenuFact {
            kind: PaneFactKind::Team,
            label: lbl.to_string(),
            value: format!("{}{lead_word}", team.name),
            tooltip: tip,
        });
    }

    if input.is_agent
        && let Some((acc_name, acc_tip)) = &input.account_info
    {
        facts.push(PaneMenuFact {
            kind: PaneFactKind::Account,
            label: match lang {
                PaneMenuLang::En => "Account".to_string(),
                PaneMenuLang::Fr => "Compte".to_string(),
            },
            value: acc_name.clone(),
            tooltip: acc_tip.clone(),
        });
    }

    if input.is_agent && input.launch_yolo {
        facts.push(PaneMenuFact {
            kind: PaneFactKind::StartedYolo,
            label: match lang {
                PaneMenuLang::En => "Started".to_string(),
                PaneMenuLang::Fr => "Démarré".to_string(),
            },
            value: "Yolo".to_string(),
            tooltip: match lang {
                PaneMenuLang::En => {
                    "Started in Yolo: this agent runs commands and changes files without asking you"
                        .to_string()
                }
                PaneMenuLang::Fr => {
                    "Démarré en Yolo : cet agent exécute des commandes et modifie des fichiers sans vous demander"
                        .to_string()
                }
            },
        });
    }

    if input.is_agent {
        let limit_ref = input.limit_reset.as_ref().map(|o| o.as_deref());
        let val = format_agent_state_fact(
            &AgentStateFactInput {
                sleeping: input.sleeping,
                exited: input.exited,
                asks_approval: input.asks_approval,
                limit_reset: limit_ref,
                activity: input.activity.unwrap_or(PaneAgentActivity::Idle),
                estimated_state: input.estimated_state,
                needs_you: input.needs_you,
            },
            lang,
        );
        facts.push(PaneMenuFact {
            kind: PaneFactKind::State,
            label: match lang {
                PaneMenuLang::En => "State".to_string(),
                PaneMenuLang::Fr => "État".to_string(),
            },
            value: val,
            tooltip: input.status_title.clone(),
        });
    }

    if let Some(q) = &input.quiet {
        let level_tag = match q.level {
            QuietLevel::Mild => "mild",
            QuietLevel::Long => "long",
        };
        facts.push(PaneMenuFact {
            kind: PaneFactKind::Quiet,
            label: match lang {
                PaneMenuLang::En => "Quiet".to_string(),
                PaneMenuLang::Fr => "Silencieux".to_string(),
            },
            value: format!("{} min", q.minutes),
            tooltip: if q.reason.is_empty() {
                level_tag.to_string()
            } else {
                q.reason.clone()
            },
        });
    }

    if let Some((cache_left, cache_tip)) = &input.prompt_cache {
        facts.push(PaneMenuFact {
            kind: PaneFactKind::PromptCache,
            label: match lang {
                PaneMenuLang::En => "Prompt cache".to_string(),
                PaneMenuLang::Fr => "Cache de prompt".to_string(),
            },
            value: cache_left.clone(),
            tooltip: cache_tip.clone(),
        });
    }

    if !input.has_model_choice
        && input.is_agent
        && let Some(model_txt) = &input.model_text
        && !model_txt.is_empty()
    {
        facts.push(PaneMenuFact {
            kind: PaneFactKind::ModelReadOnly,
            label: match lang {
                PaneMenuLang::En => "Model".to_string(),
                PaneMenuLang::Fr => "Modèle".to_string(),
            },
            value: model_txt.clone(),
            tooltip: input.model_title.clone().unwrap_or_default(),
        });
    }

    let mut entries: Vec<PaneMenuEntry> = Vec::new();
    let unavailable = |key: &str| -> Option<String> {
        input
            .disabled_reasons
            .get(key)
            .filter(|s| !s.is_empty())
            .cloned()
    };
    let push_item = |entries: &mut Vec<PaneMenuEntry>,
                     action: PaneMenuAction,
                     label: String,
                     shortcut: Option<String>,
                     tooltip: Option<String>,
                     enabled: bool,
                     danger: bool| {
        entries.push(PaneMenuEntry::Item {
            action,
            label,
            shortcut,
            tooltip,
            enabled,
            danger,
        });
    };

    if input.unsent {
        let (lbl, tip) = match lang {
            PaneMenuLang::En => (
                "Message not confirmed…",
                "A message was pasted but not seen taken",
            ),
            PaneMenuLang::Fr => (
                "Message non confirmé…",
                "Un message a été collé mais sa prise en compte n'a pas été vue",
            ),
        };
        push_item(
            &mut entries,
            PaneMenuAction::ResolveUnsent,
            lbl.to_string(),
            None,
            Some(tip.to_string()),
            true,
            false,
        );
    }
    if input.launch_stale {
        let lbl = match lang {
            PaneMenuLang::En => "Restart to apply settings",
            PaneMenuLang::Fr => "Redémarrer pour appliquer les paramètres",
        };
        push_item(
            &mut entries,
            PaneMenuAction::RestartApplySettings,
            lbl.to_string(),
            None,
            None,
            true,
            false,
        );
    }

    entries.push(PaneMenuEntry::Separator);
    push_item(
        &mut entries,
        PaneMenuAction::Copy,
        match lang {
            PaneMenuLang::En => "Copy".to_string(),
            PaneMenuLang::Fr => "Copier".to_string(),
        },
        None,
        None,
        input.has_selection,
        false,
    );
    push_item(
        &mut entries,
        PaneMenuAction::Paste,
        match lang {
            PaneMenuLang::En => "Paste".to_string(),
            PaneMenuLang::Fr => "Coller".to_string(),
        },
        None,
        None,
        true,
        false,
    );

    entries.push(PaneMenuEntry::Separator);
    let dis_copy_out = unavailable("menuCopyOutput");
    push_item(
        &mut entries,
        PaneMenuAction::CopyOutput,
        match lang {
            PaneMenuLang::En => "Copy output".to_string(),
            PaneMenuLang::Fr => "Copier la sortie".to_string(),
        },
        None,
        dis_copy_out.clone(),
        dis_copy_out.is_none(),
        false,
    );
    let dis_clear = unavailable("menuClear");
    push_item(
        &mut entries,
        PaneMenuAction::Clear,
        match lang {
            PaneMenuLang::En => "Clear".to_string(),
            PaneMenuLang::Fr => "Effacer".to_string(),
        },
        None,
        dis_clear.clone(),
        dis_clear.is_none(),
        false,
    );
    let dis_reset = unavailable("menuResetTerminal");
    let reset_tip = dis_reset.clone().or_else(|| match lang {
        PaneMenuLang::En => Some(
            "Turn off the mouse, paste and key modes a program left on. The program keeps running"
                .to_string(),
        ),
        PaneMenuLang::Fr => Some(
            "Désactiver les modes souris, collage et touches qu'un programme a laissés actifs. Le programme continue de tourner"
                .to_string(),
        ),
    });
    push_item(
        &mut entries,
        PaneMenuAction::ResetTerminal,
        match lang {
            PaneMenuLang::En => "Reset Terminal".to_string(),
            PaneMenuLang::Fr => "Réinitialiser le terminal".to_string(),
        },
        None,
        reset_tip,
        dis_reset.is_none(),
        false,
    );

    if let Some(sid) = &input.session_id
        && !sid.is_empty()
    {
        let short: String = sid.chars().take(8).collect();
        push_item(
            &mut entries,
            PaneMenuAction::CopySessionId,
            match lang {
                PaneMenuLang::En => "Copy session ID".to_string(),
                PaneMenuLang::Fr => "Copier l'ID de session".to_string(),
            },
            Some(short),
            None,
            true,
            false,
        );
    }

    let dis_find = unavailable("menuFind");
    push_item(
        &mut entries,
        PaneMenuAction::Find,
        match lang {
            PaneMenuLang::En => "Find".to_string(),
            PaneMenuLang::Fr => "Rechercher".to_string(),
        },
        Some("Ctrl+Shift+F".to_string()),
        dis_find.clone(),
        dis_find.is_none(),
        false,
    );

    if input.has_model_choice {
        let dis_model = unavailable("menuModel");
        let tip = dis_model
            .clone()
            .or_else(|| input.model_title.clone())
            .or_else(|| match lang {
                PaneMenuLang::En => Some("Choose the model this agent uses".to_string()),
                PaneMenuLang::Fr => Some("Choisir le modèle utilisé par cet agent".to_string()),
            });
        push_item(
            &mut entries,
            PaneMenuAction::ChooseModel,
            match lang {
                PaneMenuLang::En => "Model…".to_string(),
                PaneMenuLang::Fr => "Modèle…".to_string(),
            },
            input.model_text.clone(),
            tip,
            dis_model.is_none(),
            false,
        );
    }

    if let Some((selected_ls, ls_host)) = &input.ls_text_size {
        let host_disp = if ls_host.is_empty() {
            "?"
        } else {
            ls_host.as_str()
        };
        let chips = [
            LiveShareTextSize::Fit,
            LiveShareTextSize::Width,
            LiveShareTextSize::Normal,
        ]
        .into_iter()
        .map(|mode| {
            let tip = match (mode, lang) {
                (LiveShareTextSize::Fit, PaneMenuLang::En) => format!(
                    "All of {host_disp}’s terminal in this pane: larger or smaller text, nothing cut. Only on your screen"
                ),
                (LiveShareTextSize::Fit, PaneMenuLang::Fr) => format!(
                    "Tout le terminal de {host_disp} dans ce panneau : texte plus grand ou plus petit, rien de coupé. Seulement sur votre écran"
                ),
                (LiveShareTextSize::Width, PaneMenuLang::En) => {
                    "As large as its columns allow in this pane’s width: rows that do not fit scroll, following the cursor. Only on your screen".to_string()
                }
                (LiveShareTextSize::Width, PaneMenuLang::Fr) => {
                    "Aussi grand que ses colonnes le permettent dans la largeur du panneau : les lignes qui ne tiennent pas défilent, en suivant le curseur. Seulement sur votre écran".to_string()
                }
                (LiveShareTextSize::Normal, PaneMenuLang::En) => {
                    "Your usual text size: a larger terminal scrolls. Only on your screen".to_string()
                }
                (LiveShareTextSize::Normal, PaneMenuLang::Fr) => {
                    "Votre taille de texte habituelle : un terminal plus grand défile. Seulement sur votre écran".to_string()
                }
            };
            PaneMenuChip {
                id: match mode {
                    LiveShareTextSize::Fit => "fit".to_string(),
                    LiveShareTextSize::Width => "width".to_string(),
                    LiveShareTextSize::Normal => "normal".to_string(),
                },
                label: mode.label(lang).to_string(),
                selected: *selected_ls == mode,
                tooltip: tip,
            }
        })
        .collect();
        entries.push(PaneMenuEntry::LiveShareTextSizeRow {
            label: match lang {
                PaneMenuLang::En => "Text size".to_string(),
                PaneMenuLang::Fr => "Taille du texte".to_string(),
            },
            chips,
        });
    }

    entries.push(PaneMenuEntry::Separator);
    let voice_lang = if input.voice_language_name.is_empty() {
        match lang {
            PaneMenuLang::En => "current keyboard language",
            PaneMenuLang::Fr => "langue actuelle du clavier",
        }
    } else {
        input.voice_language_name.as_str()
    };
    push_item(
        &mut entries,
        PaneMenuAction::VoiceTyping,
        match lang {
            PaneMenuLang::En => "Voice typing".to_string(),
            PaneMenuLang::Fr => "Saisie vocale".to_string(),
        },
        Some(
            input
                .voice_shortcut_label
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Win+H".to_string()),
        ),
        Some(match lang {
            PaneMenuLang::En => {
                format!("Speak instead of typing ({voice_lang}). Windows voice typing")
            }
            PaneMenuLang::Fr => {
                format!("Parlez au lieu de taper ({voice_lang}). Saisie vocale Windows")
            }
        }),
        true,
        false,
    );

    if !input.voice_languages.is_empty() {
        let mut chips: Vec<PaneMenuChip> = input
            .voice_languages
            .iter()
            .map(|l| PaneMenuChip {
                id: l.tip.clone(),
                label: l.tag.chars().take(2).collect::<String>().to_uppercase(),
                selected: input.selected_voice_tip == l.tip,
                tooltip: match lang {
                    PaneMenuLang::En => format!("Speak in {}", l.name),
                    PaneMenuLang::Fr => format!("Parler en {}", l.name),
                },
            })
            .collect();
        chips.push(PaneMenuChip {
            id: String::new(),
            label: "⌨".to_string(),
            selected: input.selected_voice_tip.is_empty(),
            tooltip: match lang {
                PaneMenuLang::En => "Current keyboard language".to_string(),
                PaneMenuLang::Fr => "Langue actuelle du clavier".to_string(),
            },
        });
        entries.push(PaneMenuEntry::VoiceLanguageRow {
            label: match lang {
                PaneMenuLang::En => "Speak in".to_string(),
                PaneMenuLang::Fr => "Parler en".to_string(),
            },
            chips,
        });
    }

    entries.push(PaneMenuEntry::Separator);

    if !input.other_panes.is_empty() {
        entries.push(PaneMenuEntry::SectionLabel(match lang {
            PaneMenuLang::En => "Send selection to".to_string(),
            PaneMenuLang::Fr => "Envoyer la sélection à".to_string(),
        }));
        for p in &input.other_panes {
            push_item(
                &mut entries,
                PaneMenuAction::SendSelectionTo(p.id.clone()),
                p.title.clone(),
                (!p.where_hint.is_empty()).then(|| p.where_hint.clone()),
                None,
                input.has_selection,
                false,
            );
        }
        if input.other_panes.iter().any(|p| p.is_agent) {
            entries.push(PaneMenuEntry::SectionLabel(match lang {
                PaneMenuLang::En => "Ask to review this pane's changes".to_string(),
                PaneMenuLang::Fr => "Demander de réviser les changements de ce panneau".to_string(),
            }));
            for p in input.other_panes.iter().filter(|p| p.is_agent) {
                push_item(
                    &mut entries,
                    PaneMenuAction::AskReview(p.id.clone()),
                    p.title.clone(),
                    (!p.where_hint.is_empty()).then(|| p.where_hint.clone()),
                    None,
                    true,
                    false,
                );
            }
        }
        entries.push(PaneMenuEntry::Separator);
    }

    if let Some(team) = &input.team {
        if input.is_agent && team.can_set_lead {
            let lead_lbl = match (team.is_lead, lang) {
                (true, PaneMenuLang::En) => format!("Stop leading {}", team.name),
                (false, PaneMenuLang::En) => format!("Make lead of {}", team.name),
                (true, PaneMenuLang::Fr) => format!("Ne plus diriger {}", team.name),
                (false, PaneMenuLang::Fr) => format!("Nommer chef de {}", team.name),
            };
            push_item(
                &mut entries,
                PaneMenuAction::ToggleTeamLead(team.id.clone()),
                lead_lbl,
                None,
                None,
                true,
                false,
            );
        }
        let leave_lbl = match lang {
            PaneMenuLang::En => format!("Leave {}", team.name),
            PaneMenuLang::Fr => format!("Quitter {}", team.name),
        };
        push_item(
            &mut entries,
            PaneMenuAction::LeaveTeam(team.id.clone()),
            leave_lbl,
            None,
            None,
            true,
            false,
        );
        entries.push(PaneMenuEntry::Separator);
    }

    push_item(
        &mut entries,
        PaneMenuAction::Rename,
        match lang {
            PaneMenuLang::En => "Rename".to_string(),
            PaneMenuLang::Fr => "Renommer".to_string(),
        },
        None,
        None,
        true,
        false,
    );

    if !input.is_floating {
        push_item(
            &mut entries,
            PaneMenuAction::ToggleMaximize,
            match (input.is_maximized, lang) {
                (true, PaneMenuLang::En) => "Restore pane".to_string(),
                (false, PaneMenuLang::En) => "Maximize pane".to_string(),
                (true, PaneMenuLang::Fr) => "Restaurer le panneau".to_string(),
                (false, PaneMenuLang::Fr) => "Agrandir le panneau".to_string(),
            },
            None,
            None,
            true,
            false,
        );
        if input.can_arrange_panes {
            push_item(
                &mut entries,
                PaneMenuAction::ArrangeGrid,
                match lang {
                    PaneMenuLang::En => "Arrange panes in a grid".to_string(),
                    PaneMenuLang::Fr => "Organiser les panneaux en grille".to_string(),
                },
                None,
                None,
                true,
                false,
            );
        }
        push_item(
            &mut entries,
            PaneMenuAction::OpenHere,
            match lang {
                PaneMenuLang::En => "Open terminal or agent here…".to_string(),
                PaneMenuLang::Fr => "Ouvrir un terminal ou un agent ici…".to_string(),
            },
            None,
            None,
            true,
            false,
        );
        push_item(
            &mut entries,
            PaneMenuAction::SplitRight,
            match lang {
                PaneMenuLang::En => "Split right".to_string(),
                PaneMenuLang::Fr => "Diviser à droite".to_string(),
            },
            Some("▥".to_string()),
            None,
            true,
            false,
        );
        push_item(
            &mut entries,
            PaneMenuAction::SplitDown,
            match lang {
                PaneMenuLang::En => "Split down".to_string(),
                PaneMenuLang::Fr => "Diviser vers le bas".to_string(),
            },
            Some("▤".to_string()),
            None,
            true,
            false,
        );
    }

    entries.push(PaneMenuEntry::Separator);
    push_item(
        &mut entries,
        PaneMenuAction::Restart,
        match lang {
            PaneMenuLang::En => "Restart".to_string(),
            PaneMenuLang::Fr => "Redémarrer".to_string(),
        },
        Some("Ctrl+Shift+R".to_string()),
        None,
        true,
        false,
    );

    for choice in &input.account_restart_choices {
        let lbl = match lang {
            PaneMenuLang::En => format!("Restart on {}", choice.name),
            PaneMenuLang::Fr => format!("Redémarrer sur {}", choice.name),
        };
        let tip = choice.title_hint.clone().unwrap_or_else(|| match lang {
            PaneMenuLang::En => format!(
                "Restart this agent on {}: same pane, its conversation resumed",
                choice.name
            ),
            PaneMenuLang::Fr => format!(
                "Redémarrer cet agent sur {} : même panneau, sa conversation reprise",
                choice.name
            ),
        });
        push_item(
            &mut entries,
            PaneMenuAction::RestartOnAccount(choice.id.clone()),
            lbl,
            None,
            Some(tip),
            true,
            false,
        );
    }

    let host = input.host_name.as_deref().unwrap_or("SSH");
    if input.can_sign_in_on_host {
        push_item(
            &mut entries,
            PaneMenuAction::SignInOnHost,
            match lang {
                PaneMenuLang::En => format!("Sign in to another account on {host}…"),
                PaneMenuLang::Fr => format!("Se connecter à un autre compte sur {host}…"),
            },
            None,
            None,
            true,
            false,
        );
    }
    if input.can_make_host_default {
        push_item(
            &mut entries,
            PaneMenuAction::MakeHostDefault,
            match lang {
                PaneMenuLang::En => format!("Make this the default on {host}"),
                PaneMenuLang::Fr => format!("En faire le compte par défaut sur {host}"),
            },
            None,
            None,
            true,
            false,
        );
    }
    if input.can_check_host_account {
        push_item(
            &mut entries,
            PaneMenuAction::CheckHostAccount,
            match lang {
                PaneMenuLang::En => "Check account again".to_string(),
                PaneMenuLang::Fr => "Vérifier le compte de nouveau".to_string(),
            },
            None,
            None,
            true,
            false,
        );
        push_item(
            &mut entries,
            PaneMenuAction::CopyAccountDetails,
            match lang {
                PaneMenuLang::En => "Copy account details".to_string(),
                PaneMenuLang::Fr => "Copier les détails du compte".to_string(),
            },
            None,
            None,
            true,
            false,
        );
    }

    if input.can_view_transcript {
        push_item(
            &mut entries,
            PaneMenuAction::ToggleTranscript,
            match (input.transcript_open, lang) {
                (true, PaneMenuLang::En) => "Back to the terminal".to_string(),
                (false, PaneMenuLang::En) => "See the conversation".to_string(),
                (true, PaneMenuLang::Fr) => "Revenir au terminal".to_string(),
                (false, PaneMenuLang::Fr) => "Voir la conversation".to_string(),
            },
            None,
            None,
            true,
            false,
        );
    }

    if input.can_open_as_chat {
        let dis_chat = unavailable("menuOpenAsChat");
        let lbl = open_or_switch_chat_label(
            input.is_chat,
            input.chat_view_available,
            input.chat_view_open,
            lang,
        );
        push_item(
            &mut entries,
            PaneMenuAction::OpenOrSwitchChat,
            lbl.to_string(),
            None,
            dis_chat.clone(),
            dis_chat.is_none(),
            false,
        );
    }

    if input.can_continue_on_host && !input.is_chat {
        let reason = input
            .continue_on_host_reason
            .clone()
            .filter(|s| !s.is_empty());
        push_item(
            &mut entries,
            PaneMenuAction::ContinueOnHostChat,
            match lang {
                PaneMenuLang::En => "Continue in a chat pane (on the host)".to_string(),
                PaneMenuLang::Fr => "Continuer dans un panneau de chat (sur l’hôte)".to_string(),
            },
            None,
            reason.clone(),
            reason.is_none(),
            false,
        );
    }

    if input.can_switch_yolo {
        let dis_yolo = unavailable("menuSwitchYolo");
        push_item(
            &mut entries,
            PaneMenuAction::SwitchYolo,
            match (input.launch_yolo, lang) {
                (true, PaneMenuLang::En) => "Restart asking first".to_string(),
                (false, PaneMenuLang::En) => "Restart in Yolo".to_string(),
                (true, PaneMenuLang::Fr) => "Redémarrer en demandant d'abord".to_string(),
                (false, PaneMenuLang::Fr) => "Redémarrer en Yolo".to_string(),
            },
            None,
            dis_yolo.clone(),
            dis_yolo.is_none(),
            false,
        );
        if let Some((folder_path, on)) = &input.yolo_folder
            && !folder_path.is_empty()
        {
            let fname = folder_basename(folder_path);
            let badge = if *on {
                format!("✓ {fname}")
            } else {
                fname.to_string()
            };
            push_item(
                &mut entries,
                PaneMenuAction::ToggleYoloFolder,
                match lang {
                    PaneMenuLang::En => "Yolo in this folder".to_string(),
                    PaneMenuLang::Fr => "Yolo dans ce dossier".to_string(),
                },
                Some(badge),
                None,
                true,
                false,
            );
        }
    }

    push_item(
        &mut entries,
        PaneMenuAction::ClosePane,
        match lang {
            PaneMenuLang::En => "Close pane".to_string(),
            PaneMenuLang::Fr => "Fermer le panneau".to_string(),
        },
        None,
        None,
        true,
        true,
    );

    BuiltPaneActionsMenu {
        title: input.pane_title.clone(),
        subtitle: input.shell_name.clone(),
        facts,
        entries,
    }
}
