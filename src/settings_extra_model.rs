//! Data models, catalogs, and bounded local scanners for the remaining 10 Settings
//! pages (`Accounts`, `Skills`, `Mcp`, `Connections`, `Context`, `Orchestration`,
//! `Alerts`, `Voice`, `Devtools`, `Automations`) and Custom CLI Agents (`CustomAgent`).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MAX_CUSTOM_AGENTS: usize = 30;
pub const CUSTOM_AGENT_SWATCHES: [&str; 8] = [
    "#8a93a6", "#e07a5f", "#e5c07b", "#56b870", "#2bb5a8", "#5b8def", "#a371f7", "#e06c9f",
];

fn default_true() -> bool {
    true
}

fn default_max_workers() -> u8 {
    4
}

fn default_max_depth() -> u8 {
    1
}

fn default_volume() -> u8 {
    100
}

fn default_custom_accent() -> String {
    CUSTOM_AGENT_SWATCHES[0].to_owned()
}

// ---------------------------------------------------------------------------
// 1. Custom CLI Agents
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CustomAgent {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default = "default_custom_accent")]
    pub accent: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl CustomAgent {
    #[must_use]
    pub fn parse_accent_rgb(&self) -> (u8, u8, u8) {
        parse_hex_rgb(&self.accent).unwrap_or((0x8a, 0x93, 0xa6))
    }
}

#[must_use]
pub fn parse_hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let s = hex.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

pub fn generate_custom_agent_id(agents: &[CustomAgent]) -> Result<String, String> {
    generate_custom_agent_id_with(agents, |bytes| {
        getrandom::fill(bytes)
            .map_err(|_| "Secure random generation failed; no custom agent was created.".into())
    })
}

fn generate_custom_agent_id_with(
    agents: &[CustomAgent],
    fill: impl FnOnce(&mut [u8]) -> Result<(), String>,
) -> Result<String, String> {
    let mut bytes = [0u8; 6];
    fill(&mut bytes)?;
    let mut hex = String::with_capacity(19);
    hex.push_str("custom-");
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{b:02x}");
    }
    if agents.iter().any(|agent| agent.id == hex) {
        return Err("Custom agent identifier already exists; retry creating the agent.".into());
    }
    Ok(hex)
}

pub fn validate_custom_agent_fields(
    id: &str,
    name: &str,
    command: &str,
    accent: &str,
) -> Result<(), String> {
    if !id.starts_with("custom-")
        || id.len() < 8
        || id.len() > 48
        || !id[7..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Invalid custom agent identifier.".into());
    }
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err("Custom agent name must be 1–80 printable characters.".into());
    }
    let trimmed_cmd = command.trim();
    if trimmed_cmd.is_empty()
        || command.chars().count() > 500
        || command.contains(['\0', '\r', '\n'])
    {
        return Err("Custom agent command must be 1–500 single-line characters.".into());
    }
    if !CUSTOM_AGENT_SWATCHES.contains(&accent) {
        return Err("Invalid custom agent accent color.".into());
    }
    Ok(())
}

pub fn validate_custom_agents(agents: &[CustomAgent]) -> Result<(), String> {
    if agents.len() > MAX_CUSTOM_AGENTS {
        return Err("Too many custom agents (maximum 30).".into());
    }
    let mut ids = HashSet::with_capacity(agents.len());
    let mut names = HashSet::with_capacity(agents.len());
    for agent in agents {
        validate_custom_agent_fields(&agent.id, &agent.name, &agent.command, &agent.accent)?;
        if !ids.insert(agent.id.clone()) {
            return Err("Duplicate custom agent identifier.".into());
        }
        let lower_name = agent.name.trim().to_lowercase();
        if crate::agent_catalog::ALL
            .iter()
            .any(|preset| preset.name.to_lowercase() == lower_name || preset.id == lower_name)
        {
            return Err(format!(
                "Custom agent name '{}' conflicts with a built-in agent.",
                agent.name.trim()
            ));
        }
        if !names.insert(lower_name) {
            return Err("Duplicate custom agent name.".into());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 2. Orchestration Preferences
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerMode {
    #[default]
    Terminal,
    Chat,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct OrchestrationPreferences {
    #[serde(default = "default_true")]
    pub team_wake_ups: bool,
    pub team_wake_unconfirmed: bool,
    #[serde(default = "default_true")]
    pub confirm_workers: bool,
    #[serde(default = "default_max_workers")]
    pub max_workers: u8,
    #[serde(default = "default_max_depth")]
    pub max_depth: u8,
    pub worker_mode: WorkerMode,
    pub cursor_status_hooks: bool,
}

impl Default for OrchestrationPreferences {
    fn default() -> Self {
        Self {
            team_wake_ups: true,
            team_wake_unconfirmed: false,
            confirm_workers: true,
            max_workers: 4,
            max_depth: 1,
            worker_mode: WorkerMode::Terminal,
            cursor_status_hooks: false,
        }
    }
}

impl OrchestrationPreferences {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=8).contains(&self.max_workers) {
            return Err("Invalid orchestration max_workers (expected 1..=8).".into());
        }
        if !(1..=3).contains(&self.max_depth) {
            return Err("Invalid orchestration max_depth (expected 1..=3).".into());
        }
        Ok(())
    }
}

pub const TEAM_TOOLS_REFERENCE: [(&str, &str, &str); 14] = [
    (
        "team_inbox",
        "Read unread team messages for this agent.",
        "Lire les messages d'équipe non lus.",
    ),
    (
        "team_send",
        "Send a message to a teammate or broadcast to the team.",
        "Envoyer un message à un coéquipier ou à toute l'équipe.",
    ),
    (
        "team_members",
        "List teammates, roles, models, and live pane state.",
        "Lister les membres de l'équipe, rôles et état des panneaux.",
    ),
    (
        "team_create",
        "Create or name a team in the current project.",
        "Créer ou nommer une équipe dans le projet actif.",
    ),
    (
        "team_tasks",
        "List To do, Doing, Review, and Done cards.",
        "Lister les cartes À faire, En cours, Revue et Terminé.",
    ),
    (
        "team_task_add",
        "Create a new card on the project task board.",
        "Ajouter une carte au tableau des tâches du projet.",
    ),
    (
        "team_task_move",
        "Move a task card across workflow columns.",
        "Déplacer une carte entre les colonnes du tableau.",
    ),
    (
        "team_task_done",
        "Mark a task card done with optional completion note.",
        "Marquer une carte comme terminée.",
    ),
    (
        "team_worker_start",
        "Spawn an isolated worker agent on a task or branch.",
        "Démarrer un agent worker isolé sur une tâche.",
    ),
    (
        "team_worker_list",
        "Inspect active workers started by the team.",
        "Lister les workers actifs démarrés par l'équipe.",
    ),
    (
        "team_worker_read",
        "Read recent transcript/output from a worker pane.",
        "Lire la sortie récente d'un panneau worker.",
    ),
    (
        "team_worker_stop",
        "Stop a running worker pane cleanly.",
        "Arrêter proprement un panneau worker.",
    ),
    (
        "team_heartbeat",
        "Report liveness and current step for long tasks.",
        "Signaler l'activité en cours sur une tâche longue.",
    ),
    (
        "project_send",
        "Send an approved cross-project relay message.",
        "Envoyer un message inter-projets approuvé.",
    ),
];

// ---------------------------------------------------------------------------
// 3. Notification / Alert Preferences
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlertSound {
    None,
    #[default]
    Chime,
    Ping,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationPreferences {
    #[serde(default = "default_true")]
    pub notifications_enabled: bool,
    #[serde(default = "default_true")]
    pub desktop_notifications: bool,
    pub notify_terminal_bell: bool,
    #[serde(default = "default_true")]
    pub notify_suppress_when_focused: bool,
    pub alert_sound: AlertSound,
    #[serde(default = "default_volume")]
    pub notification_volume: u8,
    #[serde(default = "default_true")]
    pub in_app_alerts: bool,
}

impl Default for NotificationPreferences {
    fn default() -> Self {
        Self {
            notifications_enabled: true,
            desktop_notifications: true,
            notify_terminal_bell: false,
            notify_suppress_when_focused: true,
            alert_sound: AlertSound::Chime,
            notification_volume: 100,
            in_app_alerts: true,
        }
    }
}

impl NotificationPreferences {
    pub fn validate(&self) -> Result<(), String> {
        if self.notification_volume > 100 {
            return Err("Invalid notification volume (expected 0..=100).".into());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 4. Voice Typing Preferences
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct VoicePreferences {
    pub voice_tip: String,
    pub voice_tip_chosen: bool,
}

impl VoicePreferences {
    pub fn validate(&self) -> Result<(), String> {
        if self.voice_tip.len() > 64 || self.voice_tip.chars().any(char::is_control) {
            return Err("Invalid voice typing language.".into());
        }
        Ok(())
    }
}

#[must_use]
pub fn supported_voice_languages(french: bool) -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "",
            if french {
                "Langue actuelle du clavier"
            } else {
                "Current keyboard language"
            },
        ),
        ("en-US", "English (United States)"),
        ("en-GB", "English (United Kingdom)"),
        ("fr-CA", "Français (Canada)"),
        ("fr-FR", "Français (France)"),
        ("es-ES", "Español"),
        ("de-DE", "Deutsch"),
        ("pt-BR", "Português (Brasil)"),
        ("ja-JP", "日本語"),
    ]
}

// ---------------------------------------------------------------------------
// 5. AI Provider Accounts & Encrypted Provider Secrets
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MinimaxEndpoint {
    #[default]
    Overseas,
    Cn,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZcodePlanSite {
    #[default]
    Zai,
    Bigmodel,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderAccountsPreferences {
    pub gemini_cli_oauth: bool,
    pub opencode_workspace_id: String,
    pub minimax_endpoint: MinimaxEndpoint,
    pub minimax_group_id: String,
    pub minimax_usage_models: String,
    pub zcode_plan_site: ZcodePlanSite,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub host_account_defaults: BTreeMap<String, BTreeMap<String, String>>,
}

impl ProviderAccountsPreferences {
    pub fn validate(&self) -> Result<(), String> {
        if self.opencode_workspace_id.len() > 256
            || self.opencode_workspace_id.chars().any(char::is_control)
            || self.minimax_group_id.len() > 256
            || self.minimax_group_id.chars().any(char::is_control)
            || self.minimax_usage_models.len() > 512
            || self.minimax_usage_models.chars().any(char::is_control)
            || self.host_account_defaults.len() > 200
        {
            return Err("Invalid AI provider account preferences.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedAccount {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub plan: String,
    #[serde(default)]
    pub added_at: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedAccountsRoster {
    #[serde(default)]
    pub accounts: Vec<ManagedAccount>,
    #[serde(default)]
    pub selected_id: Option<String>,
}

#[must_use]
pub fn managed_accounts_root(data_dir: &Path, provider: &str) -> Option<PathBuf> {
    match provider {
        "claude" => Some(data_dir.join("claude-accounts")),
        "codex" => Some(data_dir.join("codex-accounts")),
        _ => None,
    }
}

#[must_use]
pub fn managed_account_profile_dir(
    data_dir: &Path,
    provider: &str,
    account_id: &str,
) -> Option<PathBuf> {
    if account_id.is_empty()
        || account_id.len() > 64
        || !account_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    let root = managed_accounts_root(data_dir, provider)?;
    match provider {
        "claude" => Some(root.join(account_id).join("auth")),
        "codex" => Some(root.join(account_id).join("home")),
        _ => None,
    }
}

fn extract_email_from_jwt_segment(jwt: &str) -> Option<String> {
    use base64::Engine as _;
    let payload_b64 = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload_b64))
        .ok()?;
    let val: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    val.get("email")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| s.contains('@') && !s.is_empty())
        .map(str::to_owned)
}

fn extract_provider_email_from_json(val: &serde_json::Value) -> Option<String> {
    let direct_candidates = [
        val.get("email"),
        val.get("emailAddress"),
        val.get("oauthAccount").and_then(|o| o.get("emailAddress")),
        val.get("oauthAccount").and_then(|o| o.get("email")),
        val.get("claudeAiOauth").and_then(|o| o.get("email")),
        val.get("user").and_then(|o| o.get("email")),
        val.get("account").and_then(|o| o.get("email")),
    ];
    for cand in direct_candidates.into_iter().flatten() {
        if let Some(s) = cand.as_str().map(str::trim)
            && s.contains('@')
            && !s.is_empty()
        {
            return Some(s.to_owned());
        }
    }
    if let Some(jwt) = val
        .get("tokens")
        .and_then(|t| t.get("id_token"))
        .and_then(serde_json::Value::as_str)
        && let Some(email) = extract_email_from_jwt_segment(jwt)
    {
        return Some(email);
    }
    None
}

fn probe_email_in_files(paths: &[PathBuf]) -> Option<String> {
    for path in paths {
        let Some(raw) = read_bounded_utf8(path, 256 * 1024) else {
            continue;
        };
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        if let Some(email) = extract_provider_email_from_json(&val) {
            return Some(email);
        }
    }
    None
}

#[allow(dead_code)]
#[must_use]
pub fn probe_system_provider_email(home_dir: &Path, provider: &str) -> Option<String> {
    match provider {
        "claude" => probe_email_in_files(&[
            home_dir.join(".claude.json"),
            home_dir.join(".claude").join(".credentials.json"),
        ]),
        "codex" => probe_email_in_files(&[home_dir.join(".codex").join("auth.json")]),
        _ => None,
    }
}

#[allow(dead_code)]
#[must_use]
pub fn probe_profile_provider_email(profile_dir: &Path, provider: &str) -> Option<String> {
    match provider {
        "claude" => probe_email_in_files(&[
            profile_dir.join(".claude.json"),
            profile_dir.join(".credentials.json"),
        ]),
        "codex" => probe_email_in_files(&[profile_dir.join("auth.json")]),
        _ => None,
    }
}

/// Returns `true` when the system-default provider account has a known non-empty email and the
/// currently selected managed account in `roster` carries that exact email (case-insensitive).
#[allow(dead_code)]
#[must_use]
pub fn system_account_covered(system_email: Option<&str>, roster: &ManagedAccountsRoster) -> bool {
    let Some(sys) = system_email.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    let Some(sel_id) = roster.selected_id.as_deref() else {
        return false;
    };
    roster
        .accounts
        .iter()
        .find(|a| a.id == sel_id)
        .is_some_and(|a| {
            let acct_email = a.email.trim();
            !acct_email.is_empty() && acct_email.eq_ignore_ascii_case(sys)
        })
}

#[must_use]
pub fn load_managed_accounts(data_dir: &Path, provider: &str) -> ManagedAccountsRoster {
    let Some(root) = managed_accounts_root(data_dir, provider) else {
        return ManagedAccountsRoster::default();
    };
    let path = root.join("accounts.json");
    let mut roster = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ManagedAccountsRoster>(&bytes).ok())
        .unwrap_or_default();
    for acct in &mut roster.accounts {
        if acct.email.trim().is_empty()
            && let Some(profile_dir) = managed_account_profile_dir(data_dir, provider, &acct.id)
            && let Some(email) = probe_profile_provider_email(&profile_dir, provider)
        {
            acct.email = email;
        }
    }
    roster
}

pub fn save_managed_accounts(
    data_dir: &Path,
    provider: &str,
    roster: &ManagedAccountsRoster,
) -> Result<(), String> {
    let root = managed_accounts_root(data_dir, provider).ok_or("Unsupported managed provider.")?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(roster).map_err(|e| e.to_string())?;
    let path = root.join("accounts.json");
    let tmp = root.join("accounts.json.tmp");
    fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn add_managed_account(
    data_dir: &Path,
    provider: &str,
    label: &str,
) -> Result<ManagedAccount, String> {
    add_managed_account_with(data_dir, provider, label, |bytes| {
        getrandom::fill(bytes)
            .map_err(|_| "Secure random generation failed; no account was created.".into())
    })
}

fn add_managed_account_with(
    data_dir: &Path,
    provider: &str,
    label: &str,
    fill: impl FnOnce(&mut [u8]) -> Result<(), String>,
) -> Result<ManagedAccount, String> {
    let trimmed = label.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 80 || trimmed.chars().any(char::is_control) {
        return Err("Account label must be 1–80 printable characters.".into());
    }
    let mut roster = load_managed_accounts(data_dir, provider);
    if roster.accounts.len() >= 32 {
        return Err("Maximum managed accounts reached (32).".into());
    }
    let mut raw = [0u8; 8];
    fill(&mut raw)?;
    let mut id = String::with_capacity(20);
    id.push_str("acct-");
    for b in raw {
        use std::fmt::Write as _;
        let _ = write!(&mut id, "{b:02x}");
    }
    let profile_dir = managed_account_profile_dir(data_dir, provider, &id)
        .ok_or("Invalid managed account path.")?;
    if roster.accounts.iter().any(|account| account.id == id) {
        return Err("Account identifier already exists; retry creating the account.".into());
    }
    let account_dir = profile_dir
        .parent()
        .ok_or("Invalid managed account path.")?;
    let root = account_dir
        .parent()
        .ok_or("Invalid managed account path.")?;
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    // Reserve a new account directory exclusively, never reuse another profile.
    fs::create_dir(account_dir).map_err(|e| format!("Could not reserve a new account: {e}"))?;
    fs::create_dir(&profile_dir).map_err(|e| e.to_string())?;
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let acct = ManagedAccount {
        id: id.clone(),
        label: trimmed.to_owned(),
        email: String::new(),
        plan: String::new(),
        added_at: now_secs,
    };
    roster.accounts.push(acct.clone());
    if roster.selected_id.is_none() {
        roster.selected_id = Some(id);
    }
    save_managed_accounts(data_dir, provider, &roster)?;
    Ok(acct)
}

#[must_use]
pub fn active_managed_env_line(data_dir: &Path, provider: &str) -> Option<String> {
    let roster = load_managed_accounts(data_dir, provider);
    let selected = roster.selected_id.as_deref()?;
    if !roster.accounts.iter().any(|a| a.id == selected) {
        return None;
    }
    let profile_dir = managed_account_profile_dir(data_dir, provider, selected)?;
    match provider {
        "claude" => Some(format!("CLAUDE_CONFIG_DIR={}", profile_dir.display())),
        "codex" => Some(format!("CODEX_HOME={}", profile_dir.display())),
        _ => None,
    }
}

pub const PROVIDER_SECRET_KEYS: [&str; 5] = [
    "opencodeGoApiKey",
    "opencodeCookie",
    "minimaxApiKey",
    "minimaxCookie",
    "zcodePlanApiKey",
];

#[derive(Default, Serialize, Deserialize)]
struct ProviderSecretsPayload {
    #[serde(default)]
    entries: BTreeMap<String, String>,
}

impl Drop for ProviderSecretsPayload {
    fn drop(&mut self) {
        use zeroize::Zeroize as _;
        for val in self.entries.values_mut() {
            val.zeroize();
        }
    }
}

pub fn load_provider_secret_flags(data_dir: &Path) -> BTreeMap<String, bool> {
    let mut flags = BTreeMap::new();
    for key in PROVIDER_SECRET_KEYS {
        flags.insert(key.to_owned(), false);
    }
    if let Ok(payload) = read_provider_secrets(data_dir) {
        for key in PROVIDER_SECRET_KEYS {
            if payload.entries.get(key).is_some_and(|v| !v.is_empty()) {
                flags.insert(key.to_owned(), true);
            }
        }
    }
    flags
}

pub fn set_provider_secret(data_dir: &Path, key: &str, value: &str) -> Result<(), String> {
    if !PROVIDER_SECRET_KEYS.contains(&key) {
        return Err("Unsupported provider secret key.".into());
    }
    let trimmed = value.trim();
    if trimmed.chars().count() > 16_384 || trimmed.contains('\0') {
        return Err("Provider credential exceeds 16,384 characters or contains NUL.".into());
    }
    let mut payload = read_provider_secrets(data_dir).unwrap_or_default();
    if trimmed.is_empty() {
        payload.entries.remove(key);
    } else {
        payload.entries.insert(key.to_owned(), trimmed.to_owned());
    }
    write_provider_secrets(data_dir, &payload)
}

fn read_provider_secrets(data_dir: &Path) -> Result<ProviderSecretsPayload, String> {
    let path = data_dir.join("provider-secrets.dat");
    let Ok(raw) = fs::read(&path) else {
        return Ok(ProviderSecretsPayload::default());
    };
    #[cfg(windows)]
    {
        let plain = crate::vault::protect(&raw, false)?;
        serde_json::from_slice(&plain).map_err(|_| "Corrupted provider secrets store.".into())
    }
    #[cfg(not(windows))]
    {
        serde_json::from_slice(&raw).map_err(|_| "Corrupted provider secrets store.".into())
    }
}

fn write_provider_secrets(data_dir: &Path, payload: &ProviderSecretsPayload) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let plain = zeroize::Zeroizing::new(
        serde_json::to_vec(payload).map_err(|_| "Cannot encode provider secrets.")?,
    );
    #[cfg(windows)]
    let sealed = crate::vault::protect(&plain, true)?;
    #[cfg(not(windows))]
    let sealed = plain;
    let path = data_dir.join("provider-secrets.dat");
    let tmp = data_dir.join("provider-secrets.dat.tmp");
    fs::write(&tmp, &*sealed).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CliSignInProbe {
    pub grok_signed_in: bool,
    pub muse_signed_in: bool,
    pub cursor_signed_in: bool,
}

#[must_use]
pub fn probe_cli_sign_ins(home_dir: &Path) -> CliSignInProbe {
    let non_empty_file = |p: PathBuf| fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 2);
    CliSignInProbe {
        grok_signed_in: non_empty_file(home_dir.join(".grok").join("auth.json")),
        muse_signed_in: non_empty_file(home_dir.join(".config").join("muse").join("auth.json")),
        cursor_signed_in: non_empty_file(home_dir.join(".cursor").join("cli-config.json"))
            || non_empty_file(home_dir.join(".config").join("cursor").join("auth.json")),
    }
}

// ---------------------------------------------------------------------------
// 6. Skills Scanner, Frontmatter Parser & Safe Installer
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillScope {
    User,
    Project,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub agent_label: String,
    pub scope: SkillScope,
    pub dir_path: PathBuf,
    pub skill_md_path: PathBuf,
}

const WINDOWS_RESERVED_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

#[must_use]
pub fn is_safe_skill_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 || name.ends_with('.') || name.ends_with(' ') {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return false;
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    !WINDOWS_RESERVED_NAMES.contains(&stem.as_str())
}

#[must_use]
pub fn parse_skill_frontmatter(markdown: &str) -> (Option<String>, Option<String>) {
    let text = markdown.strip_prefix('\u{feff}').unwrap_or(markdown);
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" || trimmed == "..." {
            break;
        }
        if let Some((key, raw_val)) = trimmed.split_once(':') {
            let k = key.trim();
            let v = raw_val.trim().trim_matches('"').trim_matches('\'').trim();
            if v.is_empty() || v == "|" || v == ">" {
                continue;
            }
            if k.eq_ignore_ascii_case("name") && name.is_none() {
                name = Some(truncate_chars(v, 80));
            } else if k.eq_ignore_ascii_case("description") && description.is_none() {
                description = Some(truncate_chars(v, 512));
            }
        }
    }
    (name, description)
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_owned()
    } else {
        s.chars().take(max_chars).collect()
    }
}

fn is_symlink_or_reparse(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return true;
        }
    }
    false
}

#[must_use]
pub fn user_skill_roots(home_dir: &Path) -> Vec<(&'static str, PathBuf)> {
    vec![
        ("Claude", home_dir.join(".claude").join("skills")),
        ("Agents", home_dir.join(".agents").join("skills")),
        ("Codex", home_dir.join(".codex").join("skills")),
        (
            "OpenCode",
            home_dir.join(".config").join("opencode").join("skills"),
        ),
        ("Copilot", home_dir.join(".copilot").join("skills")),
        ("Gemini", home_dir.join(".gemini").join("skills")),
        (
            "Antigravity",
            home_dir.join(".gemini").join("config").join("skills"),
        ),
        ("Muse", home_dir.join(".config").join("muse").join("skills")),
        ("Grok", home_dir.join(".grok").join("skills")),
    ]
}

#[must_use]
pub fn project_skill_roots(project_dir: &Path) -> Vec<(&'static str, PathBuf)> {
    vec![
        ("Claude", project_dir.join(".claude").join("skills")),
        ("Agents", project_dir.join(".agents").join("skills")),
        ("Codex", project_dir.join(".codex").join("skills")),
        ("OpenCode", project_dir.join(".opencode").join("skills")),
        ("Copilot", project_dir.join(".github").join("skills")),
        ("Gemini", project_dir.join(".gemini").join("skills")),
        ("Grok", project_dir.join(".grok").join("skills")),
    ]
}

#[must_use]
pub fn scan_skills(home_dir: &Path, project_dir: Option<&Path>) -> Vec<SkillEntry> {
    let mut out = Vec::new();
    let mut seen_paths = HashSet::new();
    for (agent, root) in user_skill_roots(home_dir) {
        scan_skill_dir_recursive(&root, agent, SkillScope::User, 0, &mut seen_paths, &mut out);
    }
    if let Some(proj) = project_dir {
        for (agent, root) in project_skill_roots(proj) {
            scan_skill_dir_recursive(
                &root,
                agent,
                SkillScope::Project,
                0,
                &mut seen_paths,
                &mut out,
            );
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn scan_skill_dir_recursive(
    dir: &Path,
    agent: &str,
    scope: SkillScope,
    depth: usize,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<SkillEntry>,
) {
    if depth > 3 || out.len() >= 2000 {
        return;
    }
    let Ok(meta) = fs::symlink_metadata(dir) else {
        return;
    };
    if is_symlink_or_reparse(&meta) || !meta.is_dir() {
        return;
    }
    let skill_md = dir.join("SKILL.md");
    if depth > 0
        && let Ok(md_meta) = fs::symlink_metadata(&skill_md)
        && !is_symlink_or_reparse(&md_meta)
        && md_meta.is_file()
    {
        if seen.insert(skill_md.clone()) {
            let content = read_bounded_utf8(&skill_md, 64 * 1024).unwrap_or_default();
            let (fm_name, fm_desc) = parse_skill_frontmatter(&content);
            let fallback_name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("skill")
                .to_owned();
            out.push(SkillEntry {
                name: fm_name.unwrap_or(fallback_name),
                description: fm_desc.unwrap_or_default(),
                agent_label: agent.to_owned(),
                scope,
                dir_path: dir.to_path_buf(),
                skill_md_path: skill_md,
            });
        }
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= 2000 {
            break;
        }
        let path = entry.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n == ".git" || n == "node_modules")
        {
            continue;
        }
        scan_skill_dir_recursive(&path, agent, scope, depth + 1, seen, out);
    }
}

pub fn install_skill_from_folder(
    source_dir: &Path,
    dest_skills_root: &Path,
) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(source_dir)
        .map_err(|_| "Source skill folder does not exist.".to_string())?;
    if is_symlink_or_reparse(&meta) || !meta.is_dir() {
        return Err("Source skill folder must be a real directory (not a link).".into());
    }
    let folder_name = source_dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid skill folder name.")?;
    if !is_safe_skill_name(folder_name) {
        return Err("Skill folder name must match [A-Za-z0-9._-] (max 64 chars).".into());
    }
    let skill_md = source_dir.join("SKILL.md");
    let md_meta = fs::symlink_metadata(&skill_md)
        .map_err(|_| "Source folder does not contain a SKILL.md file.".to_string())?;
    if is_symlink_or_reparse(&md_meta) || !md_meta.is_file() {
        return Err("SKILL.md must be a regular file.".into());
    }

    let mut file_count = 0usize;
    let mut total_bytes = 0u64;
    validate_skill_tree_bounds(source_dir, &mut file_count, &mut total_bytes)?;

    fs::create_dir_all(dest_skills_root).map_err(|e| e.to_string())?;
    let target_dir = dest_skills_root.join(folder_name);
    if target_dir.exists() {
        return Err(format!(
            "Skill '{folder_name}' is already installed in that location."
        ));
    }
    copy_dir_no_links(source_dir, &target_dir)?;
    Ok(target_dir)
}

fn validate_skill_tree_bounds(
    dir: &Path,
    files: &mut usize,
    bytes: &mut u64,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if is_symlink_or_reparse(&meta) {
            return Err("Skill package cannot contain symbolic links or junctions.".into());
        }
        if meta.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some(".git") {
                continue;
            }
            validate_skill_tree_bounds(&path, files, bytes)?;
        } else if meta.is_file() {
            *files += 1;
            *bytes = bytes.saturating_add(meta.len());
            if *files > 1000 {
                return Err("Skill package exceeds the 1,000-file safety limit.".into());
            }
            if *bytes > 20 * 1024 * 1024 {
                return Err("Skill package exceeds the 20 MB safety limit.".into());
            }
        }
    }
    Ok(())
}

fn copy_dir_no_links(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.file_name().and_then(|n| n.to_str()) == Some(".git") {
            continue;
        }
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if is_symlink_or_reparse(&meta) {
            return Err("Refusing to copy symbolic link or junction.".into());
        }
        let target = dst.join(entry.file_name());
        if meta.is_dir() {
            copy_dir_no_links(&path, &target)?;
        } else if meta.is_file() {
            fs::copy(&path, &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn remove_skill_dir(skill_dir: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(skill_dir).map_err(|e| e.to_string())?;
    if is_symlink_or_reparse(&meta) || !meta.is_dir() {
        return Err("Refusing to remove a symbolic link or non-directory skill.".into());
    }
    if !skill_dir.join("SKILL.md").is_file() {
        return Err("Directory does not contain SKILL.md; refusing removal.".into());
    }
    fs::remove_dir_all(skill_dir).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// 7. MCP Servers Catalog, Config Manager & Probe
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpTransport {
    Stdio,
    Http,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpCategory {
    Browser,
    Docs,
    Code,
    Utility,
    Data,
    Services,
    Productivity,
}

impl McpCategory {
    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Browser, false) => "Browser",
            (Self::Browser, true) => "Navigateur",
            (Self::Docs, false) => "Docs",
            (Self::Docs, true) => "Documentation",
            (Self::Code, false) => "Code",
            (Self::Code, true) => "Code",
            (Self::Utility, false) => "Utility",
            (Self::Utility, true) => "Utilitaires",
            (Self::Data, false) => "Data",
            (Self::Data, true) => "Données",
            (Self::Services, false) => "Services",
            (Self::Services, true) => "Services",
            (Self::Productivity, false) => "Productivity",
            (Self::Productivity, true) => "Productivité",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct McpCatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub category: McpCategory,
    pub transport: McpTransport,
    pub command_or_url: &'static str,
    pub args: &'static [&'static str],
    pub env_keys: &'static [&'static str],
    pub desc_en: &'static str,
    pub desc_fr: &'static str,
}

pub const MCP_CATALOG: [McpCatalogEntry; 30] = [
    // Browser (3)
    McpCatalogEntry {
        id: "playwright",
        name: "Playwright",
        category: McpCategory::Browser,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@playwright/mcp@latest"],
        env_keys: &[],
        desc_en: "Browser automation and accessibility snapshots via Playwright.",
        desc_fr: "Automatisation navigateur et captures d'accessibilité via Playwright.",
    },
    McpCatalogEntry {
        id: "chrome-devtools",
        name: "Chrome DevTools",
        category: McpCategory::Browser,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "chrome-devtools-mcp@latest"],
        env_keys: &[],
        desc_en: "Inspect DOM, network requests, performance traces, and console.",
        desc_fr: "Inspection DOM, réseau, performances et console Chrome DevTools.",
    },
    McpCatalogEntry {
        id: "puppeteer",
        name: "Puppeteer",
        category: McpCategory::Browser,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@modelcontextprotocol/server-puppeteer"],
        env_keys: &[],
        desc_en: "Headless Chromium navigation, screenshots, and page evaluation.",
        desc_fr: "Navigation Chromium headless, captures d'écran et scripts.",
    },
    // Docs (3)
    McpCatalogEntry {
        id: "context7",
        name: "Context7",
        category: McpCategory::Docs,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@upstash/context7-mcp@latest"],
        env_keys: &[],
        desc_en: "Up-to-date versioned library and framework documentation.",
        desc_fr: "Documentation à jour des bibliothèques et frameworks.",
    },
    McpCatalogEntry {
        id: "fetch",
        name: "Fetch",
        category: McpCategory::Docs,
        transport: McpTransport::Stdio,
        command_or_url: "uvx",
        args: &["mcp-server-fetch"],
        env_keys: &[],
        desc_en: "Fetch web pages and convert HTML to clean Markdown for agents.",
        desc_fr: "Récupère des pages web et les convertit en Markdown propre.",
    },
    McpCatalogEntry {
        id: "cloudflare-docs",
        name: "Cloudflare Docs",
        category: McpCategory::Docs,
        transport: McpTransport::Http,
        command_or_url: "https://docs.mcp.cloudflare.com/mcp",
        args: &[],
        env_keys: &[],
        desc_en: "Search official Cloudflare Workers, Pages, and platform docs.",
        desc_fr: "Recherche dans la documentation officielle Cloudflare.",
    },
    // Code (4)
    McpCatalogEntry {
        id: "github",
        name: "GitHub",
        category: McpCategory::Code,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@modelcontextprotocol/server-github"],
        env_keys: &["GITHUB_PERSONAL_ACCESS_TOKEN"],
        desc_en: "Repositories, pull requests, issues, code search, and workflows.",
        desc_fr: "Dépôts, pull requests, tickets, recherche de code et workflows.",
    },
    McpCatalogEntry {
        id: "gitlab",
        name: "GitLab",
        category: McpCategory::Code,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@modelcontextprotocol/server-gitlab"],
        env_keys: &["GITLAB_PERSONAL_ACCESS_TOKEN"],
        desc_en: "GitLab projects, merge requests, pipelines, and issues.",
        desc_fr: "Projets GitLab, merge requests, pipelines et tickets.",
    },
    McpCatalogEntry {
        id: "git",
        name: "Git",
        category: McpCategory::Code,
        transport: McpTransport::Stdio,
        command_or_url: "uvx",
        args: &["mcp-server-git"],
        env_keys: &[],
        desc_en: "Read, search, diff, and inspect local Git repositories.",
        desc_fr: "Lecture, recherche, diff et inspection de dépôts Git locaux.",
    },
    McpCatalogEntry {
        id: "semgrep",
        name: "Semgrep",
        category: McpCategory::Code,
        transport: McpTransport::Stdio,
        command_or_url: "uvx",
        args: &["semgrep-mcp"],
        env_keys: &[],
        desc_en: "Static security analysis and custom rule scanning for codebases.",
        desc_fr: "Analyse statique de sécurité et règles personnalisées Semgrep.",
    },
    // Utility (4)
    McpCatalogEntry {
        id: "filesystem",
        name: "Filesystem",
        category: McpCategory::Utility,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@modelcontextprotocol/server-filesystem", "."],
        env_keys: &[],
        desc_en: "Sandboxed file read/write/search operations on allowed paths.",
        desc_fr: "Lecture, écriture et recherche de fichiers dans les dossiers autorisés.",
    },
    McpCatalogEntry {
        id: "memory",
        name: "Memory",
        category: McpCategory::Utility,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@modelcontextprotocol/server-memory"],
        env_keys: &[],
        desc_en: "Persistent local knowledge-graph memory across agent sessions.",
        desc_fr: "Graphe de connaissances persistant entre les sessions d'agents.",
    },
    McpCatalogEntry {
        id: "sequential-thinking",
        name: "Sequential Thinking",
        category: McpCategory::Utility,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@modelcontextprotocol/server-sequential-thinking"],
        env_keys: &[],
        desc_en: "Structured multi-step problem decomposition and revision.",
        desc_fr: "Décomposition structurée de problèmes complexes étape par étape.",
    },
    McpCatalogEntry {
        id: "time",
        name: "Time",
        category: McpCategory::Utility,
        transport: McpTransport::Stdio,
        command_or_url: "uvx",
        args: &["mcp-server-time"],
        env_keys: &[],
        desc_en: "Current time and IANA timezone conversions.",
        desc_fr: "Heure courante et conversions de fuseaux horaires IANA.",
    },
    // Data (4)
    McpCatalogEntry {
        id: "postgres",
        name: "PostgreSQL",
        category: McpCategory::Data,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@modelcontextprotocol/server-postgres"],
        env_keys: &["DATABASE_URL"],
        desc_en: "Inspect PostgreSQL schemas and run read-only SQL queries.",
        desc_fr: "Inspection de schémas PostgreSQL et requêtes SQL en lecture seule.",
    },
    McpCatalogEntry {
        id: "sqlite",
        name: "SQLite",
        category: McpCategory::Data,
        transport: McpTransport::Stdio,
        command_or_url: "uvx",
        args: &["mcp-server-sqlite"],
        env_keys: &[],
        desc_en: "Query and analyze local SQLite databases and tables.",
        desc_fr: "Requêtes et analyse sur bases de données SQLite locales.",
    },
    McpCatalogEntry {
        id: "supabase",
        name: "Supabase",
        category: McpCategory::Data,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@supabase/mcp-server-supabase@latest"],
        env_keys: &["SUPABASE_ACCESS_TOKEN"],
        desc_en: "Manage Supabase projects, tables, migrations, and Edge Functions.",
        desc_fr: "Gestion de projets, tables, migrations et Edge Functions Supabase.",
    },
    McpCatalogEntry {
        id: "redis",
        name: "Redis",
        category: McpCategory::Data,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@modelcontextprotocol/server-redis"],
        env_keys: &["REDIS_URL"],
        desc_en: "Inspect and manage keys and data structures in Redis.",
        desc_fr: "Inspection et gestion des clés et structures Redis.",
    },
    // Services (5)
    McpCatalogEntry {
        id: "linear",
        name: "Linear",
        category: McpCategory::Services,
        transport: McpTransport::Http,
        command_or_url: "https://mcp.linear.app/mcp",
        args: &[],
        env_keys: &[],
        desc_en: "Search, create, and update Linear issues, cycles, and projects.",
        desc_fr: "Recherche, création et mise à jour de tickets et cycles Linear.",
    },
    McpCatalogEntry {
        id: "sentry",
        name: "Sentry",
        category: McpCategory::Services,
        transport: McpTransport::Http,
        command_or_url: "https://mcp.sentry.dev/mcp",
        args: &[],
        env_keys: &[],
        desc_en: "Inspect Sentry production errors, stack traces, and releases.",
        desc_fr: "Inspection des erreurs Sentry, traces d'appels et versions.",
    },
    McpCatalogEntry {
        id: "stripe",
        name: "Stripe",
        category: McpCategory::Services,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@stripe/mcp", "--tools=all"],
        env_keys: &["STRIPE_SECRET_KEY"],
        desc_en: "Interact with Stripe customers, products, invoices, and payments.",
        desc_fr: "Gestion des clients, produits, factures et paiements Stripe.",
    },
    McpCatalogEntry {
        id: "vercel",
        name: "Vercel",
        category: McpCategory::Services,
        transport: McpTransport::Http,
        command_or_url: "https://mcp.vercel.com",
        args: &[],
        env_keys: &[],
        desc_en: "Inspect Vercel deployments, build logs, and project settings.",
        desc_fr: "Inspection des déploiements, journaux de build et projets Vercel.",
    },
    McpCatalogEntry {
        id: "slack",
        name: "Slack",
        category: McpCategory::Services,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@modelcontextprotocol/server-slack"],
        env_keys: &["SLACK_BOT_TOKEN", "SLACK_TEAM_ID"],
        desc_en: "Read channels, threads, and post updates to Slack workspaces.",
        desc_fr: "Lecture des canaux, fils et envoi de messages Slack.",
    },
    // Productivity (7 — shared with Connections > Services)
    McpCatalogEntry {
        id: "google-workspace",
        name: "Google Workspace",
        category: McpCategory::Productivity,
        transport: McpTransport::Stdio,
        command_or_url: "uvx",
        args: &["workspace-mcp"],
        env_keys: &["GOOGLE_OAUTH_CLIENT_ID", "GOOGLE_OAUTH_CLIENT_SECRET"],
        desc_en: "Gmail, Google Calendar, Drive, Docs, and Sheets integration.",
        desc_fr: "Intégration Gmail, Google Agenda, Drive, Docs et Sheets.",
    },
    McpCatalogEntry {
        id: "outlook",
        name: "Microsoft Outlook & 365",
        category: McpCategory::Productivity,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@softeria/ms-365-mcp-server"],
        env_keys: &[],
        desc_en: "Outlook mail, calendar, OneDrive, and Microsoft 365 tools.",
        desc_fr: "Courriels Outlook, calendrier, OneDrive et outils Microsoft 365.",
    },
    McpCatalogEntry {
        id: "dropbox",
        name: "Dropbox",
        category: McpCategory::Productivity,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "dropbox-mcp-server"],
        env_keys: &["DROPBOX_ACCESS_TOKEN"],
        desc_en: "Search, organize, and inspect files in Dropbox.",
        desc_fr: "Recherche, organisation et inspection de fichiers Dropbox.",
    },
    McpCatalogEntry {
        id: "todoist",
        name: "Todoist",
        category: McpCategory::Productivity,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@abhiz123/todoist-mcp-server"],
        env_keys: &["TODOIST_API_TOKEN"],
        desc_en: "Manage Todoist tasks, projects, labels, and due dates.",
        desc_fr: "Gestion des tâches, projets, étiquettes et échéances Todoist.",
    },
    McpCatalogEntry {
        id: "airtable",
        name: "Airtable",
        category: McpCategory::Productivity,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "airtable-mcp-server"],
        env_keys: &["AIRTABLE_API_KEY"],
        desc_en: "Read and update bases, tables, and records in Airtable.",
        desc_fr: "Lecture et mise à jour des bases, tables et enregistrements Airtable.",
    },
    McpCatalogEntry {
        id: "clickup",
        name: "ClickUp",
        category: McpCategory::Productivity,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@taazkareem/clickup-mcp-server"],
        env_keys: &["CLICKUP_API_KEY", "CLICKUP_TEAM_ID"],
        desc_en: "Manage ClickUp spaces, lists, tasks, and assignees.",
        desc_fr: "Gestion des espaces, listes et tâches ClickUp.",
    },
    McpCatalogEntry {
        id: "monday",
        name: "monday.com",
        category: McpCategory::Productivity,
        transport: McpTransport::Stdio,
        command_or_url: "npx",
        args: &["-y", "@mondaydotcomorg/monday-api-mcp"],
        env_keys: &["MONDAY_TOKEN"],
        desc_en: "Boards, items, columns, and updates on monday.com.",
        desc_fr: "Tableaux, éléments, colonnes et mises à jour monday.com.",
    },
];

#[must_use]
pub fn is_valid_mcp_server_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledMcpServer {
    pub agent_id: String,
    pub agent_label: String,
    pub config_path: PathBuf,
    pub name: String,
    pub transport: McpTransport,
    pub endpoint_summary: String,
    pub env_keys: Vec<String>,
}

#[must_use]
pub fn mcp_agent_config_targets(
    home_dir: &Path,
    project_dir: Option<&Path>,
) -> Vec<(&'static str, &'static str, PathBuf)> {
    let mut targets = vec![
        (
            "claude",
            "Claude Code (User)",
            home_dir.join(".claude.json"),
        ),
        (
            "gemini",
            "Gemini CLI",
            home_dir.join(".gemini").join("settings.json"),
        ),
        (
            "cursor",
            "Cursor",
            home_dir.join(".cursor").join("mcp.json"),
        ),
        (
            "copilot",
            "GitHub Copilot",
            home_dir.join(".copilot").join("mcp-config.json"),
        ),
        (
            "qwen",
            "Qwen Code",
            home_dir.join(".qwen").join("settings.json"),
        ),
        (
            "antigravity",
            "Antigravity",
            home_dir
                .join(".gemini")
                .join("config")
                .join("mcp_config.json"),
        ),
        (
            "opencode",
            "OpenCode",
            home_dir
                .join(".config")
                .join("opencode")
                .join("opencode.json"),
        ),
        (
            "kimi",
            "Kimi Code",
            home_dir.join(".kimi-code").join("mcp.json"),
        ),
    ];
    if let Some(proj) = project_dir {
        targets.insert(
            1,
            (
                "claude-project",
                "Claude (.mcp.json)",
                proj.join(".mcp.json"),
            ),
        );
    }
    targets
}

#[must_use]
pub fn scan_installed_mcp_servers(
    home_dir: &Path,
    project_dir: Option<&Path>,
) -> Vec<InstalledMcpServer> {
    let mut out = Vec::new();
    for (agent_id, label, path) in mcp_agent_config_targets(home_dir, project_dir) {
        let Some(text) = read_bounded_utf8(&path, 512 * 1024) else {
            continue;
        };
        let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let servers_key = if agent_id == "opencode" {
            "mcp"
        } else {
            "mcpServers"
        };
        let Some(obj) = root.get(servers_key).and_then(serde_json::Value::as_object) else {
            continue;
        };
        for (name, spec) in obj {
            let Some(spec_obj) = spec.as_object() else {
                continue;
            };
            let url = spec_obj
                .get("url")
                .or_else(|| spec_obj.get("serverUrl"))
                .and_then(serde_json::Value::as_str);
            let (transport, summary) = if let Some(u) = url {
                (McpTransport::Http, mask_url_query(u))
            } else {
                let cmd = spec_obj
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("stdio");
                let args: Vec<&str> = spec_obj
                    .get("args")
                    .and_then(serde_json::Value::as_array)
                    .map(|arr| arr.iter().filter_map(serde_json::Value::as_str).collect())
                    .unwrap_or_default();
                let summary = if args.is_empty() {
                    cmd.to_owned()
                } else {
                    format!("{cmd} {}", args.join(" "))
                };
                (McpTransport::Stdio, summary)
            };
            let env_keys = spec_obj
                .get("env")
                .and_then(serde_json::Value::as_object)
                .map(|env| env.keys().cloned().collect())
                .unwrap_or_default();
            out.push(InstalledMcpServer {
                agent_id: agent_id.to_owned(),
                agent_label: label.to_owned(),
                config_path: path.clone(),
                name: name.clone(),
                transport,
                endpoint_summary: summary,
                env_keys,
            });
        }
    }
    // Also scan Codex ~/.codex/config.toml `[mcp_servers.<name>]`
    let codex_toml = home_dir.join(".codex").join("config.toml");
    if let Some(toml_text) = read_bounded_utf8(&codex_toml, 256 * 1024) {
        for (name, summary) in parse_codex_toml_mcp_servers(&toml_text) {
            out.push(InstalledMcpServer {
                agent_id: "codex".into(),
                agent_label: "Codex".into(),
                config_path: codex_toml.clone(),
                name,
                transport: if summary.starts_with("http://") || summary.starts_with("https://") {
                    McpTransport::Http
                } else {
                    McpTransport::Stdio
                },
                endpoint_summary: summary,
                env_keys: Vec::new(),
            });
        }
    }
    out
}

fn mask_url_query(raw: &str) -> String {
    if let Some((base, _)) = raw.split_once('?') {
        format!("{base}?***")
    } else {
        raw.to_owned()
    }
}

fn parse_codex_toml_mcp_servers(toml_text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut current: Option<String> = None;
    let mut current_cmd = String::new();
    for line in toml_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if let Some(prev) = current.take() {
                found.push((
                    prev,
                    if current_cmd.is_empty() {
                        "toml".into()
                    } else {
                        std::mem::take(&mut current_cmd)
                    },
                ));
            }
            let inner = &trimmed[1..trimmed.len() - 1];
            if let Some(rest) = inner.strip_prefix("mcp_servers.")
                && !rest.contains('.')
                && is_valid_mcp_server_name(rest)
            {
                current = Some(rest.to_owned());
                current_cmd.clear();
            }
        } else if current.is_some()
            && let Some((k, v)) = trimmed.split_once('=')
        {
            let key = k.trim();
            if (key == "command" || key == "url") && current_cmd.is_empty() {
                current_cmd = v.trim().trim_matches('"').trim_matches('\'').to_owned();
            }
        }
    }
    if let Some(prev) = current {
        found.push((
            prev,
            if current_cmd.is_empty() {
                "toml".into()
            } else {
                current_cmd
            },
        ));
    }
    found
}

pub fn upsert_json_mcp_server(
    config_path: &Path,
    agent_id: &str,
    server_name: &str,
    transport: McpTransport,
    command_or_url: &str,
    args: &[String],
    env_pairs: &[(String, String)],
) -> Result<(), String> {
    if !is_valid_mcp_server_name(server_name) {
        return Err("MCP server name must match [A-Za-z0-9_-]{1,64}.".into());
    }
    let endpoint = command_or_url.trim();
    if endpoint.is_empty() || endpoint.contains(['\0', '\r', '\n']) {
        return Err("MCP server command or URL cannot be empty.".into());
    }
    if transport == McpTransport::Http {
        let parsed = url::Url::parse(endpoint).map_err(|_| "Invalid MCP HTTP URL.")?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("MCP HTTP URL must use http:// or https://.".into());
        }
    }
    let mut root: serde_json::Value = if config_path.is_file() {
        let existing = read_bounded_utf8(config_path, 512 * 1024)
            .ok_or("Cannot read existing MCP configuration file.")?;
        // Write a .tessel-bak backup before modifying an existing config.
        let bak = config_path.with_extension("tessel-bak");
        let _ = fs::copy(config_path, bak);
        serde_json::from_str(&existing).map_err(|_| "Existing MCP config is not valid JSON.")?
    } else {
        serde_json::json!({})
    };

    let servers_key = if agent_id == "opencode" {
        "mcp"
    } else {
        "mcpServers"
    };
    let root_obj = root
        .as_object_mut()
        .ok_or("MCP configuration root must be a JSON object.")?;
    let servers_val = root_obj
        .entry(servers_key.to_owned())
        .or_insert_with(|| serde_json::json!({}));
    let servers_obj = servers_val
        .as_object_mut()
        .ok_or("mcpServers field must be a JSON object.")?;

    let mut entry = serde_json::Map::new();
    match transport {
        McpTransport::Http => {
            entry.insert("url".into(), serde_json::Value::String(endpoint.to_owned()));
        }
        McpTransport::Stdio => {
            entry.insert(
                "command".into(),
                serde_json::Value::String(endpoint.to_owned()),
            );
            entry.insert(
                "args".into(),
                serde_json::Value::Array(
                    args.iter()
                        .map(|a| serde_json::Value::String(a.clone()))
                        .collect(),
                ),
            );
        }
    }
    if !env_pairs.is_empty() {
        let mut env_map = serde_json::Map::new();
        for (k, v) in env_pairs {
            if !k.trim().is_empty() {
                env_map.insert(k.trim().to_owned(), serde_json::Value::String(v.clone()));
            }
        }
        if !env_map.is_empty() {
            entry.insert("env".into(), serde_json::Value::Object(env_map));
        }
    }
    servers_obj.insert(server_name.to_owned(), serde_json::Value::Object(entry));

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let encoded = serde_json::to_vec_pretty(&root).map_err(|e| e.to_string())?;
    let tmp = config_path.with_extension("json.tmp");
    fs::write(&tmp, &encoded).map_err(|e| e.to_string())?;
    fs::rename(&tmp, config_path).map_err(|e| e.to_string())
}

pub fn remove_json_mcp_server(
    config_path: &Path,
    agent_id: &str,
    server_name: &str,
) -> Result<(), String> {
    let existing =
        read_bounded_utf8(config_path, 512 * 1024).ok_or("MCP configuration file not found.")?;
    let mut root: serde_json::Value =
        serde_json::from_str(&existing).map_err(|_| "Invalid JSON in MCP configuration.")?;
    let servers_key = if agent_id == "opencode" {
        "mcp"
    } else {
        "mcpServers"
    };
    if let Some(obj) = root
        .get_mut(servers_key)
        .and_then(serde_json::Value::as_object_mut)
    {
        let bak = config_path.with_extension("tessel-bak");
        let _ = fs::copy(config_path, bak);
        obj.remove(server_name);
        let encoded = serde_json::to_vec_pretty(&root).map_err(|e| e.to_string())?;
        let tmp = config_path.with_extension("json.tmp");
        fs::write(&tmp, &encoded).map_err(|e| e.to_string())?;
        fs::rename(&tmp, config_path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Bounded stdio JSON-RPC 2.0 `initialize` probe for an MCP server.
pub fn probe_mcp_stdio(command: &str, args: &[String]) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd.exe");
        c.arg("/C").arg(command).args(args);
        c
    } else {
        let mut c = Command::new(command);
        c.args(args);
        c
    };
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Spawn failed: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let init_req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"tessel-rust","version":"0.1.0"}}}"#;
        let _ = writeln!(stdin, "{init_req}");
        let _ = stdin.flush();
    }
    let stdout = child.stdout.take().ok_or("No stdout pipe.")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let res = reader.read_line(&mut line).map(|_| line);
        let _ = tx.send(res);
    });
    let reply = rx
        .recv_timeout(std::time::Duration::from_secs(12))
        .map_err(|_| {
            let _ = child.kill();
            "Timed out waiting for MCP initialize response.".to_string()
        })?
        .map_err(|e| {
            let _ = child.kill();
            format!("Read failed: {e}")
        })?;
    let _ = child.kill();
    let _ = child.wait();
    if reply.contains("\"result\"") {
        Ok("Connected (JSON-RPC initialize OK)".into())
    } else if reply.trim().is_empty() {
        Err("Server exited without a JSON-RPC response.".into())
    } else {
        Ok(format!("Server responded ({} bytes)", reply.trim().len()))
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpProbeOutcome {
    Connected(String),
    SignsInFromAgent,
}

impl McpProbeOutcome {
    #[allow(dead_code)]
    #[must_use]
    pub fn signs_in_badge(french: bool) -> &'static str {
        if french {
            "Connexion par l'agent"
        } else {
            "Signs in from the agent"
        }
    }

    #[allow(dead_code)]
    #[must_use]
    pub fn signs_in_tooltip(french: bool) -> &'static str {
        if french {
            "Le serveur demande une connexion. Le test de Tessel n'a pas de jeton : il ne peut pas savoir si l'agent s'est déjà connecté, chaque agent garde le sien."
        } else {
            "The server asks for a sign-in. Tessel's test carries no token, so it cannot tell whether the agent has already signed in: each agent keeps its own."
        }
    }
}

#[allow(dead_code)]
pub fn classify_mcp_http_status(name: &str, status: u16) -> Result<McpProbeOutcome, String> {
    match status {
        401 | 403 => Ok(McpProbeOutcome::SignsInFromAgent),
        200..=299 => Ok(McpProbeOutcome::Connected(format!(
            "{name}: connected (HTTP {status})"
        ))),
        _ => Err(format!("{name}: HTTP {status}")),
    }
}

/// Bounded HTTP/SSE MCP endpoint probe. Treats HTTP `401` and `403` as [`McpProbeOutcome::SignsInFromAgent`].
#[allow(dead_code)]
pub fn probe_mcp_http(name: &str, endpoint: &str) -> Result<McpProbeOutcome, String> {
    let parsed = url::Url::parse(endpoint).map_err(|_| "Invalid HTTP MCP URL.".to_owned())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Invalid HTTP MCP URL.".into());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let init_req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"tessel-rust","version":"0.1.0"}}}"#;
    let resp = client
        .post(parsed.clone())
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .body(init_req)
        .send()
        .map_err(|e| format!("{name}: {e}"))?;
    let code = resp.status().as_u16();
    if code == 405 {
        let get_resp = client
            .get(parsed)
            .header("Accept", "text/event-stream, application/json")
            .send()
            .map_err(|e| format!("{name}: {e}"))?;
        return classify_mcp_http_status(name, get_resp.status().as_u16());
    }
    classify_mcp_http_status(name, code)
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpSignInStep {
    TerminalCommand(String),
    CardMessage(String),
}

#[allow(dead_code)]
pub fn mcp_agent_sign_in_step(
    agent_id: &str,
    agent_label: &str,
    server_name: &str,
    french: bool,
) -> Result<McpSignInStep, String> {
    if !is_valid_mcp_server_name(server_name) {
        return Err(if french {
            "Nom de serveur MCP invalide ([A-Za-z0-9_-], max 64).".into()
        } else {
            "Invalid MCP server name ([A-Za-z0-9_-], max 64).".into()
        });
    }
    match agent_id {
        "codex" => Ok(McpSignInStep::TerminalCommand(format!(
            "codex mcp login {server_name}"
        ))),
        "opencode" => Ok(McpSignInStep::TerminalCommand(format!(
            "opencode mcp auth {server_name}"
        ))),
        "claude" | "claude-project" => Ok(McpSignInStep::CardMessage(if french {
            format!(
                "Dans un panneau Claude Code, tapez /mcp, choisissez « {server_name} » puis Authenticate. Votre navigateur s'ouvre pour vous connecter."
            )
        } else {
            format!(
                "In a Claude Code pane, type /mcp, pick \"{server_name}\" and choose Authenticate. Your browser opens to sign in."
            )
        })),
        "gemini" | "qwen" => {
            let disp = match agent_id {
                "gemini" => "Gemini CLI",
                "qwen" => "Qwen Code",
                _ => agent_label,
            };
            Ok(McpSignInStep::CardMessage(if french {
                format!(
                    "Dans un panneau {disp}, tapez /mcp auth {server_name}. Votre navigateur s'ouvre pour vous connecter."
                )
            } else {
                format!(
                    "In a {disp} pane, type /mcp auth {server_name}. Your browser opens to sign in."
                )
            }))
        }
        _ => {
            let disp = if agent_label.trim().is_empty() {
                agent_id
            } else {
                agent_label
            };
            Ok(McpSignInStep::CardMessage(if french {
                format!(
                    "Connectez-vous à « {server_name} » depuis un panneau {disp} (avec sa propre commande MCP), ou définissez la clé d'API du serveur."
                )
            } else {
                format!(
                    "Sign in to \"{server_name}\" from a {disp} pane (its own MCP command), or set the server's API key."
                )
            }))
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpSignInPlan {
    pub pane_title: String,
    pub commands: Vec<String>,
    pub card_message: Option<String>,
}

impl McpSignInPlan {
    #[allow(dead_code)]
    #[must_use]
    pub fn chained_command(&self, powershell_style: bool) -> Option<String> {
        if self.commands.is_empty() {
            None
        } else {
            Some(
                self.commands
                    .join(if powershell_style { "; " } else { " && " }),
            )
        }
    }
}

#[allow(dead_code)]
pub fn plan_mcp_sign_in(
    agents: &[(String, String)],
    server_name: &str,
    french: bool,
) -> Result<McpSignInPlan, String> {
    if !is_valid_mcp_server_name(server_name) {
        return Err(if french {
            "Nom de serveur MCP invalide ([A-Za-z0-9_-], max 64).".into()
        } else {
            "Invalid MCP server name ([A-Za-z0-9_-], max 64).".into()
        });
    }
    let mut commands = Vec::new();
    let mut messages = Vec::new();
    for (agent_id, agent_label) in agents {
        match mcp_agent_sign_in_step(agent_id, agent_label, server_name, french)? {
            McpSignInStep::TerminalCommand(cmd) => {
                if !commands.contains(&cmd) {
                    commands.push(cmd);
                }
            }
            McpSignInStep::CardMessage(msg) => {
                if !messages.contains(&msg) {
                    messages.push(msg);
                }
            }
        }
    }
    Ok(McpSignInPlan {
        pane_title: if french {
            format!("Connexion : {server_name}")
        } else {
            format!("Sign in: {server_name}")
        },
        commands,
        card_message: (!messages.is_empty()).then(|| messages.join("\n")),
    })
}

/// Retires the legacy `google-workspace` MCP server from agents holding `tessel_apps`
/// once the Tessel Google connector is active (`<data_dir>/tessel-apps/old-google-retired.json`).
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct RetireOldGoogleTracker {
    prev_connected: Option<bool>,
    done: std::collections::BTreeSet<String>,
    refused: std::collections::BTreeSet<String>,
    loaded_from_disk: bool,
}

impl RetireOldGoogleTracker {
    #[allow(dead_code)]
    #[must_use]
    pub fn state_file_path(data_dir: &Path) -> PathBuf {
        data_dir.join("tessel-apps").join("old-google-retired.json")
    }

    fn load_done_from_disk(&mut self, data_dir: &Path) {
        let path = Self::state_file_path(data_dir);
        if let Some(raw) = read_bounded_utf8(&path, 64 * 1024)
            && let Ok(list) = serde_json::from_str::<Vec<String>>(&raw)
        {
            self.done = list.into_iter().collect();
        }
        self.loaded_from_disk = true;
    }

    fn save_done_to_disk(&self, data_dir: &Path) {
        let path = Self::state_file_path(data_dir);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let vec: Vec<&String> = self.done.iter().collect();
        if let Ok(bytes) = serde_json::to_vec_pretty(&vec) {
            let tmp = path.with_extension("json.tmp");
            if fs::write(&tmp, &bytes).is_ok() {
                let _ = fs::rename(&tmp, &path);
            }
        }
    }

    /// Runs the retirement pass using `remove_fn` (returning `true` if at least one MCP config changed).
    #[allow(dead_code)]
    pub fn sync_with_remover<F>(
        &mut self,
        data_dir: &Path,
        google_connected: bool,
        installed: &[InstalledMcpServer],
        mut remove_fn: F,
    ) -> bool
    where
        F: FnMut(&Path, &str, &str) -> Result<(), String>,
    {
        if !google_connected {
            self.prev_connected = Some(false);
            return false;
        }
        let just_connected = self.prev_connected == Some(false);
        self.prev_connected = Some(true);
        if just_connected {
            self.refused.clear();
            self.done.clear();
        } else if !self.loaded_from_disk {
            self.load_done_from_disk(data_dir);
        }

        // Only consider agents that hold `tessel_apps` (e.g. Cursor never holds `tessel_apps`,
        // so Cursor keeps `google-workspace`).
        let mut tessel_apps_holders: Vec<(String, PathBuf)> = Vec::new();
        for srv in installed.iter().filter(|s| s.name == "tessel_apps") {
            if !tessel_apps_holders
                .iter()
                .any(|(id, p)| id == &srv.agent_id && p == &srv.config_path)
            {
                tessel_apps_holders.push((srv.agent_id.clone(), srv.config_path.clone()));
            }
        }

        let mut removed_any = false;
        let mut disk_dirty = false;
        for (agent_id, cfg_path) in tessel_apps_holders {
            if self.done.contains(&agent_id) || self.refused.contains(&agent_id) {
                continue;
            }
            let has_old_google = installed.iter().any(|s| {
                s.agent_id == agent_id && s.config_path == cfg_path && s.name == "google-workspace"
            });
            if has_old_google {
                match remove_fn(&cfg_path, &agent_id, "google-workspace") {
                    Ok(()) => {
                        self.done.insert(agent_id);
                        removed_any = true;
                        disk_dirty = true;
                    }
                    Err(_) => {
                        self.refused.insert(agent_id);
                    }
                }
            } else {
                self.done.insert(agent_id);
                disk_dirty = true;
            }
        }
        if disk_dirty {
            self.save_done_to_disk(data_dir);
        }
        removed_any
    }

    #[allow(dead_code)]
    pub fn sync(
        &mut self,
        data_dir: &Path,
        google_connected: bool,
        installed: &[InstalledMcpServer],
    ) -> bool {
        self.sync_with_remover(
            data_dir,
            google_connected,
            installed,
            remove_json_mcp_server,
        )
    }
}

// ---------------------------------------------------------------------------
// 8. Tessel Connectors Catalog (`Connections` Page)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct TesselConnectorSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub read_only: bool,
    pub available: bool,
    pub summary_en: &'static str,
    pub summary_fr: &'static str,
}

pub const TESSEL_CONNECTORS: [TesselConnectorSpec; 10] = [
    TesselConnectorSpec {
        id: "google",
        name: "Google (Gmail, Calendar, Drive)",
        read_only: false,
        available: true,
        summary_en: "Search mail, create drafts, read Drive files, and manage calendar events.",
        summary_fr: "Rechercher des courriels, créer des brouillons, lire Drive et gérer l'agenda.",
    },
    TesselConnectorSpec {
        id: "microsoft",
        name: "Microsoft (Outlook, OneDrive)",
        read_only: false,
        available: false,
        summary_en: "Outlook mail, calendar events, and OneDrive documents.",
        summary_fr: "Courriels Outlook, calendrier et documents OneDrive.",
    },
    TesselConnectorSpec {
        id: "github",
        name: "GitHub",
        read_only: false,
        available: false,
        summary_en: "Pull requests, issues, checks, and repository actions.",
        summary_fr: "Pull requests, tickets, vérifications CI et dépôts.",
    },
    TesselConnectorSpec {
        id: "dropbox",
        name: "Dropbox",
        read_only: false,
        available: false,
        summary_en: "Browse, inspect, and organize cloud folders in Dropbox.",
        summary_fr: "Parcourir, inspecter et organiser les dossiers Dropbox.",
    },
    TesselConnectorSpec {
        id: "notion",
        name: "Notion",
        read_only: false,
        available: false,
        summary_en: "Search pages, databases, and workspace knowledge bases.",
        summary_fr: "Rechercher des pages, bases de données et notes Notion.",
    },
    TesselConnectorSpec {
        id: "linear",
        name: "Linear",
        read_only: false,
        available: false,
        summary_en: "Read and update engineering issues, cycles, and roadmaps.",
        summary_fr: "Lire et mettre à jour les tickets, cycles et feuilles de route Linear.",
    },
    TesselConnectorSpec {
        id: "atlassian",
        name: "Atlassian (Jira & Confluence)",
        read_only: false,
        available: false,
        summary_en: "Jira issues, sprints, and Confluence documentation.",
        summary_fr: "Tickets Jira, sprints et documentation Confluence.",
    },
    TesselConnectorSpec {
        id: "sentry",
        name: "Sentry",
        read_only: false,
        available: false,
        summary_en: "Inspect live crash reports, stack traces, and alerts.",
        summary_fr: "Inspecter les rapports d'erreurs, traces et alertes Sentry.",
    },
    TesselConnectorSpec {
        id: "vercel",
        name: "Vercel",
        read_only: false,
        available: false,
        summary_en: "Preview deployments, domains, and build logs.",
        summary_fr: "Déploiements d'aperçu, domaines et journaux de build Vercel.",
    },
    TesselConnectorSpec {
        id: "stripe",
        name: "Stripe",
        read_only: true,
        available: false,
        summary_en: "Read-only billing, subscription, and invoice inspection.",
        summary_fr: "Inspection en lecture seule de la facturation et des abonnements Stripe.",
    },
];

// ---------------------------------------------------------------------------
// 9. Context Scanner & Token Estimator (`Context` Page)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextModelFamily {
    #[default]
    AnthropicNew,
    AnthropicOld,
    Google,
    Xai,
    OpenAi,
    Other,
}

impl ContextModelFamily {
    pub const ALL: [Self; 6] = [
        Self::AnthropicNew,
        Self::AnthropicOld,
        Self::Google,
        Self::Xai,
        Self::OpenAi,
        Self::Other,
    ];

    #[must_use]
    pub const fn factor(self) -> f64 {
        match self {
            Self::AnthropicNew => 1.35,
            Self::AnthropicOld => 1.20,
            Self::Google => 1.10,
            Self::Xai => 1.05,
            Self::OpenAi => 1.00,
            Self::Other => 1.10,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::AnthropicNew => "Claude 3.7 / 4 (×1.35)",
            Self::AnthropicOld => "Claude 3.5 (×1.20)",
            Self::Google => "Gemini (×1.10)",
            Self::Xai => "Grok / xAI (×1.05)",
            Self::OpenAi => "OpenAI o3 / GPT-4o (×1.00)",
            Self::Other => "Other models (×1.10)",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ContextPreferences {
    pub model_family: ContextModelFamily,
}

#[derive(Clone, Debug)]
pub struct ContextItem {
    pub label: String,
    pub bytes: usize,
    pub estimated_tokens: usize,
}

#[derive(Clone, Debug, Default)]
pub struct ContextScanReport {
    pub rules_files: Vec<ContextItem>,
    pub skills_items: Vec<ContextItem>,
    pub mcp_items: Vec<ContextItem>,
    pub team_rule_tokens: usize,
    pub total_tokens: usize,
}

#[must_use]
pub fn estimate_tokens(text_len_bytes: usize, family: ContextModelFamily) -> usize {
    let base = (text_len_bytes as f64) / 4.0;
    (base * family.factor()).ceil() as usize
}

fn is_sensitive_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower == "id_rsa"
        || lower == "id_ed25519"
}

#[must_use]
pub fn scan_project_context(
    home_dir: &Path,
    project_dir: Option<&Path>,
    skills: &[SkillEntry],
    mcp_servers: &[InstalledMcpServer],
    family: ContextModelFamily,
) -> ContextScanReport {
    const MAX_PER_FILE: usize = 256 * 1024;
    const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;
    const MAX_FILES: usize = 100;

    let mut report = ContextScanReport::default();
    let mut total_bytes = 0usize;
    let mut file_count = 0usize;

    let mut candidate_paths: Vec<(String, PathBuf)> = vec![
        (
            "~/.claude/CLAUDE.md".into(),
            home_dir.join(".claude").join("CLAUDE.md"),
        ),
        (
            "~/.gemini/GEMINI.md".into(),
            home_dir.join(".gemini").join("GEMINI.md"),
        ),
        (
            "~/.codex/AGENTS.md".into(),
            home_dir.join(".codex").join("AGENTS.md"),
        ),
    ];
    if let Some(proj) = project_dir {
        let proj_str = proj.to_string_lossy();
        // Reject UNC network paths to avoid blocking network I/O.
        if !proj_str.starts_with("\\\\") || proj_str.starts_with("\\\\?\\") {
            for rel in [
                "CLAUDE.md",
                "AGENTS.md",
                "GEMINI.md",
                "QWEN.md",
                ".cursorrules",
                ".clinerules",
                ".github/copilot-instructions.md",
            ] {
                candidate_paths.push((rel.to_owned(), proj.join(rel)));
            }
            let cursor_rules_dir = proj.join(".cursor").join("rules");
            if let Ok(entries) = fs::read_dir(&cursor_rules_dir) {
                for entry in entries.flatten().take(32) {
                    let p = entry.path();
                    if let Some(fname) = p.file_name().and_then(|n| n.to_str())
                        && !is_sensitive_filename(fname)
                    {
                        candidate_paths.push((format!(".cursor/rules/{fname}"), p));
                    }
                }
            }
        }
    }

    for (label, path) in candidate_paths {
        if file_count >= MAX_FILES || total_bytes >= MAX_TOTAL_BYTES {
            break;
        }
        if let Some(fname) = path.file_name().and_then(|n| n.to_str())
            && is_sensitive_filename(fname)
        {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if is_symlink_or_reparse(&meta) || !meta.is_file() {
            continue;
        }
        let Some(content) = read_bounded_utf8(&path, MAX_PER_FILE) else {
            continue;
        };
        let b = content.len().min(MAX_TOTAL_BYTES - total_bytes);
        total_bytes += b;
        file_count += 1;
        report.rules_files.push(ContextItem {
            label,
            bytes: b,
            estimated_tokens: estimate_tokens(b, family),
        });
    }

    for skill in skills.iter().take(200) {
        let listing_bytes = skill.name.len() + skill.description.len() + 24;
        report.skills_items.push(ContextItem {
            label: format!("{} ({})", skill.name, skill.agent_label),
            bytes: listing_bytes,
            estimated_tokens: estimate_tokens(listing_bytes, family),
        });
    }

    for srv in mcp_servers.iter().take(100) {
        // Average upfront MCP tool schema overhead per server (~600 bytes)
        let schema_bytes = 600 + srv.name.len() + srv.endpoint_summary.len();
        report.mcp_items.push(ContextItem {
            label: format!("{} · {}", srv.name, srv.agent_label),
            bytes: schema_bytes,
            estimated_tokens: estimate_tokens(schema_bytes, family),
        });
    }

    // Tessel's standing task-board & team tools rule (~640 bytes)
    report.team_rule_tokens = estimate_tokens(640, family);
    report.total_tokens = report
        .rules_files
        .iter()
        .chain(report.skills_items.iter())
        .chain(report.mcp_items.iter())
        .map(|i| i.estimated_tokens)
        .sum::<usize>()
        + report.team_rule_tokens;

    report
}

// ---------------------------------------------------------------------------
// 10. Developer Tools Catalog & Probe (`Devtools` Page)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct DevToolSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub binary: &'static str,
    pub version_arg: &'static str,
    pub winget_id: &'static str,
    pub docs_url: &'static str,
    pub desc_en: &'static str,
    pub desc_fr: &'static str,
}

pub const DEVTOOLS_CATALOG: [DevToolSpec; 11] = [
    DevToolSpec {
        id: "git",
        name: "Git",
        binary: "git",
        version_arg: "--version",
        winget_id: "Git.Git",
        docs_url: "https://git-scm.com/",
        desc_en: "Distributed version control, branches, and worktrees.",
        desc_fr: "Gestion de versions distribuée, branches et worktrees.",
    },
    DevToolSpec {
        id: "gh",
        name: "GitHub CLI",
        binary: "gh",
        version_arg: "--version",
        winget_id: "GitHub.cli",
        docs_url: "https://cli.github.com/",
        desc_en: "Pull requests, issues, and CI checks from Tessel and terminals.",
        desc_fr: "Pull requests, tickets et vérifications CI depuis Tessel.",
    },
    DevToolSpec {
        id: "node",
        name: "Node.js (LTS)",
        binary: "node",
        version_arg: "--version",
        winget_id: "OpenJS.NodeJS.LTS",
        docs_url: "https://nodejs.org/",
        desc_en: "JavaScript runtime required by npm-based CLI agents and MCP servers.",
        desc_fr: "Exécutable JavaScript requis par les agents CLI npm et serveurs MCP.",
    },
    DevToolSpec {
        id: "python",
        name: "Python 3",
        binary: "python",
        version_arg: "--version",
        winget_id: "Python.Python.3.12",
        docs_url: "https://www.python.org/",
        desc_en: "Python interpreter for scripts, tooling, and data workflows.",
        desc_fr: "Interpréteur Python pour scripts, outils et données.",
    },
    DevToolSpec {
        id: "uv",
        name: "uv",
        binary: "uv",
        version_arg: "--version",
        winget_id: "astral-sh.uv",
        docs_url: "https://docs.astral.sh/uv/",
        desc_en: "Fast Python package & tool runner (`uvx` for Python MCP servers).",
        desc_fr: "Gestionnaire Python rapide (`uvx` pour les serveurs MCP Python).",
    },
    DevToolSpec {
        id: "rg",
        name: "ripgrep (rg)",
        binary: "rg",
        version_arg: "--version",
        winget_id: "BurntSushi.ripgrep.MSVC",
        docs_url: "https://github.com/BurntSushi/ripgrep",
        desc_en: "Ultra-fast recursive code search used by coding agents.",
        desc_fr: "Recherche de code ultra-rapide utilisée par les agents.",
    },
    DevToolSpec {
        id: "pwsh",
        name: "PowerShell 7",
        binary: "pwsh",
        version_arg: "--version",
        winget_id: "Microsoft.PowerShell",
        docs_url: "https://learn.microsoft.com/powershell/",
        desc_en: "Modern cross-platform PowerShell shell.",
        desc_fr: "Shell PowerShell 7 moderne multiplateforme.",
    },
    DevToolSpec {
        id: "code",
        name: "Visual Studio Code",
        binary: "code",
        version_arg: "--version",
        winget_id: "Microsoft.VisualStudioCode",
        docs_url: "https://code.visualstudio.com/",
        desc_en: "External code editor (`code` CLI).",
        desc_fr: "Éditeur externe Visual Studio Code (`code`).",
    },
    DevToolSpec {
        id: "docker",
        name: "Docker Desktop",
        binary: "docker",
        version_arg: "--version",
        winget_id: "Docker.DockerDesktop",
        docs_url: "https://www.docker.com/products/docker-desktop/",
        desc_en: "Container runtime and Compose tooling.",
        desc_fr: "Exécution de conteneurs et Docker Compose.",
    },
    DevToolSpec {
        id: "bun",
        name: "Bun",
        binary: "bun",
        version_arg: "--version",
        winget_id: "Oven-sh.Bun",
        docs_url: "https://bun.sh/",
        desc_en: "Fast all-in-one JavaScript/TypeScript runtime and bundler.",
        desc_fr: "Runtime et gestionnaire JavaScript/TypeScript rapide.",
    },
    DevToolSpec {
        id: "jq",
        name: "jq",
        binary: "jq",
        version_arg: "--version",
        winget_id: "jqlang.jq",
        docs_url: "https://jqlang.github.io/jq/",
        desc_en: "Command-line JSON processor.",
        desc_fr: "Processeur JSON en ligne de commande.",
    },
];

#[derive(Clone, Debug, Default)]
pub struct DevToolStatus {
    pub id: String,
    pub installed: bool,
    pub version: String,
    pub extra_note: String,
}

#[must_use]
pub fn winget_install_command(winget_id: &str) -> String {
    format!(
        "winget install --id {winget_id} --exact --source winget --accept-package-agreements --accept-source-agreements"
    )
}

#[must_use]
pub fn probe_devtools() -> Vec<DevToolStatus> {
    DEVTOOLS_CATALOG
        .iter()
        .map(|spec| {
            let version = run_short_stdout(spec.binary, &[spec.version_arg]);
            let installed = version.is_some();
            let first_line = version
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_owned();
            let extra_note = if installed && spec.id == "git" {
                let name = run_short_stdout("git", &["config", "--global", "user.name"])
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                let email = run_short_stdout("git", &["config", "--global", "user.email"])
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if name.is_empty() || email.is_empty() {
                    "identity-missing".into()
                } else {
                    format!("{name} <{email}>")
                }
            } else if installed && spec.id == "gh" {
                if run_short_stdout("gh", &["auth", "status"]).is_some() {
                    "authenticated".into()
                } else {
                    "auth-missing".into()
                }
            } else {
                String::new()
            };
            DevToolStatus {
                id: spec.id.to_owned(),
                installed,
                version: first_line,
                extra_note,
            }
        })
        .collect()
}

fn run_short_stdout(bin: &str, args: &[&str]) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(windows) && matches!(bin, "code" | "npm" | "npx") {
        let mut c = Command::new("cmd.exe");
        c.arg("/C").arg(bin).args(args);
        c
    } else {
        let mut c = Command::new(bin);
        c.args(args);
        c
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn read_bounded_utf8(path: &Path, max_bytes: usize) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(max_bytes as u64).read_to_end(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn custom_identifier_entropy_and_collision_errors_never_return_fallback_ids() {
        assert!(
            generate_custom_agent_id_with(&[], |bytes| {
                bytes.fill(0xA5);
                Err("Synthetic entropy failure".into())
            })
            .is_err()
        );
        let fill = |bytes: &mut [u8]| {
            bytes.fill(0xA5);
            Ok(())
        };
        let id = generate_custom_agent_id_with(&[], fill).unwrap();
        let agents = vec![CustomAgent {
            id: id.clone(),
            name: "Existing".into(),
            command: "synthetic-cli".into(),
            accent: "#ffffff".into(),
            enabled: true,
        }];
        assert!(
            generate_custom_agent_id_with(&agents, fill)
                .unwrap_err()
                .contains("already exists")
        );
        assert_eq!(agents[0].id, id);
        assert_eq!(agents[0].command, "synthetic-cli");
    }

    #[test]
    fn managed_account_entropy_failure_and_collisions_preserve_existing_profiles() {
        let dir = tempfile::tempdir().unwrap();
        for provider in ["codex", "claude"] {
            let data = dir.path().join(provider);
            let failed = add_managed_account_with(&data, provider, "Synthetic", |bytes| {
                bytes.fill(0xA5);
                Err("Synthetic entropy failure".into())
            });
            assert!(failed.unwrap_err().contains("entropy"));
            assert!(!data.exists(), "entropy failure must not create any files");
            let fill = |bytes: &mut [u8]| {
                bytes.fill(0xA5);
                Ok(())
            };
            let account = add_managed_account_with(&data, provider, "First", fill).unwrap();
            let profile = managed_account_profile_dir(&data, provider, &account.id).unwrap();
            fs::write(profile.join("marker"), "synthetic profile").unwrap();
            let root = managed_accounts_root(&data, provider).unwrap();
            let roster_before = fs::read(root.join("accounts.json")).unwrap();
            assert!(add_managed_account_with(&data, provider, "Second", fill).is_err());
            assert_eq!(fs::read(root.join("accounts.json")).unwrap(), roster_before);
            assert_eq!(
                fs::read_to_string(profile.join("marker")).unwrap(),
                "synthetic profile"
            );
            // An orphan directory absent from the roster must also not be reused.
            let orphan =
                managed_account_profile_dir(&data, provider, "acct-bbbbbbbbbbbbbbbb").unwrap();
            fs::create_dir_all(&orphan).unwrap();
            fs::write(orphan.join("marker"), "orphan profile").unwrap();
            assert!(
                add_managed_account_with(&data, provider, "Third", |bytes| {
                    bytes.fill(0xBB);
                    Ok(())
                })
                .is_err()
            );
            assert_eq!(
                fs::read_to_string(orphan.join("marker")).unwrap(),
                "orphan profile"
            );
            assert_eq!(fs::read(root.join("accounts.json")).unwrap(), roster_before);
        }
    }

    use super::*;

    #[test]
    fn custom_agent_validation_enforces_limits_swatches_and_uniqueness() {
        let a1 = CustomAgent {
            id: "custom-a1b2c3".into(),
            name: "My Runner".into(),
            command: "my-agent --fast".into(),
            accent: "#5b8def".into(),
            enabled: true,
        };
        assert!(validate_custom_agents(std::slice::from_ref(&a1)).is_ok());

        let mut dup = a1.clone();
        dup.id = "custom-d4e5f6".into();
        dup.name = "my runner".into();
        assert!(validate_custom_agents(&[a1.clone(), dup]).is_err());

        let builtin_conflict = CustomAgent {
            id: "custom-999999".into(),
            name: "Claude".into(),
            command: "claude".into(),
            accent: "#5b8def".into(),
            enabled: true,
        };
        assert!(validate_custom_agents(&[builtin_conflict]).is_err());
    }

    #[test]
    fn skill_name_safety_and_frontmatter_parsing() {
        assert!(is_safe_skill_name("chrome-devtools"));
        assert!(is_safe_skill_name("my_skill.v2"));
        assert!(!is_safe_skill_name("../escape"));
        assert!(!is_safe_skill_name(".hidden"));
        assert!(!is_safe_skill_name("CON"));
        assert!(!is_safe_skill_name("trailing."));

        let md =
            "---\nname: \"a11y-audit\"\ndescription: Checks semantic HTML and ARIA\n---\n# Body\n";
        let (name, desc) = parse_skill_frontmatter(md);
        assert_eq!(name.as_deref(), Some("a11y-audit"));
        assert_eq!(desc.as_deref(), Some("Checks semantic HTML and ARIA"));
    }

    #[test]
    fn mcp_catalog_has_all_30_entries_and_json_upsert_writes_backup() {
        assert_eq!(MCP_CATALOG.len(), 30);
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join(".mcp.json");
        upsert_json_mcp_server(
            &cfg,
            "claude-project",
            "memory",
            McpTransport::Stdio,
            "npx",
            &["-y".into(), "@modelcontextprotocol/server-memory".into()],
            &[],
        )
        .unwrap();
        upsert_json_mcp_server(
            &cfg,
            "claude-project",
            "linear",
            McpTransport::Http,
            "https://mcp.linear.app/mcp",
            &[],
            &[],
        )
        .unwrap();
        assert!(dir.path().join(".mcp.tessel-bak").is_file());
        let scanned = scan_installed_mcp_servers(dir.path(), Some(dir.path()));
        assert_eq!(scanned.len(), 2);
        remove_json_mcp_server(&cfg, "claude-project", "memory").unwrap();
        let scanned_after = scan_installed_mcp_servers(dir.path(), Some(dir.path()));
        assert_eq!(scanned_after.len(), 1);
        assert_eq!(scanned_after[0].name, "linear");
    }

    #[test]
    fn context_scanner_ignores_env_secrets_and_scales_by_model_family() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("AGENTS.md"),
            "# Project Rules\nAlways run cargo test.\n",
        )
        .unwrap();
        fs::write(dir.path().join(".env"), "SUPER_SECRET=1234567890").unwrap();
        let openai = scan_project_context(
            dir.path(),
            Some(dir.path()),
            &[],
            &[],
            ContextModelFamily::OpenAi,
        );
        let claude = scan_project_context(
            dir.path(),
            Some(dir.path()),
            &[],
            &[],
            ContextModelFamily::AnthropicNew,
        );
        assert_eq!(openai.rules_files.len(), 1);
        assert_eq!(openai.rules_files[0].label, "AGENTS.md");
        assert!(claude.total_tokens > openai.total_tokens);
    }
}
