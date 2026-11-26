//! Pure data models and exact behavioral parity logic for:
//! - Settings dialog page & 5-group hierarchy (`C:\Tessel\src\renderer\src\components\settings\settingsPages.js`)
//! - Page alias resolution (`text` -> `terminal#set-terminal-font`), anchor validation, MCP sub-tab
//!   state, and multi-word EN/FR settings search (`C:\Tessel\src\renderer\src\components\SettingsDialog.vue`)

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsLang {
    #[default]
    En,
    Fr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsGroupId {
    Ai,
    Workspace,
    Workflows,
    Insights,
    About,
}

pub const ALL_SETTINGS_GROUPS: [SettingsGroupId; 5] = [
    SettingsGroupId::Ai,
    SettingsGroupId::Workspace,
    SettingsGroupId::Workflows,
    SettingsGroupId::Insights,
    SettingsGroupId::About,
];

impl SettingsGroupId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ai => "ai",
            Self::Workspace => "workspace",
            Self::Workflows => "workflows",
            Self::Insights => "insights",
            Self::About => "about",
        }
    }

    #[must_use]
    pub const fn title(self, lang: SettingsLang) -> &'static str {
        match (self, lang) {
            (Self::Ai, SettingsLang::En) => "AI",
            (Self::Ai, SettingsLang::Fr) => "IA",
            (Self::Workspace, SettingsLang::En) => "Workspace",
            (Self::Workspace, SettingsLang::Fr) => "Espace de travail",
            (Self::Workflows, SettingsLang::En | SettingsLang::Fr) => "Workflows",
            (Self::Insights, SettingsLang::En) => "Insights",
            (Self::Insights, SettingsLang::Fr) => "Analyses",
            (Self::About, SettingsLang::En) => "About",
            (Self::About, SettingsLang::Fr) => "À propos",
        }
    }

    #[must_use]
    pub const fn pages(self) -> &'static [SettingsPageId] {
        match self {
            Self::Ai => &[
                SettingsPageId::Agents,
                SettingsPageId::Accounts,
                SettingsPageId::Skills,
                SettingsPageId::Mcp,
                SettingsPageId::Connections,
                SettingsPageId::Context,
                SettingsPageId::Orchestration,
            ],
            Self::Workspace => &[
                SettingsPageId::TesselAccount,
                SettingsPageId::General,
                SettingsPageId::Appearance,
                SettingsPageId::Terminal,
                SettingsPageId::Editor,
                SettingsPageId::Browser,
                SettingsPageId::Alerts,
                SettingsPageId::Voice,
            ],
            Self::Workflows => &[
                SettingsPageId::Git,
                SettingsPageId::Devtools,
                SettingsPageId::Automations,
                SettingsPageId::QuickCommands,
                SettingsPageId::Ssh,
            ],
            Self::Insights => &[SettingsPageId::Stats],
            Self::About => &[SettingsPageId::Updates],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SettingsPageId {
    Stats,
    Agents,
    Accounts,
    Skills,
    Mcp,
    Connections,
    Context,
    Orchestration,
    Voice,
    #[default]
    General,
    Appearance,
    Terminal,
    Editor,
    Alerts,
    Browser,
    Git,
    Devtools,
    Automations,
    QuickCommands,
    TesselAccount,
    Updates,
    Ssh,
}

pub const ALL_SETTINGS_PAGES: [SettingsPageId; 22] = [
    SettingsPageId::Agents,
    SettingsPageId::Accounts,
    SettingsPageId::Skills,
    SettingsPageId::Mcp,
    SettingsPageId::Connections,
    SettingsPageId::Context,
    SettingsPageId::Orchestration,
    SettingsPageId::TesselAccount,
    SettingsPageId::General,
    SettingsPageId::Appearance,
    SettingsPageId::Terminal,
    SettingsPageId::Editor,
    SettingsPageId::Browser,
    SettingsPageId::Alerts,
    SettingsPageId::Voice,
    SettingsPageId::Git,
    SettingsPageId::Devtools,
    SettingsPageId::Automations,
    SettingsPageId::QuickCommands,
    SettingsPageId::Ssh,
    SettingsPageId::Stats,
    SettingsPageId::Updates,
];

impl SettingsPageId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stats => "stats",
            Self::Agents => "agents",
            Self::Accounts => "accounts",
            Self::Skills => "skills",
            Self::Mcp => "mcp",
            Self::Connections => "connections",
            Self::Context => "context",
            Self::Orchestration => "orchestration",
            Self::Voice => "voice",
            Self::General => "general",
            Self::Appearance => "appearance",
            Self::Terminal => "terminal",
            Self::Editor => "editor",
            Self::Alerts => "alerts",
            Self::Browser => "browser",
            Self::Git => "git",
            Self::Devtools => "devtools",
            Self::Automations => "automations",
            Self::QuickCommands => "quick-commands",
            Self::TesselAccount => "tessel-account",
            Self::Updates => "updates",
            Self::Ssh => "ssh",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "stats" => Some(Self::Stats),
            "agents" => Some(Self::Agents),
            "accounts" => Some(Self::Accounts),
            "skills" => Some(Self::Skills),
            "mcp" => Some(Self::Mcp),
            "connections" => Some(Self::Connections),
            "context" => Some(Self::Context),
            "orchestration" => Some(Self::Orchestration),
            "voice" => Some(Self::Voice),
            "general" => Some(Self::General),
            "appearance" => Some(Self::Appearance),
            "terminal" => Some(Self::Terminal),
            "editor" => Some(Self::Editor),
            "alerts" => Some(Self::Alerts),
            "browser" => Some(Self::Browser),
            "git" => Some(Self::Git),
            "devtools" => Some(Self::Devtools),
            "automations" => Some(Self::Automations),
            "quick-commands" => Some(Self::QuickCommands),
            "tessel-account" => Some(Self::TesselAccount),
            "updates" => Some(Self::Updates),
            "ssh" => Some(Self::Ssh),
            _ => None,
        }
    }

    #[must_use]
    pub const fn group(self) -> SettingsGroupId {
        match self {
            Self::Agents
            | Self::Accounts
            | Self::Skills
            | Self::Mcp
            | Self::Connections
            | Self::Context
            | Self::Orchestration => SettingsGroupId::Ai,
            Self::TesselAccount
            | Self::General
            | Self::Appearance
            | Self::Terminal
            | Self::Editor
            | Self::Browser
            | Self::Alerts
            | Self::Voice => SettingsGroupId::Workspace,
            Self::Git | Self::Devtools | Self::Automations | Self::QuickCommands | Self::Ssh => {
                SettingsGroupId::Workflows
            }
            Self::Stats => SettingsGroupId::Insights,
            Self::Updates => SettingsGroupId::About,
        }
    }

    #[must_use]
    pub const fn title_en(self) -> &'static str {
        match self {
            Self::Stats => "Stats & Usage",
            Self::Agents => "Agents",
            Self::Accounts => "AI provider accounts",
            Self::Skills => "Skills",
            Self::Mcp => "MCP servers",
            Self::Connections => "Connections",
            Self::TesselAccount => "Tessel account",
            Self::Context => "Context",
            Self::Orchestration => "Orchestration",
            Self::Voice => "Voice typing",
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Terminal => "Terminal",
            Self::Editor => "Editor",
            Self::Browser => "Browser",
            Self::Alerts => "Notifications",
            Self::Git => "Git & Source Control",
            Self::Devtools => "Developer tools",
            Self::Automations => "Automations",
            Self::QuickCommands => "Quick commands",
            Self::Updates => "About & updates",
            Self::Ssh => "SSH Hosts",
        }
    }

    #[must_use]
    pub const fn title(self, lang: SettingsLang) -> &'static str {
        match (self, lang) {
            (_, SettingsLang::En) => self.title_en(),
            (Self::Stats, SettingsLang::Fr) => "Statistiques & usage",
            (Self::Agents, SettingsLang::Fr) => "Agents",
            (Self::Accounts, SettingsLang::Fr) => "Comptes de fournisseurs d'IA",
            (Self::Skills, SettingsLang::Fr) => "Skills",
            (Self::Mcp, SettingsLang::Fr) => "Serveurs MCP",
            (Self::Connections, SettingsLang::Fr) => "Connexions",
            (Self::TesselAccount, SettingsLang::Fr) => "Compte Tessel",
            (Self::Context, SettingsLang::Fr) => "Contexte",
            (Self::Orchestration, SettingsLang::Fr) => "Orchestration",
            (Self::Voice, SettingsLang::Fr) => "Saisie vocale",
            (Self::General, SettingsLang::Fr) => "Général",
            (Self::Appearance, SettingsLang::Fr) => "Apparence",
            (Self::Terminal, SettingsLang::Fr) => "Terminal",
            (Self::Editor, SettingsLang::Fr) => "Éditeur",
            (Self::Browser, SettingsLang::Fr) => "Navigateur",
            (Self::Alerts, SettingsLang::Fr) => "Notifications",
            (Self::Git, SettingsLang::Fr) => "Git & gestion de code source",
            (Self::Devtools, SettingsLang::Fr) => "Outils de développement",
            (Self::Automations, SettingsLang::Fr) => "Automatisations",
            (Self::QuickCommands, SettingsLang::Fr) => "Commandes rapides",
            (Self::Updates, SettingsLang::Fr) => "À propos et mises à jour",
            (Self::Ssh, SettingsLang::Fr) => "Hôtes SSH",
        }
    }

    #[must_use]
    pub const fn description(self, lang: SettingsLang) -> &'static str {
        match (self, lang) {
            (Self::Stats, SettingsLang::En) => {
                "The Usage menu, Tessel stats and local agent token analytics."
            }
            (Self::Stats, SettingsLang::Fr) => {
                "Le menu Utilisation, les statistiques de Tessel et l'analyse locale des jetons des agents."
            }
            (Self::Agents, SettingsLang::En) => {
                "Which agents Tessel offers, how they start and what they may do."
            }
            (Self::Agents, SettingsLang::Fr) => {
                "Les agents que Tessel propose, comment ils démarrent et ce qu'ils peuvent faire."
            }
            (Self::Accounts, SettingsLang::En) => {
                "Your sign-ins and usage keys for each AI provider. Keys and cookies stay encrypted on this computer."
            }
            (Self::Accounts, SettingsLang::Fr) => {
                "Vos connexions et clés d'usage pour chaque fournisseur d'IA. Les clés et les cookies restent chiffrés sur cet ordinateur."
            }
            (Self::Skills, SettingsLang::En) => {
                "See, add, update and remove the skills of your agents."
            }
            (Self::Skills, SettingsLang::Fr) => {
                "Voir, ajouter, mettre à jour et retirer les compétences de vos agents."
            }
            (Self::Mcp, SettingsLang::En) => {
                "Give your agents extra tools: install, turn on or off and connect MCP servers."
            }
            (Self::Mcp, SettingsLang::Fr) => {
                "Donnez des outils de plus à vos agents : installez, activez ou désactivez et connectez des serveurs MCP."
            }
            (Self::Connections, SettingsLang::En) => {
                "Connect Gmail, Outlook, Dropbox and other accounts once, for all your agents."
            }
            (Self::Connections, SettingsLang::Fr) => {
                "Connectez Gmail, Outlook, Dropbox et d'autres comptes une seule fois, pour tous vos agents."
            }
            (Self::Context, SettingsLang::En) => {
                "What each agent loads before you type anything, what it costs in tokens, and what you can turn off."
            }
            (Self::Context, SettingsLang::Fr) => {
                "Ce que chaque agent charge avant que vous tapiez quoi que ce soit, ce que ça coûte en jetons, et ce que vous pouvez désactiver."
            }
            (Self::Orchestration, SettingsLang::En) => {
                "Agents working as a team, followed on the task board."
            }
            (Self::Orchestration, SettingsLang::Fr) => {
                "Des agents qui travaillent en équipe, suivis sur le tableau des tâches."
            }
            (Self::Voice, SettingsLang::En) => "Dictate into any pane with Windows voice typing.",
            (Self::Voice, SettingsLang::Fr) => {
                "Dictez dans n'importe quel panneau avec la saisie vocale de Windows."
            }
            (Self::General, SettingsLang::En) => "How Tessel starts and closes.",
            (Self::General, SettingsLang::Fr) => "Comment Tessel démarre et se ferme.",
            (Self::Appearance, SettingsLang::En) => {
                "The language, theme and size of the app, the panes, the left sidebar and the status bar."
            }
            (Self::Appearance, SettingsLang::Fr) => {
                "La langue, le thème et la taille de l'application, les panneaux, la barre latérale gauche et la barre d'état."
            }
            (Self::Terminal, SettingsLang::En) => {
                "Shell, font and cursor, drawing, copy and paste, mouse."
            }
            (Self::Terminal, SettingsLang::Fr) => {
                "Shell, police et curseur, rendu, copier et coller, souris."
            }
            (Self::Editor, SettingsLang::En) => {
                "The code editor in Tessel panes, its diffs, and the Files panel."
            }
            (Self::Editor, SettingsLang::Fr) => {
                "L'éditeur de code des panneaux Tessel, ses diffs et le panneau Fichiers."
            }
            (Self::Alerts, SettingsLang::En) => {
                "Native desktop notifications for agent and terminal events."
            }
            (Self::Alerts, SettingsLang::Fr) => {
                "Notifications natives de bureau pour les événements d'agent et de terminal."
            }
            (Self::Browser, SettingsLang::En) => {
                "The built-in browser: importing your sign-ins and the session agents use."
            }
            (Self::Browser, SettingsLang::Fr) => {
                "Le navigateur intégré : importer vos connexions et la session utilisée par les agents."
            }
            (Self::Git, SettingsLang::En) => "Branch naming and Source Control.",
            (Self::Git, SettingsLang::Fr) => "Nommage des branches et gestion de code source.",
            (Self::Devtools, SettingsLang::En) => {
                "Git, Node.js and the other tools agents use: install them and finish their setup."
            }
            (Self::Devtools, SettingsLang::Fr) => {
                "Git, Node.js et les autres outils qu'utilisent les agents : installez-les et terminez leur configuration."
            }
            (Self::Automations, SettingsLang::En) => {
                "Run an agent task on a schedule, while Tessel is open."
            }
            (Self::Automations, SettingsLang::Fr) => {
                "Exécuter une tâche d'agent selon une planification, quand Tessel est ouvert."
            }
            (Self::QuickCommands, SettingsLang::En) => {
                "Text you send to the active pane from the command palette."
            }
            (Self::QuickCommands, SettingsLang::Fr) => {
                "Du texte envoyé au panneau actif depuis la palette de commandes."
            }
            (Self::TesselAccount, SettingsLang::En) => {
                "Your optional account: sign in, sign out or delete it."
            }
            (Self::TesselAccount, SettingsLang::Fr) => {
                "Votre compte facultatif : connectez-vous, déconnectez-vous ou supprimez-le."
            }
            (Self::Updates, SettingsLang::En) => {
                "Tessel's version and updates, help when something goes wrong, and the licenses."
            }
            (Self::Updates, SettingsLang::Fr) => {
                "La version de Tessel et ses mises à jour, de l’aide en cas de problème, et les licences."
            }
            (Self::Ssh, SettingsLang::En) => {
                "Use existing machines over SSH for files, terminals, Git, and workspaces."
            }
            (Self::Ssh, SettingsLang::Fr) => {
                "Utilisez des machines existantes via SSH pour les fichiers, les terminaux, Git et les espaces de travail."
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Aliases, Anchor Validation & Shell State (`SettingsDialog.vue:60-118`)
// ---------------------------------------------------------------------------

pub const SETTINGS_PAGE_STORAGE_KEY: &str = "tessel.settingsPage";
pub const TEXT_ALIAS_ANCHOR: &str = "set-terminal-font";
pub const MAX_SETTINGS_SEARCH_CHARS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSettingsTarget {
    pub page: SettingsPageId,
    pub anchor: Option<String>,
}

#[must_use]
pub fn is_valid_settings_anchor(anchor: &str) -> bool {
    let len = anchor.len();
    (1..=60).contains(&len)
        && anchor
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[must_use]
pub fn resolve_page_id(raw_id: &str) -> Option<(SettingsPageId, Option<&'static str>)> {
    if raw_id == "text" {
        return Some((SettingsPageId::Terminal, Some(TEXT_ALIAS_ANCHOR)));
    }
    SettingsPageId::from_id(raw_id).map(|p| (p, None))
}

#[must_use]
pub fn resolve_initial_settings_target(
    section: Option<&str>,
    explicit_anchor: Option<&str>,
    stored_page: Option<&str>,
) -> ResolvedSettingsTarget {
    let mut alias_anchor: Option<&str> = None;
    let page = section
        .and_then(|s| {
            let (p, anc) = resolve_page_id(s)?;
            alias_anchor = anc;
            Some(p)
        })
        .or_else(|| stored_page.and_then(|s| resolve_page_id(s).map(|(p, _)| p)))
        .unwrap_or(SettingsPageId::General);

    let anchor = explicit_anchor
        .or(alias_anchor)
        .filter(|a| is_valid_settings_anchor(a))
        .map(ToString::to_string);

    ResolvedSettingsTarget { page, anchor }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum McpSettingsTab {
    #[default]
    Installed,
    Connections,
}

#[derive(Debug, Clone)]
pub struct SettingsShellState {
    pub page: SettingsPageId,
    pub anchor: Option<String>,
    pub mcp_tab: McpSettingsTab,
    pub query: String,
}

impl Default for SettingsShellState {
    fn default() -> Self {
        Self {
            page: SettingsPageId::General,
            anchor: None,
            mcp_tab: McpSettingsTab::Installed,
            query: String::new(),
        }
    }
}

impl SettingsShellState {
    #[must_use]
    pub fn open(section: Option<&str>, anchor: Option<&str>, stored_page: Option<&str>) -> Self {
        let resolved = resolve_initial_settings_target(section, anchor, stored_page);
        Self {
            page: resolved.page,
            anchor: resolved.anchor,
            mcp_tab: McpSettingsTab::Installed,
            query: String::new(),
        }
    }

    pub fn go(&mut self, id: &str) -> bool {
        let Some((target, alias_anchor)) = resolve_page_id(id) else {
            return false;
        };
        if target != SettingsPageId::Mcp {
            self.mcp_tab = McpSettingsTab::Installed;
        }
        self.page = target;
        self.anchor = alias_anchor.map(ToString::to_string);
        self.query.clear();
        true
    }

    pub fn open_mcp_tab(&mut self, tab: McpSettingsTab) {
        self.go("mcp");
        self.mcp_tab = tab;
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query
            .into()
            .chars()
            .take(MAX_SETTINGS_SEARCH_CHARS)
            .collect();
    }

    #[must_use]
    pub fn is_searching(&self) -> bool {
        !self.query.trim().is_empty()
    }

    #[must_use]
    pub fn is_page_shown(&self, id: SettingsPageId, matched: &[SettingsPageId]) -> bool {
        if self.is_searching() {
            matched.contains(&id)
        } else {
            self.page == id
        }
    }
}

// ---------------------------------------------------------------------------
// Multi-Word Settings Search (`SettingsDialog.vue:120-200`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsSearchRow {
    pub page: SettingsPageId,
    pub group_anchor: Option<String>,
    pub group_title: String,
    pub row_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettingsSearchResult {
    pub searching: bool,
    pub matched_pages: Vec<SettingsPageId>,
    pub whole_pages: Vec<SettingsPageId>,
    pub matched_row_ids: Vec<String>,
    pub result_count: usize,
}

fn contains_all_words(text: &str, words: &[String]) -> bool {
    let lower = text.to_lowercase();
    words.iter().all(|w| lower.contains(w))
}

#[must_use]
pub fn search_settings(
    query: &str,
    rows: &[SettingsSearchRow],
    lang: SettingsLang,
) -> SettingsSearchResult {
    let words: Vec<String> = query
        .chars()
        .take(MAX_SETTINGS_SEARCH_CHARS)
        .collect::<String>()
        .trim()
        .to_lowercase()
        .split_whitespace()
        .map(ToString::to_string)
        .collect();

    if words.is_empty() {
        return SettingsSearchResult::default();
    }

    let alias_text_title = match lang {
        SettingsLang::En => "Text",
        SettingsLang::Fr => "Texte",
    };
    let text_alias_matches = [alias_text_title, "Text", "Font"]
        .iter()
        .any(|name| contains_all_words(name, &words));

    let mut whole_pages = Vec::new();
    let mut matched_pages: Vec<SettingsPageId> = Vec::new();
    let mut matched_row_ids: Vec<String> = Vec::new();
    let mut seen_rows: HashSet<String> = HashSet::new();
    let mut result_count = 0usize;

    for &page in &ALL_SETTINGS_PAGES {
        let title_hit = contains_all_words(page.title(lang), &words)
            || contains_all_words(page.title_en(), &words);
        let page_rows: Vec<&SettingsSearchRow> = rows.iter().filter(|r| r.page == page).collect();

        if title_hit {
            whole_pages.push(page);
            matched_pages.push(page);
            if page_rows.is_empty() {
                result_count += 1;
            } else {
                for r in page_rows {
                    if seen_rows.insert(r.row_id.clone()) {
                        matched_row_ids.push(r.row_id.clone());
                        result_count += 1;
                    }
                }
            }
            continue;
        }

        let mut page_any = false;
        for r in &page_rows {
            let group_alias_hit = page == SettingsPageId::Terminal
                && text_alias_matches
                && r.group_anchor.as_deref() == Some(TEXT_ALIAS_ANCHOR);
            let group_hit = !r.group_title.is_empty() && contains_all_words(&r.group_title, &words);
            let row_hit = contains_all_words(&r.text, &words);
            if group_alias_hit || group_hit || row_hit {
                page_any = true;
                if seen_rows.insert(r.row_id.clone()) {
                    matched_row_ids.push(r.row_id.clone());
                    result_count += 1;
                }
            }
        }
        if !page_any && page == SettingsPageId::Terminal && text_alias_matches {
            page_any = true;
            result_count += 1;
        }
        if page_any {
            matched_pages.push(page);
        }
    }

    SettingsSearchResult {
        searching: true,
        whole_pages,
        matched_pages,
        matched_row_ids,
        result_count,
    }
}

#[must_use]
pub fn search_count_label(count: usize, lang: SettingsLang) -> String {
    match (count, lang) {
        (1, SettingsLang::En) => "1 result".to_string(),
        (n, SettingsLang::En) => format!("{n} results"),
        (1, SettingsLang::Fr) => "1 résultat".to_string(),
        (n, SettingsLang::Fr) => format!("{n} résultats"),
    }
}
