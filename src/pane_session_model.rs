// Adapted from Tessel agentSessionOptions.js, agentPrefs.js, paneLastModel.js,
// modelLabel.js, sessionOptionLabels.js, native-chat-session-option-labels.js,
// cliRequests.js, claudePricing.js, claudeModelSwitch.js, cursorModels.js,
// codexScreen.js, antigravityModels.js, screenModel.js, TerminalPane.vue,
// and Orca (github.com/stablyai/orca, MIT, Copyright (c) 2026 Lovecast Inc.;
// see THIRD_PARTY_NOTICES.md).
//! Pure per-pane session model state, persistence validation, launch signature
//! comparison, header/menu/ChatView EN+FR labels, live-switch planning across
//! all 12 cataloged agents, bounded Claude `/model` confirmation observation,
//! and bounded terminal screen model readers.
//!
//! # Scope & Non-Execution Contract
//! This module is strictly pure and never spawns a process, writes to a PTY, or
//! mutates global files. Live mid-session picks return a declarative
//! [`ModelPickPlan`] (or [`ChatSetOptionPlan`]) describing any slash command to
//! type, whether Claude's default-model file hold is needed, toast feedback,
//! and updated per-pane state.
//!
//! # Observer & Screen-Reader Limitations
//! - [`ClaudeModelSwitchObserver`] inspects a bounded sliding window of at most
//!   [`MAX_CLAUDE_OBSERVED_UTF16`] UTF-16 code units over a 5 s timeout
//!   ([`CLAUDE_SWITCH_TIMEOUT_MS`]). It auto-submits `\r` at most once when
//!   Claude Code prompts to rebuild a cached conversation. If neither
//!   `"Set model to …"` nor `"Kept model as …"` appears within the window, the
//!   observer settles to [`ClaudeSwitchOutcome::Unknown`]; callers must surface
//!   the unverified warning rather than claiming the model changed.
//! - Terminal screen readers ([`cursor_model_on_screen`],
//!   [`cursor_context_on_screen`], [`codex_model_on_screen`],
//!   [`antigravity_model_on_screen`], [`model_from_screen`],
//!   [`muse_footer_model`]) inspect at most [`MAX_SCREEN_LINES`] lines and skip
//!   lines exceeding [`MAX_SCREEN_LINE_UTF16`] UTF-16 units. They suppress
//!   detection while an interactive picker covers the prompt, and represent
//!   heuristic TUI status-bar/banner readings ([`ModelSource::Screen`]) that
//!   always yield to transcript/session telemetry ([`ModelSource::Session`]).

use crate::{
    agent_arg_options::safe_session_value,
    agent_env::{self, js_space, trim},
    agent_model_antigravity,
    agent_model_catalog::{self as catalog, Catalog},
    agent_model_defaults::Defaults,
    agent_model_defaults_ui::{
        agent_own_default_label, decompose_cursor_model, model_choice_label, session_pill_label,
        values_on_listed_row,
    },
    agent_model_launch,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Maximum option entries inspected when sanitizing a saved pane choice (`Object.entries(v).slice(0, 10)`).
pub const MAX_PANE_OPTION_ENTRIES: usize = 10;
/// Maximum ASCII length of a per-pane option key (`VALUE_ID_RE = /^[A-Za-z][\w]{0,39}$/`).
pub const MAX_OPTION_KEY_LEN: usize = 40;
/// Maximum UTF-16 code units of a session option string value (`SAFE_VALUE`).
pub const MAX_OPTION_VALUE_UTF16: usize = 160;
/// Maximum UTF-16 code units of a saved last-known model ID or display name (`paneLastModel.js`).
pub const MAX_SAVED_MODEL_UTF16: usize = 120;
/// Default detection window (ms) for [`ClaudeModelSwitchObserver`] (`DETECTION_TIMEOUT_MS`).
pub const CLAUDE_SWITCH_TIMEOUT_MS: u64 = 5_000;
/// Sliding PTY buffer cap (in UTF-16 code units) for [`ClaudeModelSwitchObserver`] (`MAX_OBSERVED`).
pub const MAX_CLAUDE_OBSERVED_UTF16: usize = 64 * 1024;
/// Maximum number of terminal lines inspected by any bounded screen reader call.
pub const MAX_SCREEN_LINES: usize = 64;
/// Maximum UTF-16 code units accepted on a single screen line by screen readers.
pub const MAX_SCREEN_LINE_UTF16: usize = 512;
/// Maximum UTF-16 code units accepted by [`model_from_switch_line`].
pub const MAX_SWITCH_LINE_UTF16: usize = 1024;

// -----------------------------------------------------------------------------
// 1. Persistence, Validation, Effective Launch & Signatures
// -----------------------------------------------------------------------------

/// Validated per-pane session option map (`{ "model": "...", ... }`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneSessionOptions(Map<String, Value>);

impl PaneSessionOptions {
    /// Creates a minimal validated pane choice with only `"model"`.
    #[must_use]
    pub fn new(model: impl Into<String>) -> Option<Self> {
        let model = model.into();
        if !safe_session_value(&model) {
            return None;
        }
        let mut map = Map::new();
        map.insert("model".into(), Value::String(model));
        Some(Self(map))
    }

    /// Sanitizes an untrusted JSON value using [`valid_pane_session_options`].
    #[must_use]
    pub fn from_value(value: &Value) -> Option<Self> {
        valid_pane_session_options(value)
    }

    /// Selected model ID (guaranteed non-empty and `safe_session_value`).
    #[must_use]
    pub fn model(&self) -> &str {
        self.0
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    /// Selected reasoning effort, if set as a string.
    #[must_use]
    pub fn effort(&self) -> Option<&str> {
        self.0.get("effort").and_then(Value::as_str)
    }

    /// Looks up an option value by key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// Sets or removes a non-model option if the key and value satisfy safety bounds.
    pub fn set_option(&mut self, key: &str, value: Option<Value>) -> bool {
        if key == "model" || !valid_option_key(key) {
            return false;
        }
        match value {
            None => {
                self.0.remove(key);
                true
            }
            Some(val) if valid_option_scalar(&val) => {
                if self.0.contains_key(key) || self.0.len() <= MAX_PANE_OPTION_ENTRIES {
                    self.0.insert(key.to_owned(), val);
                    true
                } else {
                    false
                }
            }
            Some(_) => false,
        }
    }

    /// Borrows the underlying sanitized JSON object map.
    #[must_use]
    pub fn as_map(&self) -> &Map<String, Value> {
        &self.0
    }

    /// Clones this validated choice into a [`serde_json::Value::Object`].
    #[must_use]
    pub fn to_value(&self) -> Value {
        Value::Object(self.0.clone())
    }

    /// Converts this validated choice into a [`serde_json::Value::Object`].
    #[must_use]
    pub fn into_value(self) -> Value {
        Value::Object(self.0)
    }
}

impl Serialize for PaneSessionOptions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PaneSessionOptions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(deserializer)?;
        valid_pane_session_options(&raw)
            .ok_or_else(|| serde::de::Error::custom("invalid pane sessionOptions"))
    }
}

/// Checks `VALUE_ID_RE = /^[A-Za-z][\w]{0,39}$/`.
#[must_use]
pub fn valid_option_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    (1..=MAX_OPTION_KEY_LEN).contains(&bytes.len())
        && bytes[0].is_ascii_alphabetic()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

fn valid_option_scalar(value: &Value) -> bool {
    value.is_boolean()
        || value.as_str().is_some_and(|s| {
            let n = s.encode_utf16().count();
            (1..=MAX_OPTION_VALUE_UTF16).contains(&n) && safe_session_value(s)
        })
}

/// Sanitizes a saved per-pane choice (`agentSessionOptions.js:validPaneSessionOptions`).
///
/// Requires a valid `"model"` string; retains up to [`MAX_PANE_OPTION_ENTRIES`]
/// entries from `value`, silently dropping malformed option keys or values.
#[must_use]
pub fn valid_pane_session_options(value: &Value) -> Option<PaneSessionOptions> {
    let obj = value.as_object()?;
    let model = obj.get("model")?.as_str()?;
    let model_utf16 = model.encode_utf16().count();
    if !(1..=MAX_OPTION_VALUE_UTF16).contains(&model_utf16) || !safe_session_value(model) {
        return None;
    }
    let mut out = Map::new();
    out.insert("model".into(), Value::String(model.to_owned()));
    for (id, val) in obj.iter().take(MAX_PANE_OPTION_ENTRIES) {
        if id == "model" || !valid_option_key(id) {
            continue;
        }
        if valid_option_scalar(val) {
            out.insert(id.clone(), val.clone());
        }
    }
    Some(PaneSessionOptions(out))
}

/// Sanitizes a saved per-pane choice directly to `Option<serde_json::Value>` for
/// `SavedSession.session_options`.
#[must_use]
pub fn valid_pane_session_options_value(value: &Value) -> Option<Value> {
    valid_pane_session_options(value).map(PaneSessionOptions::into_value)
}

/// Converts a camelCase or snake_case JSON map of per-agent settings defaults
/// (`{ claude: { model, valuesByModel } }`) into `BTreeMap<String, Defaults>`.
#[must_use]
pub fn persisted_defaults_from_json(persisted: Option<&Value>) -> BTreeMap<String, Defaults> {
    let Some(obj) = persisted.and_then(Value::as_object) else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for (agent_id, entry) in obj.iter().take(60) {
        let Some(entry_obj) = entry.as_object() else {
            continue;
        };
        let model = entry_obj
            .get("model")
            .and_then(Value::as_str)
            .filter(|s| !trim(s).is_empty())
            .map(str::to_owned);
        let by_model_raw = entry_obj
            .get("valuesByModel")
            .or_else(|| entry_obj.get("values_by_model"))
            .and_then(Value::as_object);
        let mut values_by_model = BTreeMap::new();
        if let Some(by_model) = by_model_raw {
            for (m_id, vals) in by_model.iter().take(100) {
                let Some(vals_obj) = vals.as_object() else {
                    continue;
                };
                let mut inner = BTreeMap::new();
                for (k, v) in vals_obj.iter().take(MAX_PANE_OPTION_ENTRIES) {
                    if v.is_string() || v.is_boolean() {
                        inner.insert(k.clone(), v.clone());
                    }
                }
                values_by_model.insert(m_id.clone(), inner);
            }
        }
        out.insert(
            agent_id.clone(),
            Defaults {
                model,
                values_by_model,
            },
        );
    }
    out
}

/// Resolves the session values a pane launches with (`agentPrefs.js:launchSessionValues`).
///
/// An explicit pane model choice replaces the global `Settings > Agents` choice
/// as a whole (never inheriting `effort` or `fastMode` from another global model).
#[must_use]
pub fn launch_session_values(
    pane_choice: Option<&Value>,
    persisted: &BTreeMap<String, Defaults>,
    agent_id: &str,
) -> Option<Value> {
    if let Some(choice) = pane_choice
        && choice
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|m| !m.is_empty())
    {
        return Some(choice.clone());
    }
    persisted.get(agent_id).and_then(Defaults::values)
}

/// Permission mode for launching an agent pane (`"manual"` or `"yolo"`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentPermissionMode {
    #[default]
    Manual,
    Yolo,
}

impl AgentPermissionMode {
    #[must_use]
    pub fn from_str_lossy(value: &str) -> Self {
        if value == "yolo" {
            Self::Yolo
        } else {
            Self::Manual
        }
    }
}

/// Per-agent custom command/arguments/environment (`Settings > Agents`).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentPrefEntry {
    pub enabled: Option<bool>,
    pub command: String,
    pub args: String,
    pub env: String,
}

impl std::fmt::Debug for AgentPrefEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentPrefEntry")
            .field("enabled", &self.enabled)
            .field("command", &"<redacted>")
            .field("args", &"<redacted>")
            .field("env", &"<redacted>")
            .finish()
    }
}

/// Effective command, arguments, and environment for launching a pane (`agentPrefs.js:effectiveAgent`).
///
/// `Debug` redacts `command`, `args`, and `env` so sensitive launch flags/secrets
/// are never written to logs.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchSpec {
    pub command: String,
    pub args: String,
    pub env: BTreeMap<String, String>,
}

impl std::fmt::Debug for LaunchSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchSpec")
            .field("command", &"<redacted>")
            .field("args", &"<redacted>")
            .field("env_count", &self.env.len())
            .finish()
    }
}

fn yolo_args_for(agent_id: &str) -> &'static str {
    match agent_id {
        "claude" | "openclaude" | "antigravity" | "qoder" => "--dangerously-skip-permissions",
        "codex" => "--dangerously-bypass-approvals-and-sandbox",
        "gemini" | "copilot" | "kimi" | "cursor" | "crush" | "commandcode" | "rovo" | "hermes"
        | "trae" | "muse" | "ante" => "--yolo",
        "qwen" => "--approval-mode yolo",
        "opencode" => "--auto",
        "cline" => "--auto-approve true",
        "aider" => "--yes-always",
        "amp" => "--dangerously-allow-all",
        "grok" => "--permission-mode bypassPermissions",
        "droid" => "--auto high",
        "kiro" => "--trust-all-tools",
        "autohand" => "--unrestricted",
        "continue" => "--allow \"*\"",
        "zcode" => "--mode yolo",
        "vibe" => "--agent auto-approve",
        "devin" => "--permission-mode bypass --respect-workspace-trust false",
        _ => "",
    }
}

fn yolo_env_for(agent_id: &str) -> &'static [(&'static str, &'static str)] {
    if agent_id == "goose" {
        &[("GOOSE_MODE", "auto")]
    } else {
        &[]
    }
}

/// Computes the effective `{ command, args, env }` for an agent pane (`agentPrefs.js:effectiveAgent`).
#[must_use]
pub fn effective_agent_launch(
    agent_id: &str,
    default_command: &str,
    prefs: Option<&AgentPrefEntry>,
    permissions: AgentPermissionMode,
    session_values: Option<&Value>,
    models: Option<&[Value]>,
) -> LaunchSpec {
    let given = prefs
        .map(|p| trim(&p.command))
        .filter(|s| !s.is_empty())
        .unwrap_or(default_command);
    let own = prefs.map_or("", |p| trim(&p.args));
    let base = if !own.is_empty() {
        own
    } else if permissions == AgentPermissionMode::Yolo {
        yolo_args_for(agent_id)
    } else {
        ""
    };
    let chosen = session_values
        .map(|vals| agent_model_launch::text(agent_id, vals, own, models))
        .unwrap_or_default();
    let args = match (chosen.is_empty(), base.is_empty()) {
        (true, true) => String::new(),
        (false, true) => chosen,
        (true, false) => base.to_owned(),
        (false, false) => format!("{chosen} {base}"),
    };
    let mut env = BTreeMap::new();
    if permissions == AgentPermissionMode::Yolo && own.is_empty() {
        for &(k, v) in yolo_env_for(agent_id) {
            env.insert(k.to_owned(), v.to_owned());
        }
    }
    if let Some(pref_env) = prefs.map(|p| p.env.as_str())
        && let Ok(parsed) = agent_env::parse(pref_env)
    {
        env.extend(parsed);
    }
    LaunchSpec {
        command: given.to_owned(),
        args,
        env,
    }
}

/// Checks whether `value` is a canonical 64-character lowercase hex SHA-256
/// launch digest (identical to `launch_fingerprint::valid`).
#[must_use]
pub fn valid_launch_signature(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Opaque 64-character lowercase hex SHA-256 launch identity compatible with
/// `launch_fingerprint::of`.
///
/// Enforces at the type level that raw `command`, `args`, or `env` text can
/// never be stored in [`PaneModelNode`], logged via `Debug`, or serialized.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LaunchSignature(String);

impl LaunchSignature {
    /// Hashes canonical `(command, args, sorted_env_pairs)` JSON bytes with SHA-256,
    /// matching `launch_fingerprint::of` byte-for-byte.
    #[must_use]
    pub fn of(command: &str, args: &str, env: &BTreeMap<String, String>) -> Self {
        let env_pairs: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let canonical =
            serde_json::to_vec(&(command, args, env_pairs)).expect("string tuple encodes");
        Self(format!("{:x}", Sha256::digest(canonical)))
    }

    /// Computes the opaque SHA-256 signature for `launch`.
    #[must_use]
    pub fn from_spec(launch: &LaunchSpec) -> Self {
        Self::of(&launch.command, &launch.args, &launch.env)
    }

    /// Wraps an existing 64-character lowercase hex SHA-256 digest (such as
    /// `SavedSession::launch_fingerprint`), rejecting raw JSON or malformed strings.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        valid_launch_signature(value).then(|| Self(value.to_owned()))
    }

    /// Borrows the 64-character lowercase hex SHA-256 digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes this signature into its 64-character lowercase hex SHA-256 `String`.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for LaunchSignature {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for LaunchSignature {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl From<LaunchSignature> for String {
    fn from(sig: LaunchSignature) -> Self {
        sig.0
    }
}

impl TryFrom<&str> for LaunchSignature {
    type Error = &'static str;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value).ok_or("launch signature must be a 64-char lowercase hex SHA-256 digest")
    }
}

impl TryFrom<String> for LaunchSignature {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if valid_launch_signature(&value) {
            Ok(Self(value))
        } else {
            Err("launch signature must be a 64-char lowercase hex SHA-256 digest")
        }
    }
}

impl PartialEq<str> for LaunchSignature {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for LaunchSignature {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for LaunchSignature {
    fn eq(&self, other: &String) -> bool {
        &self.0 == other
    }
}

impl std::fmt::Display for LaunchSignature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Debug for LaunchSignature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let prefix = self.0.get(..12).unwrap_or(&self.0);
        write!(f, "LaunchSignature({prefix}…)")
    }
}

impl Serialize for LaunchSignature {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for LaunchSignature {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// Computes the canonical tuple SHA-256 [`LaunchSignature`] for `launch`
/// (compatible with `launch_fingerprint::of`).
#[must_use]
pub fn launch_signature(launch: &LaunchSpec) -> LaunchSignature {
    LaunchSignature::from_spec(launch)
}

/// Checks whether a running agent pane's saved launch identity differs from its
/// current effective settings/model (`TerminalPane.vue:launchStale`).
#[must_use]
pub fn is_launch_stale(
    stored_launch_sig: Option<&LaunchSignature>,
    has_agent_command: bool,
    detected: bool,
    current_sig: &LaunchSignature,
) -> bool {
    let Some(stored) = stored_launch_sig else {
        return false;
    };
    has_agent_command && !detected && current_sig != stored
}

/// Last-known model metadata persisted across Tessel restarts (`paneLastModel.js`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedPaneModel {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

fn clean_saved_text(value: Option<&Value>) -> Option<String> {
    let s = trim(value?.as_str()?);
    let len = s.encode_utf16().count();
    if !(1..=MAX_SAVED_MODEL_UTF16).contains(&len) {
        return None;
    }
    if s.chars()
        .any(|c| ('\u{0000}'..='\u{001f}').contains(&c) || c == '\u{007f}')
    {
        return None;
    }
    Some(s.to_owned())
}

fn valid_saved_effort(value: Option<&Value>) -> Option<String> {
    let s = value?.as_str()?;
    let bytes = s.as_bytes();
    if (2..=12).contains(&bytes.len()) && bytes.iter().all(u8::is_ascii_lowercase) {
        Some(s.to_owned())
    } else {
        None
    }
}

/// Validates a persisted `lastModel` object from disk (`paneLastModel.js:savedPaneModel`).
#[must_use]
pub fn saved_pane_model(value: Option<&Value>) -> Option<SavedPaneModel> {
    let obj = value?.as_object()?;
    let model = clean_saved_text(obj.get("model"))?;
    let name = clean_saved_text(obj.get("name"));
    let effort = valid_saved_effort(obj.get("effort"));
    Some(SavedPaneModel {
        model,
        name,
        effort,
    })
}

/// Produces the `{ model, name?, effort? }` snapshot to write to `node.lastModel`,
/// returning `None` when `source == "saved"` so stale startup placeholders do
/// not overwrite themselves (`paneLastModel.js:paneModelToSave`).
#[must_use]
pub fn pane_model_to_save(value: Option<&Value>) -> Option<SavedPaneModel> {
    let obj = value?.as_object()?;
    if obj.get("source").and_then(Value::as_str) == Some("saved") {
        return None;
    }
    saved_pane_model(value)
}

/// Structural equality on [`SavedPaneModel`] (`paneLastModel.js:samePaneModel`).
#[must_use]
pub fn same_pane_model(a: Option<&SavedPaneModel>, b: Option<&SavedPaneModel>) -> bool {
    a == b
}

/// Maps an exact model or Cursor variant ID to its picker row values
/// (`agentSessionOptions.js:listedModelValues`).
#[must_use]
pub fn listed_model_values(models: &[Value], id: &str) -> Option<Map<String, Value>> {
    if id.is_empty() {
        return None;
    }
    decompose_cursor_model(models, id).or_else(|| {
        models
            .iter()
            .any(|m| m["id"].as_str() == Some(id))
            .then(|| {
                let mut out = Map::new();
                out.insert("model".into(), Value::String(id.to_owned()));
                out
            })
    })
}

fn effort_rank(value: &Value) -> i32 {
    ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
        .iter()
        .position(|e| value == *e)
        .map_or(-1, |i| i as i32)
}

/// Composes a Cursor grouped row + option values into the closest listed variant ID
/// (`cursorModels.js:composeCursorModel`).
#[must_use]
pub fn compose_cursor_row_variant(model: &Value, values: &Map<String, Value>) -> String {
    let Some(variants) = model["variants"].as_array().filter(|v| !v.is_empty()) else {
        return model["id"].as_str().unwrap_or_default().to_owned();
    };
    let effort = values
        .get("effort")
        .filter(|v| {
            v.is_string()
                && model["efforts"]
                    .as_array()
                    .is_some_and(|efforts| efforts.contains(v))
        })
        .unwrap_or(&model["defaultEffort"]);
    let fast = if model["hasFast"] == true {
        values.get("fastMode") == Some(&Value::Bool(true))
    } else {
        variants[0]["fast"] == true
    };
    let thinking = if model["hasThinking"] == true {
        values
            .get("thinking")
            .and_then(Value::as_bool)
            .unwrap_or(model["defaultThinking"] == true)
    } else {
        variants[0]["thinking"] == true
    };
    let mut best: Option<(&Value, i32)> = None;
    for variant in variants {
        let diff = effort_rank(&variant["effort"]) - effort_rank(effort);
        let cost = diff.abs() * 4
            + i32::from(diff > 0)
            + i32::from((variant["thinking"] == true) != thinking) * 6
            + i32::from((variant["fast"] == true) != fast) * 2;
        if best.is_none_or(|(_, prior)| cost < prior) {
            best = Some((variant, cost));
        }
    }
    best.and_then(|(v, _)| v["id"].as_str())
        .unwrap_or_default()
        .to_owned()
}

/// Resolves the concrete model ID sent for `values` (`agentSessionOptions.js:composedModelId`).
#[must_use]
pub fn composed_model_id(
    agent_id: &str,
    values: Option<&Value>,
    models: Option<&[Value]>,
) -> Option<String> {
    let obj = values?.as_object()?;
    let model_id = obj.get("model")?.as_str()?;
    if model_id.is_empty() {
        return None;
    }
    let Some(cat) = catalog::find(agent_id) else {
        return Some(model_id.to_owned());
    };
    if !matches!(agent_id, "cursor" | "antigravity") {
        return Some(model_id.to_owned());
    }
    let model = models
        .and_then(|rows| rows.iter().find(|m| m["id"].as_str() == Some(model_id)))
        .or_else(|| {
            cat.models
                .iter()
                .find(|m| m["id"].as_str() == Some(model_id))
        });
    let Some(model) = model else {
        return Some(model_id.to_owned());
    };
    let mut picked = Map::new();
    if let Some(opts) = model["options"].as_array() {
        for opt in opts {
            if let Some(id) = opt["id"].as_str()
                && let Some(val) = obj.get(id)
                && !val.is_null()
            {
                picked.insert(id.to_owned(), val.clone());
            }
        }
    }
    if agent_id == "antigravity" {
        if picked
            .get("effort")
            .and_then(Value::as_str)
            .is_some_and(|e| !e.is_empty())
        {
            for level in ["low", "medium", "high", "xhigh", "max"] {
                if let Some(base) = model_id.strip_suffix(&format!("-{level}")) {
                    return Some(base.to_owned());
                }
            }
        }
        return Some(model_id.to_owned());
    }
    if model["variants"].is_array() {
        return Some(compose_cursor_row_variant(model, &picked));
    }
    if model_id == "auto" {
        return Some(model_id.to_owned());
    }
    let mut out = model_id.to_owned();
    if model_id.starts_with("claude-") && picked.get("thinking") == Some(&Value::Bool(true)) {
        out.push_str("-thinking");
    }
    if let Some(effort) = picked.get("effort").and_then(Value::as_str) {
        out.push('-');
        out.push_str(effort);
    }
    if !model_id.starts_with("claude-") && picked.get("fastMode") == Some(&Value::Bool(true)) {
        out.push_str("-fast");
    }
    Some(out)
}

/// Validates a CLI/IPC `--model` & `--effort` request against the agent's catalog
/// and discovered models (`cliRequests.js:sessionChoiceError`).
#[must_use]
pub fn session_choice_error(
    agent_id: &str,
    model: &str,
    effort: &str,
    models: Option<&[Value]>,
    french: bool,
) -> Option<String> {
    if model.is_empty() {
        return None;
    }
    let Some(cat) = catalog::find(agent_id) else {
        return Some(if french {
            "Tessel ne peut pas choisir le modèle de cet agent.".into()
        } else {
            "Tessel cannot choose the model of this agent.".into()
        });
    };
    let list: &[Value] = match models {
        Some(rows) if !rows.is_empty() => rows,
        _ => &cat.models,
    };
    let ids: Vec<&str> = list.iter().filter_map(|m| m["id"].as_str()).collect();
    if !ids.contains(&model) && listed_model_values(list, model).is_some() {
        return None;
    }
    if !safe_session_value(model) || !ids.contains(&model) {
        let available = if ids.is_empty() {
            "—".to_owned()
        } else {
            ids.join(", ")
        };
        return Some(if french {
            format!("Modèle inconnu « {model} ». Disponibles : {available}")
        } else {
            format!("Unknown model “{model}”. Available: {available}")
        });
    }
    if !effort.is_empty() {
        let choices: Vec<&str> = catalog::model_options(cat, Some(list), model)
            .iter()
            .find(|o| o["id"] == "effort" && o["kind"]["type"] == "select")
            .and_then(|o| o["kind"]["choices"].as_array())
            .map(|arr| arr.iter().filter_map(|c| c["value"].as_str()).collect())
            .unwrap_or_default();
        if choices.is_empty() {
            return Some(if french {
                format!("{model} n'a pas de choix d'effort.")
            } else {
                format!("{model} has no effort choice.")
            });
        }
        if !choices.contains(&effort) {
            let available = choices.join(", ");
            return Some(if french {
                format!("Effort inconnu « {effort} » pour {model}. Disponibles : {available}")
            } else {
                format!("Unknown effort “{effort}” for {model}. Available: {available}")
            });
        }
    }
    None
}

// -----------------------------------------------------------------------------
// 2. Display Labels, Header Chip, Source Provenance & Effective Model Resolution
// -----------------------------------------------------------------------------

/// Normalizes a raw effort token to one of Tessel's 8 canonical tier IDs
/// (`modelLabel.js:effortLevel`).
#[must_use]
pub fn effort_level(raw: &str) -> Option<&'static str> {
    let trimmed = trim(raw);
    if trimmed.is_empty() {
        return None;
    }
    let mut key = String::with_capacity(trimmed.len());
    let mut prev_sep = false;
    for ch in trimmed.chars() {
        if js_space(ch) || ch == '_' || ch == '-' {
            if !prev_sep {
                key.push('-');
                prev_sep = true;
            }
        } else {
            key.push(ch.to_ascii_lowercase());
            prev_sep = false;
        }
    }
    match key.as_str() {
        "none" | "off" | "disabled" => Some("none"),
        "minimal" => Some("minimal"),
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" | "x-high" | "extra-high" => Some("xhigh"),
        "max" => Some("max"),
        "ultra" => Some("ultra"),
        _ => None,
    }
}

fn ascii_title_word(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::with_capacity(word.len());
    out.push(first.to_ascii_uppercase());
    for ch in chars {
        out.push(ch.to_ascii_lowercase());
    }
    out
}

fn strip_prefix_ascii_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

fn strip_suffix_ascii_ci<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    let split = s.len().checked_sub(suffix.len())?;
    let tail = s.get(split..)?;
    tail.eq_ignore_ascii_case(suffix).then(|| &s[..split])
}

fn format_claude_label(s: &str, cap: &str) -> Option<String> {
    let rest = strip_prefix_ascii_ci(s, "claude-").unwrap_or(s);
    let (family, tail) = ["opus", "sonnet", "haiku", "fable"]
        .into_iter()
        .find_map(|fam| {
            let after = strip_prefix_ascii_ci(rest, fam)?;
            (after.is_empty() || after.starts_with('-')).then_some((fam, after))
        })?;
    let name = ascii_title_word(family);
    if tail.is_empty() {
        return Some(format!("{name}{cap}"));
    }
    let parts: Vec<&str> = tail[1..].split('-').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let ver = match parts.as_slice() {
        [a] => format!(" {a}"),
        [a, b] if (1..=2).contains(&b.len()) => format!(" {a}.{b}"),
        [a, b] if b.len() == 8 => format!(" {a}"),
        [a, b, c] if (1..=2).contains(&b.len()) && c.len() == 8 => format!(" {a}.{b}"),
        _ => return None,
    };
    Some(format!("{name}{ver}{cap}"))
}

fn format_gpt_label(s: &str, cap: &str) -> Option<String> {
    let after = strip_prefix_ascii_ci(s, "gpt-").filter(|rest| !rest.is_empty())?;
    let bytes = after.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return None;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        let dot = i;
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == dot + 1 {
            return None;
        }
    }
    let ver = &after[..i];
    let tail = &after[i..];
    if tail.is_empty() {
        return Some(format!("GPT-{ver}{cap}"));
    }
    if !tail.starts_with('-') {
        return None;
    }
    let segs: Vec<&str> = tail[1..].split('-').collect();
    if segs
        .iter()
        .any(|w| w.is_empty() || !w.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return None;
    }
    let words: Vec<String> = segs
        .into_iter()
        .map(|w| {
            if w.eq_ignore_ascii_case("mini") || w.eq_ignore_ascii_case("nano") {
                w.to_ascii_lowercase()
            } else {
                ascii_title_word(w)
            }
        })
        .collect();
    Some(format!("GPT-{ver} {}{cap}", words.join(" ")))
}

fn format_muse_label(s: &str) -> Option<String> {
    if strip_prefix_ascii_ci(s, "muse-").is_none_or(str::is_empty) {
        return None;
    }
    let segs: Vec<&str> = s.split('-').collect();
    if segs.len() < 2
        || segs[1].is_empty()
        || !segs[1].bytes().all(|b| b.is_ascii_alphabetic())
        || segs[2..]
            .iter()
            .any(|w| w.is_empty() || !w.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.'))
    {
        return None;
    }
    let words: Vec<String> = segs
        .into_iter()
        .map(|w| {
            if w.as_bytes()[0].is_ascii_digit() {
                w.to_owned()
            } else {
                ascii_title_word(w)
            }
        })
        .collect();
    Some(words.join(" "))
}

/// Short human-readable label for a raw model ID (`modelLabel.js:modelLabel`).
#[must_use]
pub fn model_label(model: &str) -> String {
    let s = trim(model);
    if s.is_empty() {
        return String::new();
    }
    let s = s.rsplit_once('/').map_or(s, |(_, tail)| tail);
    let (s, big) = match strip_suffix_ascii_ci(s, "[1m]") {
        Some(core) => (core, true),
        None => (s, false),
    };
    let cap = if big { " (1M)" } else { "" };
    if let Some(claude) = format_claude_label(s, cap) {
        return claude;
    }
    if let Some(gpt) = format_gpt_label(s, cap) {
        return gpt;
    }
    if let Some(muse) = format_muse_label(s) {
        return muse;
    }
    format!("{s}{cap}")
}

/// Effort label used in the pane header chip, tooltip, and native chat bar
/// (`native-chat-session-option-labels.js:nativeChatSessionChoiceLabel`).
#[must_use]
pub fn header_effort_label(effort: &str, french: bool) -> &str {
    match effort {
        "none" => {
            if french {
                "Aucun"
            } else {
                "None"
            }
        }
        "minimal" => "Minimal",
        "low" => {
            if french {
                "Faible"
            } else {
                "Low"
            }
        }
        "medium" => {
            if french {
                "Moyen"
            } else {
                "Medium"
            }
        }
        "high" => {
            if french {
                "Élevé"
            } else {
                "High"
            }
        }
        "xhigh" => {
            if french {
                "Très élevé"
            } else {
                "Extra high"
            }
        }
        "max" => "Max",
        "ultra" => "Ultra",
        _ => effort,
    }
}

/// Provenance of a pane's currently displayed model (`TerminalPane.vue:modelTitle`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelSource {
    Session,
    Command,
    Picked,
    Chosen,
    Screen,
    Saved,
    Running,
    #[default]
    Settings,
}

impl ModelSource {
    #[must_use]
    pub fn description(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Session, false) => "from its latest answer",
            (Self::Session, true) => "d'après sa dernière réponse",
            (Self::Command, false) => "from its command",
            (Self::Command, true) => "d'après sa commande",
            (Self::Picked, false) => "the model last picked in it",
            (Self::Picked, true) => "le dernier modèle choisi dans l'agent",
            (Self::Chosen, false) => "chosen in Tessel (pane menu > Model, or Settings > Agents)",
            (Self::Chosen, true) => {
                "choisi dans Tessel (menu du panneau > Modèle, ou Paramètres > Agents)"
            }
            (Self::Screen, false) => "read from its screen (status bar or banner)",
            (Self::Screen, true) => "lu à l'écran (barre d'état ou bannière)",
            (Self::Saved, false) => {
                "what it showed before Tessel restarted (read again in a moment)"
            }
            (Self::Saved, true) => {
                "ce qu'il affichait avant le redémarrage de Tessel (relu dans un instant)"
            }
            (Self::Running, false) => "the only model Ollama has running",
            (Self::Running, true) => "le seul modèle qu'Ollama fait tourner",
            (Self::Settings, false) => "from its settings (a change inside the agent may not show)",
            (Self::Settings, true) => {
                "d'après ses paramètres (un changement fait dans l'agent peut ne pas apparaître)"
            }
        }
    }
}

/// Model currently shown by a pane (`agentModel` in `TerminalPane.vue`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShownAgentModel {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen_effort: Option<String>,
    #[serde(default)]
    pub source: ModelSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<u64>,
}

/// Optimistic live-pick override (`node.modelChoice` in `TerminalPane.vue`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptimisticModelChoice {
    pub model: String,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<String>,
}

/// Result of reconciling `node.sessionOptions` + `node.modelChoice` against an
/// observed agent model report (`TerminalPane.vue:chosenShown`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChosenShownOutcome {
    pub shown: Option<ShownAgentModel>,
    pub model_choice: Option<OptimisticModelChoice>,
}

/// Reconciles optimistic `modelChoice` and `sessionOptions` against a session/screen/settings
/// model report, including SSH remote `seen` clock-skew protection (`TerminalPane.vue:chosenShown`).
#[must_use]
pub fn resolve_chosen_shown(
    session_options: Option<&Value>,
    model_choice: Option<&OptimisticModelChoice>,
    remote_host: bool,
    observed: Option<&ShownAgentModel>,
    models: &[Value],
) -> ChosenShownOutcome {
    let has_optimistic = model_choice.is_some_and(|c| !c.model.is_empty());
    if !has_optimistic
        && let Some(opt_model) = session_options
            .and_then(|v| v.get("model"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    {
        if let Some(obs) = observed
            && obs.source == ModelSource::Session
            && !obs.model.is_empty()
        {
            return ChosenShownOutcome {
                shown: Some(obs.clone()),
                model_choice: model_choice.cloned(),
            };
        }
        let effort = session_options
            .and_then(|v| v.get("effort"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| observed.and_then(|o| o.chosen_effort.clone()));
        return ChosenShownOutcome {
            shown: Some(ShownAgentModel {
                model: model_choice_label(models, opt_model).to_owned(),
                name: None,
                effort,
                chosen_effort: None,
                source: ModelSource::Chosen,
                at: None,
            }),
            model_choice: model_choice.cloned(),
        };
    }

    let Some(mut choice) = model_choice.cloned().filter(|c| !c.model.is_empty()) else {
        return ChosenShownOutcome {
            shown: observed.cloned(),
            model_choice: None,
        };
    };

    if let Some(obs) = observed
        && obs.source == ModelSource::Session
    {
        if let (Some(obs_at), Some(choice_at)) = (obs.at, choice.at)
            && !remote_host
            && obs_at > choice_at
        {
            return ChosenShownOutcome {
                shown: Some(obs.clone()),
                model_choice: None,
            };
        }
        let said = format!("{}|{}", obs.model, obs.effort.as_deref().unwrap_or(""));
        match choice.seen.as_deref() {
            None => choice.seen = Some(said),
            Some(prev) if prev != said => {
                return ChosenShownOutcome {
                    shown: Some(obs.clone()),
                    model_choice: None,
                };
            }
            Some(_) => {}
        }
    }

    let effort = choice
        .effort
        .clone()
        .or_else(|| observed.and_then(|o| o.chosen_effort.clone().or_else(|| o.effort.clone())));
    let shown = ShownAgentModel {
        model: model_choice_label(models, &choice.model).to_owned(),
        name: None,
        effort,
        chosen_effort: None,
        source: ModelSource::Chosen,
        at: None,
    };
    ChosenShownOutcome {
        shown: Some(shown),
        model_choice: Some(choice),
    }
}

/// Newest known Claude model ID for a bare family alias (`claudePricing.js` + `TerminalPane.vue:newestClaudeOf`).
#[must_use]
pub fn newest_claude_of(family: &str) -> Option<&'static str> {
    match family.to_ascii_lowercase().as_str() {
        "fable" => Some("claude-fable-5-1"),
        "opus" => Some("claude-opus-5-5"),
        "sonnet" => Some("claude-sonnet-5-5"),
        "haiku" => Some("claude-haiku-4-5"),
        _ => None,
    }
}

fn matches_claude_family_version(candidate: &str, family: &str) -> bool {
    let trimmed = trim(candidate);
    let rest = strip_prefix_ascii_ci(trimmed, "claude-").unwrap_or(trimmed);
    let Some(after) = strip_prefix_ascii_ci(rest, family) else {
        return false;
    };
    let bytes = after.as_bytes();
    bytes.len() >= 2 && bytes[0] == b'-' && bytes[1].is_ascii_digit()
}

/// Fills in a concrete version when Claude reports a bare family alias (`"opus"`, `"opus[1m]"`)
/// (`TerminalPane.vue:withModelVersion`).
#[must_use]
pub fn with_model_version(shown: &ShownAgentModel, reported: &[&str]) -> ShownAgentModel {
    let raw = trim(&shown.model);
    let (core, has_1m) = match strip_suffix_ascii_ci(raw, "[1m]") {
        Some(base) => (base, true),
        None => (raw, false),
    };
    let family = match core.to_ascii_lowercase().as_str() {
        "opus" => "opus",
        "sonnet" => "sonnet",
        "haiku" => "haiku",
        "fable" => "fable",
        _ => return shown.clone(),
    };
    let full = reported
        .iter()
        .copied()
        .find(|m| matches_claude_family_version(m, family))
        .or_else(|| newest_claude_of(family));
    let Some(full) = full else {
        return shown.clone();
    };
    let full_trimmed = trim(full);
    let already_1m = strip_suffix_ascii_ci(full_trimmed, "[1m]").is_some();
    let mut updated = shown.clone();
    updated.model = if has_1m && !already_1m {
        format!("{full_trimmed}[1m]")
    } else {
        full_trimmed.to_owned()
    };
    updated
}

/// Formats `modelText` (`"Opus 5.5 · Medium"`, `TerminalPane.vue:1271-1274`).
#[must_use]
pub fn model_text(agent_model: Option<&ShownAgentModel>, french: bool) -> String {
    let Some(m) = agent_model.filter(|m| !m.model.is_empty()) else {
        return String::new();
    };
    let base = m
        .name
        .as_deref()
        .filter(|n| !n.is_empty())
        .map_or_else(|| model_label(&m.model), str::to_owned);
    match m.effort.as_deref().filter(|e| !e.is_empty()) {
        Some(eff) => format!("{base} · {}", header_effort_label(eff, french)),
        None => base,
    }
}

/// Formats `headerModelText` (`TerminalPane.vue:1672`), honoring Live Share guest overrides.
#[must_use]
pub fn header_model_text(
    is_agent: bool,
    guest_runs: bool,
    guest_model: Option<&str>,
    agent_model: Option<&ShownAgentModel>,
    french: bool,
) -> String {
    if guest_runs {
        return guest_model.unwrap_or_default().to_owned();
    }
    if !is_agent {
        return String::new();
    }
    model_text(agent_model, french)
}

/// Accessible label for the header model chip (`pane.sessionOptions.chipLabel` / `chipStatic`).
#[must_use]
pub fn chip_aria_label(
    header_text: &str,
    has_model_choice: bool,
    guest_runs: bool,
    french: bool,
) -> String {
    if header_text.is_empty() {
        return String::new();
    }
    if has_model_choice && !guest_runs {
        if french {
            format!("Modèle : {header_text}. Choisir le modèle")
        } else {
            format!("Model: {header_text}. Choose the model")
        }
    } else if french {
        format!("Modèle : {header_text}")
    } else {
        format!("Model: {header_text}")
    }
}

/// Two-line tooltip for the header model chip (`TerminalPane.vue:modelTitle`).
#[must_use]
pub fn model_title(agent_model: Option<&ShownAgentModel>, french: bool) -> String {
    let Some(m) = agent_model else {
        return String::new();
    };
    let head = match m.effort.as_deref().filter(|e| !e.is_empty()) {
        Some(eff) => {
            let eff_name = header_effort_label(eff, french);
            if french {
                format!("Modèle : {} (raisonnement {eff_name})", m.model)
            } else {
                format!("Model: {} (reasoning {eff_name})", m.model)
            }
        }
        None => {
            if french {
                format!("Modèle : {}", m.model)
            } else {
                format!("Model: {}", m.model)
            }
        }
    };
    format!("{head}\n{}", m.source.description(french))
}

/// Top reset-item label in `pane menu > Model` (`TerminalPane.vue:modelDefaultLabel`).
#[must_use]
pub fn model_default_label(
    agent_id: &str,
    models: &[Value],
    persisted: &BTreeMap<String, Defaults>,
    french: bool,
) -> String {
    if let Some(def_vals) = persisted.get(agent_id).and_then(Defaults::values) {
        let pill = session_pill_label(models, Some(&def_vals), french);
        if french {
            format!("Par défaut des paramètres ({pill})")
        } else {
            format!("Default from Settings ({pill})")
        }
    } else {
        agent_own_default_label(french).to_owned()
    }
}

/// Context note at the top of `pane menu > Model` (`TerminalPane.vue:modelMenuNote`).
#[must_use]
pub fn model_menu_note(agent_id: &str, pane_running: bool, french: bool) -> &'static str {
    if !pane_running {
        return if french {
            "S'applique au démarrage de l'agent."
        } else {
            "Applies when the agent starts."
        };
    }
    let mid_kind =
        catalog::find(agent_id).and_then(|c| c.model_apply["midSession"]["kind"].as_str());
    match mid_kind {
        Some("picker-filter") => {
            if french {
                "Un modèle choisi ici s'applique aussitôt ; son effort ou Fast ouvre son propre sélecteur dans le terminal (Tab sur le modèle)."
            } else {
                "A model chosen here switches at once; its effort or Fast opens its own picker in the terminal (Tab on the model)."
            }
        }
        Some("command") => "",
        _ => {
            if french {
                "Un modèle choisi ici s'applique au redémarrage de l'agent."
            } else {
                "A model picked here applies when the agent restarts."
            }
        }
    }
}

/// Disabled-pick reason shown while a running agent is busy (`TerminalPane.vue:modelBusyReason`).
#[must_use]
pub fn model_busy_reason(pane_running: bool, pane_busy: bool, french: bool) -> &'static str {
    if pane_running && pane_busy {
        if french {
            "Il travaille : son modèle pourra changer une fois qu'il sera inactif."
        } else {
            "It is working: its model can change once it is idle."
        }
    } else {
        ""
    }
}

/// Tooltip for the `"Apply"` / `"Restart to apply"` badge (`pane.apply.titleModel`).
#[must_use]
pub fn apply_restart_title(french: bool) -> &'static str {
    if french {
        "Paramètres > Agents ou son modèle a changé depuis le démarrage de cet agent (Yolo, arguments, variables ou modèle). Redémarrez-le pour les appliquer : même panneau, sa conversation reprise"
    } else {
        "Settings > Agents or its model changed since this agent started (Yolo, arguments, variables or model). Restart it to apply them: same pane, its conversation resumed"
    }
}

/// Label for the `"Restart to apply"` menu badge (`pane.badge.apply`).
#[must_use]
pub fn restart_apply_badge_text(french: bool) -> &'static str {
    if french {
        "Redémarrer pour appliquer"
    } else {
        "Restart to apply"
    }
}

/// Maps a displayed model ID/label (`"claude-opus-5-5"`, `"gpt-5.3-codex-high-fast"`)
/// onto a catalog row ID (`TerminalPane.vue:effectiveModelIdOf`).
#[must_use]
pub fn effective_model_id_of<'a>(list: &'a [Value], shown: &str) -> Option<&'a str> {
    if shown.is_empty() {
        return None;
    }
    if let Some(exact) = list
        .iter()
        .find(|m| m["id"].as_str() == Some(shown))
        .or_else(|| list.iter().find(|m| m["label"].as_str() == Some(shown)))
        .and_then(|m| m["id"].as_str())
    {
        return Some(exact);
    }
    if let Some(listed) = listed_model_values(list, shown)
        && let Some(row_id) = listed.get("model").and_then(Value::as_str)
        && let Some(found) = list
            .iter()
            .find(|m| m["id"].as_str() == Some(row_id))
            .and_then(|m| m["id"].as_str())
    {
        return Some(found);
    }
    list.iter().find_map(|m| {
        let id = m["id"].as_str()?;
        let mid = format!("-{id}-");
        let suffix = format!("-{id}");
        (shown.contains(&mid) || shown.ends_with(&suffix)).then_some(id)
    })
}

/// Resolves the active picker model row ID (`TerminalPane.vue:effectiveModelId`).
#[must_use]
pub fn effective_model_id<'a>(list: &'a [Value], shown_model: Option<&str>) -> Option<&'a str> {
    if list.is_empty() {
        return None;
    }
    if let Some(shown) = shown_model
        && let Some(found) = effective_model_id_of(list, shown)
    {
        return Some(found);
    }
    list.iter()
        .find(|m| m["isDefault"] == true)
        .or_else(|| list.first())
        .and_then(|m| m["id"].as_str())
}

/// Values highlighted inside `pane menu > Model` (`TerminalPane.vue:paneMenuValues`).
#[must_use]
pub fn pane_menu_values(
    agent_id: &str,
    pane_running: bool,
    pane_model_list: &[Value],
    agent_model: Option<&ShownAgentModel>,
    session_options: Option<&Value>,
) -> Option<Value> {
    if agent_id == "cursor"
        && pane_running
        && let Some(m) = agent_model
        && m.source == ModelSource::Screen
        && let Some(listed) = listed_model_values(pane_model_list, &m.model)
    {
        return Some(Value::Object(listed));
    }
    session_options.cloned()
}

/// Computes the next per-pane `sessionOptions` map for a model or option pick
/// (`TerminalPane.vue:nextPaneValues`).
#[must_use]
pub fn next_pane_values(
    agent_id: &str,
    pane_model_list: &[Value],
    base_values: Option<&Value>,
    effective_id: Option<&str>,
    option_id: &str,
    value: Option<&Value>,
) -> Option<Value> {
    let current = values_on_listed_row(pane_model_list, base_values);
    if option_id == "model" {
        let model_val = value.and_then(Value::as_str).filter(|s| !s.is_empty())?;
        let mut next = Map::new();
        next.insert("model".into(), Value::String(model_val.to_owned()));
        if let Some(cat) = catalog::find(agent_id)
            && let Some(effort) = current
                .as_ref()
                .and_then(|c| c.get("effort"))
                .and_then(Value::as_str)
                .filter(|e| !e.is_empty())
        {
            let offers = catalog::model_options(cat, Some(pane_model_list), model_val)
                .iter()
                .any(|o| {
                    o["id"] == "effort"
                        && o["kind"]["type"] == "select"
                        && o["kind"]["choices"].as_array().is_some_and(|arr| {
                            arr.iter().any(|c| c["value"].as_str() == Some(effort))
                        })
                });
            if offers {
                next.insert("effort".into(), Value::String(effort.to_owned()));
            }
        }
        return Some(Value::Object(next));
    }

    let has_current_model = current
        .as_ref()
        .and_then(|c| c.get("model"))
        .and_then(Value::as_str)
        .is_some_and(|m| !m.is_empty());
    if !has_current_model {
        let Some(model) = effective_id.filter(|m| !m.is_empty()) else {
            return current.map(Value::Object);
        };
        let Some(val) = value.filter(|v| !v.is_null()) else {
            return current.map(Value::Object);
        };
        let mut next = Map::new();
        next.insert("model".into(), Value::String(model.to_owned()));
        next.insert(option_id.to_owned(), val.clone());
        return Some(Value::Object(next));
    }

    let mut next = current?;
    match value {
        None | Some(Value::Null) => {
            next.remove(option_id);
        }
        Some(val) => {
            next.insert(option_id.to_owned(), val.clone());
        }
    }
    Some(Value::Object(next))
}

// -----------------------------------------------------------------------------
// 3. Pure Live-Switch, Action & ChatView Planners (Zero Side Effects)
// -----------------------------------------------------------------------------

/// Delivery mode for a planned PTY slash command (`claudeModelSwitch.js:typeCommand`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandDelivery {
    /// Writes the command body in one chunk, waits 120 ms, then sends `\r`.
    Write,
    /// Writes one character every 15 ms (so Codex/OpenCode do not classify it as paste),
    /// waits 120 ms, then sends `\r`.
    Type,
}

/// Declarative slash command planned by [`plan_model_pick`] or [`plan_model_action`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypedCommand {
    pub command: String,
    pub delivery: CommandDelivery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_model_label: Option<String>,
}

/// Severity of a planned toast notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToastKind {
    Info,
    Error,
}

/// Declarative toast planned for UI presentation (`TerminalPane.vue:pickModelOption`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedToast {
    pub text: String,
    pub kind: ToastKind,
    pub timeout: Option<u64>,
}

/// Outcome of [`ClaudeModelSwitchObserver`] or a simulated Claude switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaudeSwitchOutcome {
    Applied,
    Rejected,
    Unknown,
}

/// Outcome of [`plan_model_pick`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelPickOutcome {
    Saved,
    Sent,
    Picker,
    Applied,
    Rejected,
    Unknown,
    Busy,
    /// Interactive Claude `/model <id>` command planned; call [`apply_claude_switch_outcome`]
    /// once [`ClaudeModelSwitchObserver`] settles.
    AwaitClaudeConfirmation,
    ActionSent,
}

/// Mutable per-pane model state carried into and out of pure planners.
///
/// `launch_sig` uses the opaque [`LaunchSignature`] SHA-256 type so raw
/// `command`/`args`/`env` tuples can never be stored, serialized, or logged,
/// and `Debug` censors `agent_command`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneModelNode {
    pub agent_id: String,
    // Runtime launch authority is reconstructed locally, never exported with
    // per-pane model metadata (a custom command may contain credentials).
    #[serde(default, skip_serializing)]
    pub agent_command: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub detected: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub chat_view: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_host_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_options: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_sig: Option<LaunchSignature>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_choice: Option<OptimisticModelChoice>,
}

impl std::fmt::Debug for PaneModelNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaneModelNode")
            .field("agent_id", &self.agent_id)
            .field(
                "agent_command",
                &self.agent_command.as_ref().map(|_| "<redacted>"),
            )
            .field("detected", &self.detected)
            .field("chat_view", &self.chat_view)
            .field("remote_host_id", &self.remote_host_id)
            .field("session_options", &self.session_options)
            .field("launch_sig", &self.launch_sig)
            .field("model_choice", &self.model_choice)
            .finish()
    }
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// Context snapshot for pure model-pick / action / ChatView planning.
pub struct PanePickContext<'a> {
    pub is_agent: bool,
    pub pane_running: bool,
    pub pane_busy: bool,
    pub pending: bool,
    pub user_typing: bool,
    pub chat_view_available: bool,
    pub discovered_models: Option<&'a [Value]>,
    pub agent_model: Option<&'a ShownAgentModel>,
    pub agent_prefs: &'a BTreeMap<String, AgentPrefEntry>,
    pub persisted_defaults: &'a BTreeMap<String, Defaults>,
    pub permissions: AgentPermissionMode,
    pub now_ms: u64,
    pub french: bool,
}

/// Declarative result of [`plan_model_pick`] or [`plan_model_action`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelPickPlan {
    pub outcome: Option<ModelPickOutcome>,
    pub typed_commands: Vec<TypedCommand>,
    pub toast: Option<PlannedToast>,
    pub hold_claude_default_model: bool,
    pub pending_next_choice: Option<Value>,
    pub launch_stale_after: bool,
    pub pane_menu_values_before: Option<Value>,
    pub effective_id_before: Option<String>,
    pub node: PaneModelNode,
}

/// Result returned to native `ChatView` `setOption(optionId, value)` (`TerminalPane.vue:898-930`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatSetOptionResult {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub picker: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Declarative plan for native `ChatView` `setOption`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatSetOptionPlan {
    pub chat_result: ChatSetOptionResult,
    pub typed_commands: Vec<TypedCommand>,
    pub hold_claude_default_model: bool,
    pub pending_next_choice: Option<Value>,
    pub node: PaneModelNode,
}

fn compute_node_signature(
    node: &PaneModelNode,
    choice: Option<&Value>,
    pane_model_list: &[Value],
    ctx: &PanePickContext<'_>,
) -> LaunchSignature {
    let values = launch_session_values(choice, ctx.persisted_defaults, &node.agent_id);
    let default_cmd = node
        .agent_command
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&node.agent_id);
    let spec = effective_agent_launch(
        &node.agent_id,
        default_cmd,
        ctx.agent_prefs.get(&node.agent_id),
        ctx.permissions,
        values.as_ref(),
        Some(pane_model_list),
    );
    launch_signature(&spec)
}

fn adopt_live_on_node(
    node: &mut PaneModelNode,
    next: Option<Value>,
    pane_model_list: &[Value],
    ctx: &PanePickContext<'_>,
) {
    let was_current = node.launch_sig.as_ref().is_some_and(|sig| {
        compute_node_signature(node, node.session_options.as_ref(), pane_model_list, ctx).as_str()
            == sig.as_str()
    });
    node.session_options = next.clone();
    if was_current {
        node.launch_sig = Some(compute_node_signature(
            node,
            node.session_options.as_ref(),
            pane_model_list,
            ctx,
        ));
    }
    if let Some(model_id) = next
        .as_ref()
        .and_then(|v| v.get("model"))
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
    {
        let effort = next
            .as_ref()
            .and_then(|v| v.get("effort"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        node.model_choice = Some(OptimisticModelChoice {
            model: model_id.to_owned(),
            effort,
            at: Some(ctx.now_ms),
            seen: None,
        });
    }
}

fn option_apply_for<'a>(
    cat: &'a Catalog,
    pane_model_list: &'a [Value],
    agent_id: &str,
    option_id: &str,
    model_id: Option<&str>,
) -> Option<(&'a Value, bool)> {
    if option_id == "model" {
        return Some((&cat.model_apply, true));
    }
    let opt = catalog::model_options(cat, Some(pane_model_list), model_id.unwrap_or(""))
        .iter()
        .find(|o| o["id"].as_str() == Some(option_id))?;
    if opt["apply"]["composedIntoModel"] == true && matches!(agent_id, "cursor" | "antigravity") {
        return Some((&cat.model_apply, true));
    }
    Some((&opt["apply"], false))
}

#[must_use]
pub fn toast_for_outcome(outcome: ModelPickOutcome, french: bool) -> Option<PlannedToast> {
    match outcome {
        ModelPickOutcome::Picker => Some(PlannedToast {
            text: if french {
                "Son sélecteur de modèle est ouvert dans le terminal : Tab sur le modèle change son effort ou Fast.".into()
            } else {
                "Its model picker is open in the terminal: Tab on the model changes its effort or Fast.".into()
            },
            kind: ToastKind::Info,
            timeout: Some(6_000),
        }),
        ModelPickOutcome::Rejected => Some(PlannedToast {
            text: if french {
                "Claude a conservé le modèle actuel.".into()
            } else {
                "Claude kept the current model.".into()
            },
            kind: ToastKind::Error,
            timeout: None,
        }),
        ModelPickOutcome::Unknown => Some(PlannedToast {
            text: if french {
                "Impossible de vérifier le changement de modèle ; ouvrez le terminal pour vérifier."
                    .into()
            } else {
                "Could not verify the model change; open the terminal to check.".into()
            },
            kind: ToastKind::Error,
            timeout: Some(8_000),
        }),
        ModelPickOutcome::Sent => Some(PlannedToast {
            text: if french {
                "Envoyé à l'agent — non confirmé".into()
            } else {
                "Sent to the agent — not confirmed".into()
            },
            kind: ToastKind::Info,
            timeout: Some(3_000),
        }),
        _ => None,
    }
}

/// Plans a `pane menu > Model` action item (`toggle-command` `/fast` or `agent-picker`
/// `/model`, `/models`, `/thinking`) without executing any side effect.
#[must_use]
pub fn plan_model_action(
    node: &PaneModelNode,
    ctx: &PanePickContext<'_>,
    option_id: &str,
) -> ModelPickPlan {
    let next_node = node.clone();
    let Some(cat) = catalog::find(&node.agent_id) else {
        return empty_action_plan(next_node);
    };
    if !ctx.pane_running || ctx.pane_busy {
        return empty_action_plan(next_node);
    }
    let has_model_choice = ctx.is_agent && !node.detected;
    let pane_model_list = if has_model_choice {
        catalog::models_for(&node.agent_id, ctx.discovered_models)
    } else {
        Vec::new()
    };
    let pane_vals = launch_session_values(
        node.session_options.as_ref(),
        ctx.persisted_defaults,
        &node.agent_id,
    );
    let model_id = pane_vals
        .as_ref()
        .and_then(|v| v.get("model"))
        .and_then(Value::as_str);
    let Some((apply, _)) =
        option_apply_for(cat, &pane_model_list, &node.agent_id, option_id, model_id)
    else {
        return empty_action_plan(next_node);
    };
    let mid = &apply["midSession"];
    let Some(kind) = mid["kind"].as_str() else {
        return empty_action_plan(next_node);
    };
    if !matches!(kind, "toggle-command" | "agent-picker") {
        return empty_action_plan(next_node);
    }
    let Some(cmd) = mid["command"].as_str().filter(|s| !s.is_empty()) else {
        return empty_action_plan(next_node);
    };
    let delivery = if mid["delivery"].as_str() == Some("type") {
        CommandDelivery::Type
    } else {
        CommandDelivery::Write
    };
    ModelPickPlan {
        outcome: Some(ModelPickOutcome::ActionSent),
        typed_commands: vec![TypedCommand {
            command: cmd.to_owned(),
            delivery,
            expected_model_label: None,
        }],
        toast: None,
        hold_claude_default_model: false,
        pending_next_choice: None,
        launch_stale_after: false,
        pane_menu_values_before: None,
        effective_id_before: None,
        node: next_node,
    }
}

fn empty_action_plan(node: PaneModelNode) -> ModelPickPlan {
    ModelPickPlan {
        outcome: None,
        typed_commands: Vec::new(),
        toast: None,
        hold_claude_default_model: false,
        pending_next_choice: None,
        launch_stale_after: false,
        pane_menu_values_before: None,
        effective_id_before: None,
        node,
    }
}

/// Plans a `pane menu > Model` value pick using canonical SHA-256 [`LaunchSignature`]
/// (`launch_fingerprint::of` compatible) for launch identity comparison.
#[must_use]
pub fn plan_model_pick(
    node: &PaneModelNode,
    ctx: &PanePickContext<'_>,
    option_id: &str,
    value: Option<&Value>,
    claude_outcome: Option<ClaudeSwitchOutcome>,
) -> ModelPickPlan {
    let mut next_node = node.clone();
    let has_model_choice =
        ctx.is_agent && !node.detected && catalog::find(&node.agent_id).is_some();
    let pane_model_list = if has_model_choice {
        catalog::models_for(&node.agent_id, ctx.discovered_models)
    } else {
        Vec::new()
    };
    let effective_id =
        effective_model_id(&pane_model_list, ctx.agent_model.map(|m| m.model.as_str()))
            .map(str::to_owned);
    let menu_vals_before = pane_menu_values(
        &node.agent_id,
        ctx.pane_running,
        &pane_model_list,
        ctx.agent_model,
        node.session_options.as_ref(),
    );
    let pane_vals = launch_session_values(
        node.session_options.as_ref(),
        ctx.persisted_defaults,
        &node.agent_id,
    );

    let Some(cat) = catalog::find(&node.agent_id) else {
        let stale = compute_stale(&next_node, &pane_model_list, ctx);
        return ModelPickPlan {
            outcome: None,
            typed_commands: Vec::new(),
            toast: None,
            hold_claude_default_model: false,
            pending_next_choice: None,
            launch_stale_after: stale,
            pane_menu_values_before: menu_vals_before,
            effective_id_before: effective_id,
            node: next_node,
        };
    };

    let base = if node.agent_id == "cursor" && ctx.pane_running && menu_vals_before.is_some() {
        menu_vals_before.as_ref()
    } else {
        pane_vals.as_ref()
    };
    let next = next_pane_values(
        &node.agent_id,
        &pane_model_list,
        base,
        effective_id.as_deref(),
        option_id,
        value,
    );
    let next_model = next
        .as_ref()
        .and_then(|v| v.get("model"))
        .and_then(Value::as_str);
    let apply_pair = option_apply_for(cat, &pane_model_list, &node.agent_id, option_id, next_model);
    let mid = apply_pair.map(|(a, _)| &a["midSession"]);
    let is_model_apply = apply_pair.is_some_and(|(_, is_m)| is_m);
    let value_present = value.is_some_and(|v| !v.is_null());

    let mut typed_commands = Vec::new();
    let mut hold_claude = false;
    let mut pending_next = None;

    let outcome = if ctx.pane_running
        && value_present
        && mid.and_then(|m| m["kind"].as_str()) == Some("picker-filter")
    {
        if ctx.pane_busy || ctx.pending {
            ModelPickOutcome::Busy
        } else {
            let row = if option_id == "model" {
                value.and_then(Value::as_str).and_then(|id| {
                    pane_model_list
                        .iter()
                        .find(|m| m["id"].as_str() == Some(id))
                })
            } else {
                None
            };
            let filter = cursor_picker_filter(row);
            let cmd = if filter.is_empty() {
                "/model".to_owned()
            } else {
                format!("/model {filter}")
            };
            typed_commands.push(TypedCommand {
                command: cmd,
                delivery: CommandDelivery::Write,
                expected_model_label: None,
            });
            if filter.is_empty() {
                ModelPickOutcome::Picker
            } else {
                ModelPickOutcome::Sent
            }
        }
    } else if !ctx.pane_running
        || !value_present
        || mid.and_then(|m| m["kind"].as_str()) != Some("command")
    {
        next_node.session_options = next;
        ModelPickOutcome::Saved
    } else if ctx.pane_busy || ctx.pending {
        ModelPickOutcome::Busy
    } else if mid.and_then(|m| m["detectAgentInteraction"].as_str())
        == Some("claude-model-switch-confirmation")
    {
        let val_str = value.and_then(Value::as_str).unwrap_or_default();
        hold_claude = true;
        typed_commands.push(TypedCommand {
            command: format!("/model {val_str}"),
            delivery: CommandDelivery::Write,
            expected_model_label: Some(model_choice_label(&pane_model_list, val_str).to_owned()),
        });
        if let Some(settled) = claude_outcome {
            apply_claude_outcome_to_node(&mut next_node, next, settled, &pane_model_list, ctx)
        } else {
            pending_next = next;
            ModelPickOutcome::AwaitClaudeConfirmation
        }
    } else {
        let val_str = value.and_then(Value::as_str).unwrap_or_default();
        let sent = if matches!(node.agent_id.as_str(), "cursor" | "antigravity") && is_model_apply {
            composed_model_id(&node.agent_id, next.as_ref(), Some(&pane_model_list))
                .unwrap_or_else(|| val_str.to_owned())
        } else {
            val_str.to_owned()
        };
        let cmd = if is_model_apply {
            format!("/model {sent}")
        } else {
            format!("/effort {sent}")
        };
        let delivery = if mid.and_then(|m| m["delivery"].as_str()) == Some("type") {
            CommandDelivery::Type
        } else {
            CommandDelivery::Write
        };
        typed_commands.push(TypedCommand {
            command: cmd,
            delivery,
            expected_model_label: None,
        });
        adopt_live_on_node(&mut next_node, next, &pane_model_list, ctx);
        ModelPickOutcome::Sent
    };

    if outcome == ModelPickOutcome::Picker {
        next_node.chat_view = false;
    }
    let toast = toast_for_outcome(outcome, ctx.french);
    let launch_stale_after = compute_stale(&next_node, &pane_model_list, ctx);

    ModelPickPlan {
        outcome: Some(outcome),
        typed_commands,
        toast,
        hold_claude_default_model: hold_claude,
        pending_next_choice: pending_next,
        launch_stale_after,
        pane_menu_values_before: menu_vals_before,
        effective_id_before: effective_id,
        node: next_node,
    }
}

fn compute_stale(
    node: &PaneModelNode,
    pane_model_list: &[Value],
    ctx: &PanePickContext<'_>,
) -> bool {
    let has_cmd = node.agent_command.as_deref().is_some_and(|c| !c.is_empty());
    if !has_cmd || node.launch_sig.is_none() || node.detected {
        return false;
    }
    let now_sig = compute_node_signature(node, node.session_options.as_ref(), pane_model_list, ctx);
    is_launch_stale(node.launch_sig.as_ref(), has_cmd, node.detected, &now_sig)
}

fn apply_claude_outcome_to_node(
    node: &mut PaneModelNode,
    next: Option<Value>,
    outcome: ClaudeSwitchOutcome,
    pane_model_list: &[Value],
    ctx: &PanePickContext<'_>,
) -> ModelPickOutcome {
    match outcome {
        ClaudeSwitchOutcome::Applied => {
            adopt_live_on_node(node, next, pane_model_list, ctx);
            ModelPickOutcome::Applied
        }
        ClaudeSwitchOutcome::Rejected => ModelPickOutcome::Rejected,
        ClaudeSwitchOutcome::Unknown => {
            node.session_options = next;
            ModelPickOutcome::Unknown
        }
    }
}

/// Completes a deferred [`ModelPickOutcome::AwaitClaudeConfirmation`] plan once
/// [`ClaudeModelSwitchObserver`] settles.
#[must_use]
pub fn apply_claude_switch_outcome(
    node: &PaneModelNode,
    pending_next_choice: Option<Value>,
    outcome: ClaudeSwitchOutcome,
    ctx: &PanePickContext<'_>,
) -> (ModelPickOutcome, Option<PlannedToast>, bool, PaneModelNode) {
    let mut next_node = node.clone();
    let has_model_choice =
        ctx.is_agent && !node.detected && catalog::find(&node.agent_id).is_some();
    let pane_model_list = if has_model_choice {
        catalog::models_for(&node.agent_id, ctx.discovered_models)
    } else {
        Vec::new()
    };
    let mapped = apply_claude_outcome_to_node(
        &mut next_node,
        pending_next_choice,
        outcome,
        &pane_model_list,
        ctx,
    );
    let toast = toast_for_outcome(mapped, ctx.french);
    let stale = compute_stale(&next_node, &pane_model_list, ctx);
    (mapped, toast, stale, next_node)
}

/// Pure planner for native `ChatView` `setOption(optionId, value)` (`TerminalPane.vue:898-930`).
#[must_use]
pub fn plan_chat_view_set_option(
    node: &PaneModelNode,
    ctx: &PanePickContext<'_>,
    option_id: &str,
    value: Option<&Value>,
    claude_outcome: Option<ClaudeSwitchOutcome>,
) -> ChatSetOptionPlan {
    let has_model_choice =
        ctx.is_agent && !node.detected && catalog::find(&node.agent_id).is_some();
    let chat_available = ctx.chat_view_available && has_model_choice;
    let val_str = value.and_then(Value::as_str).unwrap_or_default();

    if !chat_available || !matches!(option_id, "model" | "effort") || val_str.is_empty() {
        let msg = if ctx.french {
            "Cette option n’est pas disponible pour cet agent."
        } else {
            "This option is not available for this agent."
        };
        return failed_chat_plan(node.clone(), msg);
    }

    let pane_model_list = catalog::models_for(&node.agent_id, ctx.discovered_models);
    let effective_id =
        effective_model_id(&pane_model_list, ctx.agent_model.map(|m| m.model.as_str()));
    if option_id == "effort"
        && let Some(cat) = catalog::find(&node.agent_id)
    {
        let has_effort =
            catalog::model_options(cat, Some(&pane_model_list), effective_id.unwrap_or(""))
                .iter()
                .any(|o| o["id"] == "effort");
        if !has_effort {
            let msg = if ctx.french {
                "Ce modèle n'a pas d'effort de raisonnement à choisir."
            } else {
                "This model has no reasoning effort to choose."
            };
            return failed_chat_plan(node.clone(), msg);
        }
    }

    if !ctx.pane_running {
        let msg = if ctx.french {
            "S'applique au démarrage de l'agent."
        } else {
            "Applies when the agent starts."
        };
        return failed_chat_plan(node.clone(), msg);
    }

    if ctx.pane_busy || ctx.pending {
        let msg = if ctx.french {
            "Il travaille : son modèle pourra changer une fois qu'il sera inactif."
        } else {
            "It is working: its model can change once it is idle."
        };
        return failed_chat_plan(node.clone(), msg);
    }

    if ctx.user_typing {
        let msg = if ctx.french {
            "Une ligne est tapée dans son terminal : envoyez-la ou effacez-la d'abord."
        } else {
            "A line is typed in its terminal: send or clear it there first."
        };
        return failed_chat_plan(node.clone(), msg);
    }

    let mut pick_plan = plan_model_pick(node, ctx, option_id, value, claude_outcome);
    // Note: ChatView `setOption` returns `{ ok: true, picker: true }` on Cursor's
    // picker without deleting `node.chatView` itself (`TerminalPane.vue:917-920`).
    if pick_plan.outcome == Some(ModelPickOutcome::Picker) {
        pick_plan.node.chat_view = node.chat_view;
    }

    let chat_result = match pick_plan.outcome {
        Some(
            ModelPickOutcome::Applied
            | ModelPickOutcome::Sent
            | ModelPickOutcome::AwaitClaudeConfirmation,
        ) => ChatSetOptionResult {
            ok: true,
            picker: false,
            error: None,
        },
        Some(ModelPickOutcome::Picker) => ChatSetOptionResult {
            ok: true,
            picker: true,
            error: None,
        },
        Some(ModelPickOutcome::Rejected) => ChatSetOptionResult {
            ok: false,
            picker: false,
            error: Some(if ctx.french {
                "Claude a conservé le modèle actuel.".into()
            } else {
                "Claude kept the current model.".into()
            }),
        },
        Some(ModelPickOutcome::Unknown) => ChatSetOptionResult {
            ok: false,
            picker: false,
            error: Some(if ctx.french {
                "Impossible de vérifier le changement de modèle ; ouvrez le terminal pour vérifier."
                    .into()
            } else {
                "Could not verify the model change; open the terminal to check.".into()
            }),
        },
        Some(ModelPickOutcome::Busy) => ChatSetOptionResult {
            ok: false,
            picker: false,
            error: Some(if ctx.french {
                "Il travaille : son modèle pourra changer une fois qu'il sera inactif.".into()
            } else {
                "It is working: its model can change once it is idle.".into()
            }),
        },
        _ => ChatSetOptionResult {
            ok: false,
            picker: false,
            error: Some(if ctx.french {
                "Un modèle choisi ici s'applique au redémarrage de l'agent.".into()
            } else {
                "A model picked here applies when the agent restarts.".into()
            }),
        },
    };

    ChatSetOptionPlan {
        chat_result,
        typed_commands: pick_plan.typed_commands,
        hold_claude_default_model: pick_plan.hold_claude_default_model,
        pending_next_choice: pick_plan.pending_next_choice,
        node: pick_plan.node,
    }
}

fn failed_chat_plan(node: PaneModelNode, msg: &str) -> ChatSetOptionPlan {
    ChatSetOptionPlan {
        chat_result: ChatSetOptionResult {
            ok: false,
            picker: false,
            error: Some(msg.to_owned()),
        },
        typed_commands: Vec::new(),
        hold_claude_default_model: false,
        pending_next_choice: None,
        node,
    }
}

// -----------------------------------------------------------------------------
// 4. Claude `/model` Switch Confirmation Observer & ANSI Stripping
// -----------------------------------------------------------------------------

/// Strips OSC, CSI, and single-character terminal escape sequences (`claudeModelSwitch.js:stripAnsi`).
#[must_use]
pub fn strip_ansi(value: &str) -> String {
    let pass1 = strip_osc_sequences(value);
    let pass2 = strip_csi_sequences(&pass1);
    strip_single_escapes(&pass2)
}

fn strip_osc_sequences(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut last = 0;
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == 0x1b && bytes[i + 1] == b']' {
            let body_start = i + 2;
            let limit = input[body_start..]
                .find('\x07')
                .map_or(input.len(), |off| body_start + off);
            if limit < input.len() {
                out.push_str(&input[last..i]);
                i = limit + 1;
                last = i;
                continue;
            }
            if let Some(st_rel) = input[body_start..limit].rfind("\x1b\\") {
                out.push_str(&input[last..i]);
                i = body_start + st_rel + 2;
                last = i;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&input[last..]);
    out
}

fn strip_csi_sequences(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut last = 0;
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == 0x1b && bytes[i + 1] == b'[' {
            let mut j = i + 2;
            while j < bytes.len() && (0x30..=0x3f).contains(&bytes[j]) {
                j += 1;
            }
            while j < bytes.len() && (0x20..=0x2f).contains(&bytes[j]) {
                j += 1;
            }
            if j < bytes.len() && (0x40..=0x7e).contains(&bytes[j]) {
                out.push_str(&input[last..i]);
                i = j + 1;
                last = i;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&input[last..]);
    out
}

fn strip_single_escapes(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut last = 0;
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == 0x1b {
            let b1 = bytes[i + 1];
            if (0x40..=0x5a).contains(&b1) || (0x5c..=0x5f).contains(&b1) || b1 == b'c' {
                out.push_str(&input[last..i]);
                i += 2;
                last = i;
                continue;
            }
            if b"()*+-./".contains(&b1)
                && i + 2 < bytes.len()
                && (0x30..=0x7e).contains(&bytes[i + 2])
            {
                out.push_str(&input[last..i]);
                i += 3;
                last = i;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&input[last..]);
    out
}

fn compact_ansi_buffer(buffer: &str) -> String {
    strip_ansi(buffer)
        .chars()
        .filter(|c| !js_space(*c))
        .collect::<String>()
        .to_lowercase()
}

/// Detects Claude Code's cached-conversation model-switch confirmation prompt.
#[must_use]
pub fn has_claude_model_switch_confirmation(buffer: &str) -> bool {
    let text = compact_ansi_buffer(buffer);
    text.contains("switchmodel?") && text.contains("thisconversationiscachedforthecurrentmodel")
}

/// Detects Claude Code's `"Set model to …"` echo matching `model_label` (exact or
/// ordered version-resolved tokens, e.g. `"Opus"` matching `"Opus 5.5 (1M context)"`).
#[must_use]
pub fn has_claude_model_switch_success(buffer: &str, model_label: &str) -> bool {
    let text = compact_ansi_buffer(buffer);
    let compact_label: String = model_label
        .chars()
        .filter(|c| !js_space(*c))
        .collect::<String>()
        .to_lowercase();
    if text.contains(&format!("setmodelto{compact_label}")) {
        return true;
    }
    let Some(success_start) = text.rfind("setmodelto") else {
        return false;
    };
    let lower_label = model_label.to_lowercase();
    let tokens = extract_label_tokens(&lower_label);
    if tokens.is_empty() {
        return false;
    }
    let success_text = &text[success_start..];
    let mut token_end = 0;
    for tok in tokens {
        let Some(rel) = success_text[token_end..].find(tok) else {
            return false;
        };
        token_end += rel + tok.len();
    }
    true
}

fn extract_label_tokens(lower: &str) -> Vec<&str> {
    let bytes = lower.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_lowercase() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_lowercase() {
                i += 1;
            }
            out.push(&lower[start..i]);
        } else if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            while i < bytes.len() && bytes[i].is_ascii_lowercase() {
                i += 1;
            }
            out.push(&lower[start..i]);
        } else {
            i += 1;
        }
    }
    out
}

/// Detects Claude Code's `"Kept model as …"` rejection line.
#[must_use]
pub fn has_claude_model_switch_rejection(buffer: &str) -> bool {
    compact_ansi_buffer(buffer).contains("keptmodelas")
}

/// Step result returned by [`ClaudeModelSwitchObserver::observe`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaudeObserverStep {
    /// Still watching PTY output; no action needed yet.
    Pending,
    /// Cached-conversation prompt detected: caller should write `"\r"` to the PTY
    /// (and call [`ClaudeModelSwitchObserver::dispose`] if writing fails).
    SubmitEnter,
    /// Terminal settled on a final switch outcome.
    Settled(ClaudeSwitchOutcome),
}

/// Bounded state machine mirroring `claudeModelSwitch.js:createClaudeModelSwitchObserver`.
#[derive(Clone)]
pub struct ClaudeModelSwitchObserver {
    expected_model_label: String,
    timeout_ms: u64,
    armed: bool,
    settled: Option<ClaudeSwitchOutcome>,
    confirmation_submitted: bool,
    deadline_ms: Option<u64>,
    observed_utf16: Vec<u16>,
}

impl std::fmt::Debug for ClaudeModelSwitchObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeModelSwitchObserver")
            .field("expected_model_label", &self.expected_model_label)
            .field("timeout_ms", &self.timeout_ms)
            .field("armed", &self.armed)
            .field("settled", &self.settled)
            .field("confirmation_submitted", &self.confirmation_submitted)
            .field("deadline_ms", &self.deadline_ms)
            .field("observed_utf16_len", &self.observed_utf16.len())
            .finish()
    }
}

impl ClaudeModelSwitchObserver {
    #[must_use]
    pub fn new(expected_model_label: impl Into<String>, timeout_ms: u64) -> Self {
        Self {
            expected_model_label: expected_model_label.into(),
            timeout_ms: if timeout_ms == 0 {
                CLAUDE_SWITCH_TIMEOUT_MS
            } else {
                timeout_ms
            },
            armed: false,
            settled: None,
            confirmation_submitted: false,
            deadline_ms: None,
            observed_utf16: Vec::with_capacity(MAX_CLAUDE_OBSERVED_UTF16),
        }
    }

    /// Arms PTY output collection before sending `/model <id>`.
    pub fn arm(&mut self) {
        if self.settled.is_none() {
            self.armed = true;
        }
    }

    /// Starts the detection timeout once `/model <id>` has been delivered.
    pub fn start_detection(&mut self, now_ms: u64) {
        if self.settled.is_none() {
            self.deadline_ms = Some(now_ms.saturating_add(self.timeout_ms));
        }
    }

    #[must_use]
    pub fn deadline_ms(&self) -> Option<u64> {
        self.deadline_ms
    }

    #[must_use]
    pub fn outcome(&self) -> Option<ClaudeSwitchOutcome> {
        self.settled
    }

    /// Feeds a chunk of PTY output, maintaining at most [`MAX_CLAUDE_OBSERVED_UTF16`]
    /// UTF-16 code units (`observed = "${observed}${data}".slice(-MAX_OBSERVED)`).
    pub fn observe(&mut self, chunk: &str, now_ms: u64) -> ClaudeObserverStep {
        if let Some(done) = self.settled {
            return ClaudeObserverStep::Settled(done);
        }
        if !self.armed {
            return ClaudeObserverStep::Pending;
        }
        if let Some(done) = self.check_timeout(now_ms) {
            return ClaudeObserverStep::Settled(done);
        }
        // Trim before extending: a large PTY chunk must not cause a temporary
        // allocation proportional to the entire incoming output.
        let units = chunk.encode_utf16().count();
        let kept = units.min(MAX_CLAUDE_OBSERVED_UTF16);
        let drop_n = (self.observed_utf16.len() + kept).saturating_sub(MAX_CLAUDE_OBSERVED_UTF16);
        self.observed_utf16.drain(..drop_n);
        self.observed_utf16
            .extend(chunk.encode_utf16().skip(units - kept).take(kept));
        let observed = String::from_utf16_lossy(&self.observed_utf16);
        if !self.expected_model_label.is_empty()
            && has_claude_model_switch_success(&observed, &self.expected_model_label)
        {
            return self.finish(ClaudeSwitchOutcome::Applied);
        }
        if has_claude_model_switch_rejection(&observed) {
            return self.finish(ClaudeSwitchOutcome::Rejected);
        }
        if !self.confirmation_submitted && has_claude_model_switch_confirmation(&observed) {
            self.confirmation_submitted = true;
            self.deadline_ms = Some(now_ms.saturating_add(self.timeout_ms));
            return ClaudeObserverStep::SubmitEnter;
        }
        ClaudeObserverStep::Pending
    }

    /// Checks whether the active detection window has expired at `now_ms`.
    pub fn check_timeout(&mut self, now_ms: u64) -> Option<ClaudeSwitchOutcome> {
        if let Some(done) = self.settled {
            return Some(done);
        }
        if let Some(deadline) = self.deadline_ms
            && now_ms >= deadline
        {
            let _ = self.finish(ClaudeSwitchOutcome::Unknown);
            return self.settled;
        }
        None
    }

    /// Aborts observation and settles to [`ClaudeSwitchOutcome::Unknown`].
    pub fn dispose(&mut self) -> ClaudeSwitchOutcome {
        let _ = self.finish(ClaudeSwitchOutcome::Unknown);
        self.settled.unwrap_or(ClaudeSwitchOutcome::Unknown)
    }

    fn finish(&mut self, outcome: ClaudeSwitchOutcome) -> ClaudeObserverStep {
        if self.settled.is_none() {
            self.settled = Some(outcome);
            self.deadline_ms = None;
        }
        ClaudeObserverStep::Settled(self.settled.unwrap_or(outcome))
    }
}

#[cfg(test)]
mod observer_boundary_tests {
    use super::*;
    #[test]
    fn oversized_chunks_never_grow_the_buffer_and_expired_output_cannot_confirm() {
        let mut observer = ClaudeModelSwitchObserver::new("Opus", 0);
        observer.arm();
        observer.start_detection(100);
        let chunk = "🔥".repeat(MAX_CLAUDE_OBSERVED_UTF16 * 8);
        assert_eq!(observer.observe(&chunk, 101), ClaudeObserverStep::Pending);
        assert_eq!(observer.observed_utf16.len(), MAX_CLAUDE_OBSERVED_UTF16);
        assert_eq!(
            observer.observed_utf16.capacity(),
            MAX_CLAUDE_OBSERVED_UTF16
        );
        for late in [
            "Set model to Opus 5.5",
            "Switch model? This conversation is cached for the current model.",
        ] {
            let mut observer = ClaudeModelSwitchObserver::new("Opus", 0);
            observer.arm();
            observer.start_detection(100);
            assert_eq!(
                observer.observe(late, 5100),
                ClaudeObserverStep::Settled(ClaudeSwitchOutcome::Unknown)
            );
            assert!(!observer.confirmation_submitted);
        }
    }
}

// -----------------------------------------------------------------------------
// 5. Bounded Terminal Screen Readers (Cursor, Codex, Antigravity, Muse, Generic)
// -----------------------------------------------------------------------------

fn bounded_tail<T>(slice: &[T]) -> &[T] {
    if slice.len() > MAX_SCREEN_LINES {
        &slice[slice.len() - MAX_SCREEN_LINES..]
    } else {
        slice
    }
}

fn bounded_head<T>(slice: &[T]) -> &[T] {
    if slice.len() > MAX_SCREEN_LINES {
        &slice[..MAX_SCREEN_LINES]
    } else {
        slice
    }
}

fn within_screen_line_cap(line: &str) -> bool {
    line.encode_utf16().take(MAX_SCREEN_LINE_UTF16 + 1).count() <= MAX_SCREEN_LINE_UTF16
}

fn cursor_zero_width(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}')
}

fn cursor_tidy(text: &str) -> String {
    let stripped: String = text.chars().filter(|c| !cursor_zero_width(*c)).collect();
    stripped
        .split(js_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_cursor_size_word(word: &str) -> bool {
    let bytes = word.as_bytes();
    if bytes.len() < 2 || !matches!(bytes[bytes.len() - 1], b'K' | b'k' | b'M' | b'm') {
        return false;
    }
    is_decimal_number(&word[..word.len() - 1])
}

fn is_decimal_number(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_digit() {
        return false;
    }
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == bytes.len() {
        return true;
    }
    if bytes[i] != b'.' || i + 1 >= bytes.len() {
        return false;
    }
    bytes[i + 1..].iter().all(u8::is_ascii_digit)
}

fn replace_case_insensitive(haystack: &str, needle_lower: &str, replacement: &str) -> String {
    let nlen = needle_lower.len();
    if nlen == 0 || haystack.len() < nlen {
        return haystack.to_owned();
    }
    let mut out = String::with_capacity(haystack.len());
    let mut i = 0;
    while i < haystack.len() {
        if i + nlen <= haystack.len()
            && haystack.is_char_boundary(i)
            && haystack.is_char_boundary(i + nlen)
            && haystack[i..i + nlen].eq_ignore_ascii_case(needle_lower)
        {
            out.push_str(replacement);
            i += nlen;
        } else if let Some(ch) = haystack[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

fn cursor_plain_name(label: &str) -> String {
    let tidied = cursor_tidy(label);
    let no_zdr = replace_case_insensitive(&tidied, "(no zdr)", " ");
    no_zdr
        .split(' ')
        .filter(|w| !w.is_empty() && !is_cursor_size_word(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Filter text typed into Cursor's `/model <filter>` picker (`cursorModels.js:cursorPickerFilter`).
#[must_use]
pub fn cursor_picker_filter(model: Option<&Value>) -> String {
    let Some(row) = model.filter(|v| v.is_object()) else {
        return String::new();
    };
    let raw = row["label"]
        .as_str()
        .or_else(|| row["id"].as_str())
        .unwrap_or_default();
    let name = cursor_plain_name(raw);
    let len = name.encode_utf16().count();
    let safe = (1..=80).contains(&len)
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_ .()+-".contains(&b));
    if safe { name } else { String::new() }
}

const CURSOR_STATUS_EFFORTS: [(&str, &str); 7] = [
    ("xhigh", "Extra High"),
    ("minimal", "Minimal"),
    ("medium", "Medium"),
    ("none", "None"),
    ("high", "High"),
    ("low", "Low"),
    ("max", "Max"),
];

#[derive(Clone, Debug)]
struct CursorStatusParams {
    effort: &'static str,
    fast: bool,
    thinking: bool,
}

fn parse_cursor_status_parameters(rest: &str) -> Option<CursorStatusParams> {
    let mut effort = "";
    let mut fast = false;
    let mut thinking = false;
    let mut s = format!(" {rest} ");
    for (eff_id, word) in CURSOR_STATUS_EFFORTS {
        let pat = format!(" {word} ");
        if let Some(pos) = s.find(&pat) {
            effort = eff_id;
            s.replace_range(pos..pos + pat.len(), " ");
            break;
        }
    }
    for w in s.split(' ').filter(|w| !w.is_empty()) {
        if is_cursor_size_word(w) {
            continue;
        }
        if w == "Fast" {
            fast = true;
        } else if w == "Thinking" {
            thinking = true;
        } else {
            return None;
        }
    }
    Some(CursorStatusParams {
        effort,
        fast,
        thinking,
    })
}

/// Parsed Cursor status-bar model reading (`cursorModels.js:cursorModelFromStatusLine`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorScreenModel {
    pub model: String,
    pub row: String,
    pub name: String,
    pub effort: Option<String>,
    pub fast_mode: bool,
    pub thinking: bool,
}

/// Parses a single Cursor status segment (`"Codex 5.3 272K High Fast"`) against grouped rows.
#[must_use]
pub fn cursor_model_from_status_line(line: &str, models: &[Value]) -> Option<CursorScreenModel> {
    if !within_screen_line_cap(line) {
        return None;
    }
    let text = cursor_tidy(line);
    let text_len = text.encode_utf16().count();
    if text.is_empty() || text_len > 120 {
        return None;
    }
    let mut best: Option<(String, usize, &Value, CursorStatusParams)> = None;
    for m in models {
        let Some(label) = m["label"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        let name = cursor_plain_name(label);
        let name_utf16 = name.encode_utf16().count();
        if name.is_empty() {
            continue;
        }
        let matches = text == name
            || (text.len() > name.len()
                && text.starts_with(&name)
                && text.as_bytes()[name.len()] == b' ');
        if !matches {
            continue;
        }
        if best
            .as_ref()
            .is_some_and(|(_, best_len, _, _)| *best_len >= name_utf16)
        {
            continue;
        }
        let rest = trim(&text[name.len()..]);
        if let Some(params) = parse_cursor_status_parameters(rest) {
            best = Some((name, name_utf16, m, params));
        }
    }
    let (best_name, _, model, params) = best?;
    let row_id = model["id"].as_str()?.to_owned();
    let effort_supported = !params.effort.is_empty()
        && model["efforts"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|e| e.as_str() == Some(params.effort)));
    let effort = effort_supported.then_some(params.effort);

    let mut picked = Map::new();
    if let Some(eff) = effort {
        picked.insert("effort".into(), Value::String(eff.to_owned()));
    }
    picked.insert("fastMode".into(), Value::Bool(params.fast));
    if params.thinking {
        picked.insert("thinking".into(), Value::Bool(true));
    }
    let composed_id = if model["variants"].is_array() {
        compose_cursor_row_variant(model, &picked)
    } else {
        row_id.clone()
    };

    let mut display_parts = vec![best_name];
    if params.fast {
        display_parts.push("Fast".into());
    }
    if params.thinking {
        display_parts.push("Thinking".into());
    }
    let resolved_effort = effort.map(str::to_owned).or_else(|| {
        model["efforts"]
            .as_array()
            .filter(|arr| arr.len() > 1)
            .and_then(|_| model["defaultEffort"].as_str().map(str::to_owned))
    });

    Some(CursorScreenModel {
        model: composed_id,
        row: row_id,
        name: display_parts.join(" "),
        effort: resolved_effort,
        fast_mode: params.fast,
        thinking: params.thinking,
    })
}

/// Returns `true` when Cursor's `/model` picker overlay is visible (`cursorModels.js:cursorPickerShown`).
#[must_use]
pub fn cursor_picker_shown<S: AsRef<str>>(lines: &[S]) -> bool {
    bounded_tail(lines).iter().any(|raw| {
        let line = raw.as_ref();
        if !within_screen_line_cap(line) {
            return false;
        }
        if line.contains("Type to filter") || line.contains("Enter to select") {
            return true;
        }
        let trimmed = line.trim_start_matches(js_space);
        has_word_prefix(trimmed, "Available models") || has_word_prefix(trimmed, "Models matching")
    })
}

fn has_word_prefix(s: &str, prefix: &str) -> bool {
    let Some(rest) = s.strip_prefix(prefix) else {
        return false;
    };
    rest.bytes()
        .next()
        .is_none_or(|b| !(b.is_ascii_alphanumeric() || b == b'_'))
}

fn is_cursor_status_tail(part: &str) -> bool {
    if part == "MAX" {
        return true;
    }
    if let Some(num) = part.strip_suffix('%') {
        return is_decimal_number(num);
    }
    if let Some(before) = part
        .strip_suffix(" tokens")
        .or_else(|| part.strip_suffix(" token"))
    {
        let num = before
            .strip_suffix('k')
            .or_else(|| before.strip_suffix('M'))
            .unwrap_or(before);
        return is_decimal_number(num);
    }
    if let Some(before) = part
        .strip_suffix(" files edited")
        .or_else(|| part.strip_suffix(" file edited"))
    {
        return !before.is_empty() && before.bytes().all(|b| b.is_ascii_digit());
    }
    false
}

struct CursorParsedStatusLine {
    model: CursorScreenModel,
    segment: String,
    rest: Vec<String>,
}

fn parse_cursor_status_line(line: &str, models: &[Value]) -> Option<CursorParsedStatusLine> {
    if !within_screen_line_cap(line) {
        return None;
    }
    let tidied = cursor_tidy(line);
    let parts: Vec<&str> = tidied.split(" · ").collect();
    if parts.len() > 6 {
        return None;
    }
    for i in 0..parts.len().min(2) {
        let Some(model) = cursor_model_from_status_line(parts[i], models) else {
            continue;
        };
        let rest = &parts[i + 1..];
        return rest
            .iter()
            .all(|p| is_cursor_status_tail(p))
            .then(|| CursorParsedStatusLine {
                model,
                segment: parts[i].to_owned(),
                rest: rest.iter().map(|s| (*s).to_owned()).collect(),
            });
    }
    None
}

/// Reads Cursor's active model from its bottom status bar (`cursorModels.js:cursorModelOnScreen`).
#[must_use]
pub fn cursor_model_on_screen<S: AsRef<str>>(
    lines: &[S],
    models: &[Value],
) -> Option<CursorScreenModel> {
    let slice = bounded_tail(lines);
    if cursor_picker_shown(slice) {
        return None;
    }
    for raw in slice.iter().rev() {
        let line = raw.as_ref();
        if trim(line).is_empty() {
            continue;
        }
        if let Some(found) = parse_cursor_status_line(line, models) {
            return Some(found.model);
        }
    }
    None
}

/// Context usage parsed from Cursor's status bar (`cursorModels.js:cursorContextOnScreen`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorContextOnScreen {
    NoneYet,
    Usage {
        used_tokens: u64,
        window_tokens: u64,
        percentage: u64,
    },
}

impl CursorContextOnScreen {
    #[must_use]
    pub fn to_value(self) -> Value {
        match self {
            Self::NoneYet => json!({ "none": true }),
            Self::Usage {
                used_tokens,
                window_tokens,
                percentage,
            } => json!({
                "usedTokens": used_tokens,
                "windowTokens": window_tokens,
                "percentage": percentage,
            }),
        }
    }
}

fn size_tokens(word: &str) -> u64 {
    if !is_cursor_size_word(word) {
        return 0;
    }
    let unit = word.as_bytes()[word.len() - 1];
    let Ok(num) = word[..word.len() - 1].parse::<f64>() else {
        return 0;
    };
    let mult = if matches!(unit, b'M' | b'm') {
        1_000_000.0
    } else {
        1_000.0
    };
    js_round_non_neg(num * mult)
}

fn js_round_non_neg(x: f64) -> u64 {
    if !x.is_finite() || x <= 0.0 {
        0
    } else {
        (x + 0.5).floor() as u64
    }
}

/// Reads Cursor's context usage from its status bar (`cursorModels.js:cursorContextOnScreen`).
#[must_use]
pub fn cursor_context_on_screen<S: AsRef<str>>(
    lines: &[S],
    models: &[Value],
) -> Option<CursorContextOnScreen> {
    let slice = bounded_tail(lines);
    if cursor_picker_shown(slice) {
        return None;
    }
    for raw in slice.iter().rev() {
        let line = raw.as_ref();
        if trim(line).is_empty() {
            continue;
        }
        let Some(found) = parse_cursor_status_line(line, models) else {
            continue;
        };
        let listed_label = models
            .iter()
            .find(|m| m["id"].as_str() == Some(found.model.row.as_str()))
            .and_then(|m| m["label"].as_str())
            .unwrap_or_default();
        let window_tokens = found
            .segment
            .split(' ')
            .map(size_tokens)
            .find(|n| *n > 0)
            .or_else(|| {
                cursor_tidy(listed_label)
                    .split(' ')
                    .map(size_tokens)
                    .find(|n| *n > 0)
            })
            .unwrap_or(0);

        let pct = found.rest.iter().find(|p| p.ends_with('%'));
        let count = found
            .rest
            .iter()
            .find(|p| p.ends_with(" tokens") || p.ends_with(" token"));
        if window_tokens == 0 || (pct.is_none() && count.is_none()) {
            return Some(CursorContextOnScreen::NoneYet);
        }

        let used = if let Some(pct_str) = pct {
            let val = pct_str[..pct_str.len() - 1].parse::<f64>().unwrap_or(0.0);
            js_round_non_neg((val / 100.0) * window_tokens as f64)
        } else if let Some(count_str) = count {
            let first_word = count_str.split(' ').next().unwrap_or_default();
            let (num_str, mult) = if let Some(n) = first_word.strip_suffix('M') {
                (n, 1_000_000.0)
            } else if let Some(n) = first_word.strip_suffix('k') {
                (n, 1_000.0)
            } else {
                (first_word, 1.0)
            };
            let val = num_str.parse::<f64>().unwrap_or(0.0);
            js_round_non_neg(val * mult)
        } else {
            0
        };

        if used == 0 || (used as f64) > (window_tokens as f64) * 1.5 {
            return Some(CursorContextOnScreen::NoneYet);
        }
        let percentage = js_round_non_neg((used as f64 / window_tokens as f64) * 100.0);
        return Some(CursorContextOnScreen::Usage {
            used_tokens: used,
            window_tokens,
            percentage,
        });
    }
    None
}

/// Model & effort read from Codex's or Antigravity's footer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenAgentModel {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub effort: Option<String>,
}

const CODEX_SCREEN_EFFORTS: [&str; 7] =
    ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

fn codex_norm(s: &str) -> String {
    trim(s)
        .to_lowercase()
        .split(js_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn split_middle_dot_spaced(s: &str) -> &str {
    let mut i = 0;
    while i < s.len() {
        let Some(ch) = s[i..].chars().next() else {
            break;
        };
        if js_space(ch) {
            let start = i;
            let mut j = i + ch.len_utf8();
            while j < s.len()
                && let Some(c2) = s[j..].chars().next()
                && js_space(c2)
            {
                j += c2.len_utf8();
            }
            if let Some(after_dot) = s[j..].strip_prefix('·')
                && after_dot.chars().next().is_some_and(js_space)
            {
                return &s[..start];
            }
            i = j;
        } else {
            i += ch.len_utf8();
        }
    }
    s
}

fn is_codex_word_token(w: &str, allow_leading_digit: bool) -> bool {
    let bytes = w.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let first_ok =
        bytes[0].is_ascii_alphabetic() || (allow_leading_digit && bytes[0].is_ascii_digit());
    first_ok
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:/-".contains(b))
}

fn parse_codex_footer_segment(first: &str) -> Option<(String, Option<String>)> {
    let mut rest = first;
    // Strip optional trailing `\s+fast` (case-insensitive).
    if let Some((before_fast, last_tok)) = rsplit_once_js_space(rest)
        && last_tok.eq_ignore_ascii_case("fast")
    {
        rest = before_fast;
    }
    let mut effort = None;
    if let Some((before_eff, last_tok)) = rsplit_once_js_space(rest) {
        let lower = last_tok.to_ascii_lowercase();
        if CODEX_SCREEN_EFFORTS.contains(&lower.as_str()) {
            effort = Some(lower);
            rest = before_eff;
        }
    }
    // `rest` must match `^[A-Za-z][\w.:/-]*(?: [A-Za-z0-9][\w.:/-]*){0,2}$` (literal single spaces!).
    let words: Vec<&str> = rest.split(' ').collect();
    if !(1..=3).contains(&words.len()) {
        return None;
    }
    if !is_codex_word_token(words[0], false)
        || words[1..].iter().any(|w| !is_codex_word_token(w, true))
    {
        return None;
    }
    Some((rest.to_owned(), effort))
}

fn rsplit_once_js_space(s: &str) -> Option<(&str, &str)> {
    let trimmed = s.trim_end_matches(js_space);
    if trimmed.len() != s.len() {
        return None;
    }
    let idx = s.rfind(js_space)?;
    let tail = s[idx..].trim_start_matches(js_space);
    let head = s[..idx].trim_end_matches(js_space);
    if head.is_empty() || tail.is_empty() {
        None
    } else {
        Some((head, tail))
    }
}

fn is_codex_family_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    let prefix_ok = key.starts_with("gpt")
        || key.starts_with("codex")
        || (bytes.len() >= 2 && bytes[0] == b'o' && bytes[1].is_ascii_digit());
    prefix_ok
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(*b, b'_' | b'.' | b'-'))
}

/// Reads Codex CLI's model & effort from the footer block strictly below its `›`
/// input prompt box (`codexScreen.js:codexModelOnScreen`).
#[must_use]
pub fn codex_model_on_screen<S: AsRef<str>>(
    lines: &[S],
    models: &[Value],
) -> Option<ScreenAgentModel> {
    let slice = bounded_tail(lines);
    if slice.iter().any(|l| !within_screen_line_cap(l.as_ref())) {
        return None;
    }
    let rows: Vec<&str> = slice
        .iter()
        .map(|l| l.as_ref().trim_end_matches(js_space))
        .collect();
    let prompt = rows
        .iter()
        .rposition(|r| r.trim_start_matches(js_space).starts_with('›'))?;
    let mut end = rows.len();
    while end > prompt + 1 && trim(rows[end - 1]).is_empty() {
        end -= 1;
    }
    let blank = (prompt + 1..end)
        .rev()
        .find(|&i| trim(rows[i]).is_empty())?;
    let list: Vec<&Value> = models.iter().filter(|m| m["id"].is_string()).collect();

    for row in &rows[blank + 1..end.min(blank + 3)] {
        let first = split_middle_dot_spaced(trim(row));
        let Some((name, effort)) = parse_codex_footer_segment(first) else {
            continue;
        };
        let key = codex_norm(&name);
        let listed = list.iter().copied().find(|x| {
            x["id"].as_str().is_some_and(|id| codex_norm(id) == key)
                || x["label"]
                    .as_str()
                    .is_some_and(|lbl| codex_norm(lbl) == key)
        });
        if listed.is_none() {
            let has_ws = name.chars().any(js_space);
            let numeric_local = name.bytes().any(|b| b.is_ascii_digit())
                && name.bytes().any(|b| b == b'-' || b == b':');
            if has_ws || !(is_codex_family_key(&key) || numeric_local) {
                continue;
            }
        }
        let model_id = listed
            .and_then(|l| l["id"].as_str())
            .map_or_else(|| key.clone(), str::to_owned);
        let display_name = listed
            .and_then(|l| l["label"].as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        return Some(ScreenAgentModel {
            model: model_id,
            name: display_name,
            effort,
        });
    }
    None
}

const ANTIGRAVITY_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

fn valid_antigravity_clean_name(name: &str) -> bool {
    let clean: String = name.chars().filter(|c| !matches!(c, '(' | ')')).collect();
    let len = clean.encode_utf16().count();
    (1..=80).contains(&len)
        && clean.as_bytes()[0].is_ascii_alphanumeric()
        && clean
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_ .+:/-".contains(&b))
}

fn split_two_or_more_js_spaces(line: &str) -> Option<&str> {
    let mut last_split_end = None;
    let mut i = 0;
    while i < line.len() {
        let Some(ch) = line[i..].chars().next() else {
            break;
        };
        if js_space(ch) {
            let mut count = 1usize;
            let mut j = i + ch.len_utf8();
            while j < line.len()
                && let Some(c2) = line[j..].chars().next()
                && js_space(c2)
            {
                count += 1;
                j += c2.len_utf8();
            }
            if count >= 2 {
                last_split_end = Some(j);
            }
            i = j;
        } else {
            i += ch.len_utf8();
        }
    }
    last_split_end.map(|pos| &line[pos..])
}

fn parse_antigravity_dot_right(right: &str) -> Option<(String, String)> {
    // Matches `/^(.+?)\s+·\s+(low|medium|high|xhigh|max)$/i`.
    let (before_eff, eff_tok) = rsplit_once_js_space(right)?;
    let lower = eff_tok.to_ascii_lowercase();
    if !ANTIGRAVITY_EFFORTS.contains(&lower.as_str()) {
        return None;
    }
    let before_dot = before_eff.strip_suffix('·')?;
    let trimmed = before_dot.trim_end_matches(js_space);
    if trimmed.len() == before_dot.len() || trim(trimmed).is_empty() {
        return None;
    }
    Some((trim(trimmed).to_owned(), lower))
}

fn antigravity_row<'a>(list: &'a [Value], name: &str, effort: Option<&str>) -> Option<&'a Value> {
    let key = name.to_lowercase();
    list.iter()
        .find(|m| m["label"].as_str().is_some_and(|l| l.to_lowercase() == key))
        .or_else(|| {
            list.iter().find(|m| {
                let Some(lbl) = m["label"].as_str() else {
                    return false;
                };
                let Some((p_name, p_eff)) = agent_model_antigravity::label_parts(lbl) else {
                    return false;
                };
                p_name.to_lowercase() == key
                    && (effort.is_none() || p_eff.is_none() || p_eff == effort)
            })
        })
}

fn antigravity_row_name(row: &Value) -> String {
    let lbl = row["label"].as_str().unwrap_or_default();
    agent_model_antigravity::label_parts(lbl).map_or_else(|| lbl.to_owned(), |(name, _)| name)
}

/// Reads Antigravity (`agy`)'s right-aligned footer model (`antigravityModels.js:antigravityModelOnScreen`).
#[must_use]
pub fn antigravity_model_on_screen<S: AsRef<str>>(
    lines: &[S],
    models: &[Value],
) -> Option<ScreenAgentModel> {
    let slice = bounded_tail(lines);
    let rows: Vec<&str> = slice
        .iter()
        .map(AsRef::as_ref)
        .filter(|l| within_screen_line_cap(l))
        .map(|l| l.trim_end_matches(js_space))
        .filter(|l| !trim(l).is_empty())
        .collect();

    let min_idx = rows.len().saturating_sub(3);
    for row in rows[min_idx..].iter().rev() {
        let Some(right_raw) = split_two_or_more_js_spaces(row) else {
            continue;
        };
        let right = trim(right_raw);
        let (name, effort) = if let Some((n, e)) = parse_antigravity_dot_right(right) {
            (n, Some(e))
        } else {
            let Some((lbl_name, lbl_eff)) = agent_model_antigravity::label_parts(right) else {
                continue;
            };
            if lbl_eff.is_none() && !models.iter().any(|m| m["label"].as_str() == Some(right)) {
                continue;
            }
            (lbl_name, lbl_eff.map(str::to_owned))
        };
        if !valid_antigravity_clean_name(&name) {
            continue;
        }
        let found = antigravity_row(models, &name, effort.as_deref());
        if !models.is_empty() && found.is_none() {
            continue;
        }
        return Some(ScreenAgentModel {
            model: found
                .and_then(|f| f["id"].as_str())
                .map_or_else(|| name.clone(), str::to_owned),
            name: Some(found.map_or(name, antigravity_row_name)),
            effort,
        });
    }
    None
}

/// Resolves an Antigravity model ID or label against its grouped catalog list
/// (`antigravityModels.js:antigravityListed`).
#[must_use]
pub fn antigravity_listed(
    models: &[Value],
    id_or_label: &str,
    effort: Option<&str>,
) -> Option<ScreenAgentModel> {
    let value = trim(id_or_label);
    if value.is_empty() {
        return None;
    }
    let eff_non_empty = effort.filter(|e| !e.is_empty());
    if let Some(by_id) = models.iter().find(|m| m["id"].as_str() == Some(value)) {
        let parsed_eff = by_id["label"]
            .as_str()
            .and_then(agent_model_antigravity::label_parts)
            .and_then(|(_, e)| e);
        return Some(ScreenAgentModel {
            model: value.to_owned(),
            name: Some(antigravity_row_name(by_id)),
            effort: eff_non_empty.or(parsed_eff).map(str::to_owned),
        });
    }
    for level in ANTIGRAVITY_EFFORTS {
        if let Some(base_id) = value.strip_suffix(&format!("-{level}"))
            && let Some(base) = models
                .iter()
                .find(|m| m["id"].as_str() == Some(base_id) && m["efforts"].is_array())
        {
            return Some(ScreenAgentModel {
                model: base_id.to_owned(),
                name: base["label"].as_str().map(str::to_owned),
                effort: Some(eff_non_empty.unwrap_or(level).to_owned()),
            });
        }
    }
    let (p_name, p_eff) = agent_model_antigravity::label_parts(value)?;
    let row = antigravity_row(models, &p_name, p_eff.or(eff_non_empty))?;
    let row_eff = row["label"]
        .as_str()
        .and_then(agent_model_antigravity::label_parts)
        .and_then(|(_, e)| e);
    Some(ScreenAgentModel {
        model: row["id"].as_str()?.to_owned(),
        name: Some(antigravity_row_name(row)),
        effort: eff_non_empty.or(p_eff).or(row_eff).map(str::to_owned),
    })
}

fn is_left_model_boundary(c: char) -> bool {
    js_space(c) || matches!(c, '(' | '[' | '|' | ':' | '·' | '•' | ',')
}

fn is_right_model_boundary_at(line: &str, byte_idx: usize) -> bool {
    if byte_idx >= line.len() {
        return true;
    }
    line[byte_idx..]
        .chars()
        .next()
        .is_some_and(|c| js_space(c) || matches!(c, ')' | ']' | '|' | ':' | '·' | '•' | ','))
}

fn is_word_dot_dash(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-')
}

fn match_claude_product_core(line: &str, start: usize) -> Option<usize> {
    // `(?:opus|sonnet|haiku|fable)[ -]?\d+(?:[.-]\d+)?(?:\s?\(1M\))?` (case-insensitive)
    let rest = &line[start..];
    let fam_len = ["opus", "sonnet", "haiku", "fable"]
        .into_iter()
        .find(|fam| strip_prefix_ascii_ci(rest, fam).is_some())?
        .len();
    let bytes = line.as_bytes();
    let mut i = start + fam_len;
    if i < bytes.len() && matches!(bytes[i], b' ' | b'-') {
        i += 1;
    }
    let d_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == d_start {
        return None;
    }
    if i + 1 < bytes.len() && matches!(bytes[i], b'.' | b'-') && bytes[i + 1].is_ascii_digit() {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    // Try consuming optional `\s?\(1M\)` if followed by right boundary.
    let mut j = i;
    if let Some(ch) = line[j..].chars().next()
        && js_space(ch)
    {
        j += ch.len_utf8();
    }
    if line
        .get(j..j + 4)
        .is_some_and(|tag| tag.eq_ignore_ascii_case("(1m)"))
        && is_right_model_boundary_at(line, j + 4)
    {
        return Some(j + 4);
    }
    is_right_model_boundary_at(line, i).then_some(i)
}

fn matches_token_model_core(tok: &str) -> bool {
    let lower = tok.to_ascii_lowercase();
    let b = lower.as_bytes();
    if lower == "big-pickle"
        || matches!(
            lower.as_str(),
            "o1-mini" | "o1-pro" | "o3-mini" | "o3-pro" | "o4-mini" | "o4-pro"
        )
    {
        return true;
    }
    if let Some(rest) = lower.strip_prefix("mai-code-") {
        return !rest.is_empty();
    }
    for prefix in [
        "claude-",
        "kimi-",
        "deepseek-",
        "mistral-",
        "devstral-",
        "codestral-",
        "magistral-",
    ] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            return rest
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric);
        }
    }
    for prefix in ["gpt-", "gemini-", "moonshot-v", "grok-", "glm-"] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            return rest.as_bytes().first().is_some_and(u8::is_ascii_digit);
        }
    }
    if let Some(rest) = lower.strip_prefix("muse-") {
        return rest.as_bytes().first().is_some_and(u8::is_ascii_lowercase);
    }
    if let Some(rest) = lower.strip_prefix("qwen-") {
        return ["coder", "max", "plus", "turbo"]
            .iter()
            .any(|p| rest.starts_with(p));
    }
    if b.len() >= 5 && lower.starts_with("qwen") && b[4].is_ascii_digit() {
        return true;
    }
    if let Some(rest) = lower.strip_prefix("llama-") {
        return rest.as_bytes().first().is_some_and(u8::is_ascii_digit);
    }
    if b.len() >= 6 && lower.starts_with("llama") && b[5].is_ascii_digit() {
        return true;
    }
    false
}

fn match_model_core_at(line: &str, core_pos: usize) -> Option<usize> {
    if let Some(end) = match_claude_product_core(line, core_pos) {
        return Some(end);
    }
    let bytes = line.as_bytes();
    let mut end = core_pos;
    while end < bytes.len() && is_word_dot_dash(bytes[end]) {
        end += 1;
    }
    if end == core_pos || !is_right_model_boundary_at(line, end) {
        return None;
    }
    matches_token_model_core(&line[core_pos..end]).then_some(end)
}

fn match_model_at(line: &str, pos: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    if pos < bytes.len() && bytes[pos].is_ascii_alphanumeric() {
        let mut p = pos + 1;
        while p < bytes.len() && is_word_dot_dash(bytes[p]) {
            p += 1;
        }
        if p < bytes.len()
            && bytes[p] == b'/'
            && let Some(end) = match_model_core_at(line, p + 1)
        {
            return Some(end);
        }
    }
    match_model_core_at(line, pos)
}

/// Extracts a known provider/model token from a single status/banner line (`screenModel.js:modelInLine`).
#[must_use]
pub fn model_in_line(line: &str) -> Option<String> {
    if !within_screen_line_cap(line) {
        return None;
    }
    let mut prev_ok = true;
    for (idx, ch) in line.char_indices() {
        if prev_ok && let Some(end) = match_model_at(line, idx) {
            let trimmed = line[idx..end].trim_end_matches(['.', '-']);
            return (!trimmed.is_empty()).then(|| trimmed.to_owned());
        }
        prev_ok = is_left_model_boundary(ch);
    }
    None
}

/// Reads an agent's model from its bottom status lines (bottom-up) or top banner
/// lines (top-down) (`screenModel.js:modelFromScreen`).
#[must_use]
pub fn model_from_screen<B: AsRef<str>, T: AsRef<str>>(bottom: &[B], top: &[T]) -> Option<String> {
    for raw in bounded_tail(bottom).iter().rev() {
        let line = raw.as_ref();
        if !trim(line).is_empty()
            && let Some(found) = model_in_line(line)
        {
            return Some(found);
        }
    }
    for raw in bounded_head(top) {
        let line = raw.as_ref();
        if !trim(line).is_empty()
            && let Some(found) = model_in_line(line)
        {
            return Some(found);
        }
    }
    None
}

/// Model & effort parsed from Muse Code's status footer (`screenModel.js:museFooterModel`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MuseFooterModel {
    pub model: String,
    pub effort: String,
}

const MUSE_FOOTER_EFFORTS: [&str; 8] = [
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

fn parse_muse_footer_line(line: &str) -> Option<MuseFooterModel> {
    if !within_screen_line_cap(line) {
        return None;
    }
    let s = line.trim_start_matches(js_space);
    let model_end = s.find(js_space)?;
    let model = &s[..model_end];
    let mb = model.as_bytes();
    if !(1..=120).contains(&mb.len())
        || !mb[0].is_ascii_alphanumeric()
        || !mb
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:/@+-".contains(b))
    {
        return None;
    }
    let rest1 = s[model_end..].trim_start_matches(js_space);
    let after_dot1 = rest1.strip_prefix('·')?;
    let rest2 = after_dot1.trim_start_matches(js_space);
    if rest2.len() == after_dot1.len() {
        return None;
    }
    let eff_end = rest2.find(js_space)?;
    let eff_lower = rest2[..eff_end].to_ascii_lowercase();
    if !MUSE_FOOTER_EFFORTS.contains(&eff_lower.as_str()) {
        return None;
    }
    let rest3 = rest2[eff_end..].trim_start_matches(js_space);
    let after_dot2 = rest3.strip_prefix('·')?;
    let rest4 = after_dot2.trim_start_matches(js_space);
    if rest4.len() == after_dot2.len() || rest4.is_empty() {
        return None;
    }
    Some(MuseFooterModel {
        model: model.to_owned(),
        effort: eff_lower,
    })
}

/// Reads Muse Code's footer (`"muse-wonder-1.0 · medium · C:\\repo · YOLO"`) bottom-up.
#[must_use]
pub fn muse_footer_model<S: AsRef<str>>(lines: &[S]) -> Option<MuseFooterModel> {
    for raw in bounded_tail(lines).iter().rev() {
        if let Some(found) = parse_muse_footer_line(raw.as_ref()) {
            return Some(found);
        }
    }
    None
}

fn find_case_insensitive_ascii(haystack: &str, needle_lower: &str) -> Option<usize> {
    let nlen = needle_lower.len();
    if nlen == 0 || haystack.len() < nlen {
        return None;
    }
    (0..=haystack.len() - nlen).find(|&i| {
        haystack.is_char_boundary(i)
            && haystack.is_char_boundary(i + nlen)
            && haystack[i..i + nlen].eq_ignore_ascii_case(needle_lower)
    })
}

/// Parses the model echoed by Claude Code's `/model` (`"Set model to opus (claude-opus-5-5)"`
/// -> `"claude-opus-5-5"`, `screenModel.js:modelFromSwitchLine`).
#[must_use]
pub fn model_from_switch_line(text: &str) -> Option<String> {
    if text.encode_utf16().take(MAX_SWITCH_LINE_UTF16 + 1).count() > MAX_SWITCH_LINE_UTF16 {
        return None;
    }
    let idx = text.find("Set model to")?;
    let after = &text[idx + "Set model to".len()..];
    let trimmed_after = after.trim_start_matches(js_space);
    if trimmed_after.len() == after.len() || trim(trimmed_after).is_empty() {
        return None;
    }
    let no_backticks = trimmed_after.trim_end_matches(js_space).replace('`', " ");
    let without_saved = if let Some(pos) =
        find_case_insensitive_ascii(&no_backticks, "and saved as your default")
    {
        let before = &no_backticks[..pos];
        let before_trimmed = before.trim_end_matches(js_space);
        if before_trimmed.len() < before.len() {
            before_trimmed
        } else {
            &no_backticks
        }
    } else {
        &no_backticks
    };
    let rest = trim(without_saved);
    if rest.is_empty() {
        return None;
    }
    if let Some(found) = model_in_line(rest) {
        return Some(found);
    }
    let first = rest
        .split(|c: char| js_space(c) || c == '(')
        .next()
        .unwrap_or_default();
    let capped: Vec<u16> = first.encode_utf16().take(60).collect();
    let fallback = String::from_utf16_lossy(&capped);
    (!fallback.is_empty()).then_some(fallback)
}
