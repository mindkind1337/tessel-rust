// Adapted from Tessel src/main/agentModel.js, src/main/agentModelSources.js,
// src/main/museSessions.js, and src/renderer/src/components/TerminalPane.vue.
// MIT Lovecast Inc. 2026; see THIRD_PARTY_NOTICES.md.
//! Pure, bounded, I/O-free parsers and header orchestrators for agent runtime
//! models (`agentModel.js`, `agentModelSources.js`, `museSessions.js`, and
//! `TerminalPane.vue`'s `refreshModel`).
//!
//! Security & resource invariants:
//! - Zero filesystem, SQLite, process, or network I/O (`&str` / `&[Row]` / `&Value` in,
//!   typed structs out). File verification, symlink confinement, and background
//!   chunked reading live in `codex_sessions`, `pane_model_log`, and `pane_model_worker`.
//! - Oversized tool-output JSONL lines (`> MAX_JSONL_LINE_BYTES`) inside a valid
//!   `<= MAX_STREAM_TEXT_BYTES` chunk are skipped in O(1) without rejecting the
//!   surrounding chunk or allocating for the oversized line, so a recent model
//!   record sharing a chunk with a multi-megabyte tool result is never lost.
//! - Every JSON candidate passes a linear O(n) non-recursive nesting depth check
//!   (`MAX_JSON_DEPTH`) before `serde_json::from_str`.
//! - Command-line strings are wrapped in [`CommandLine`] and redacted in `Debug`
//!   output so credentials passed on custom launch commands never leak to logs.

use crate::pane_session_model::{
    CursorContextOnScreen, ModelSource, MuseFooterModel, OptimisticModelChoice, PaneModelNode,
    SavedPaneModel, ShownAgentModel, antigravity_listed, antigravity_model_on_screen,
    codex_model_on_screen, cursor_context_on_screen, cursor_model_on_screen, cursor_picker_shown,
    effort_level, model_from_screen, model_from_switch_line, muse_footer_model, pane_model_to_save,
    resolve_chosen_shown, with_model_version,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

/// Maximum byte size accepted for a whole settings/config document (JSON/JSONC/TOML/YAML).
pub const MAX_DOCUMENT_BYTES: usize = 512 * 1024;
/// Maximum byte size accepted for a stream/log slice (`pane_model_log::MAX_SCAN`).
pub const MAX_STREAM_TEXT_BYTES: usize = 64 * 1024 * 1024;
/// Maximum byte size parsed for a single JSONL line; larger tool-output lines are skipped.
pub const MAX_JSONL_LINE_BYTES: usize = 2 * 1024 * 1024;
/// Maximum number of lines inspected in a single JSONL text slice.
pub const MAX_JSONL_LINES: usize = 32_768;
/// Maximum JSON bracket/brace nesting depth allowed before `serde_json` parsing.
pub const MAX_JSON_DEPTH: usize = 64;
/// Maximum UTF-16 code units in a valid agent model identifier (`agentModelSources.js`).
pub const MAX_MODEL_ID_UTF16: usize = 120;
/// Maximum UTF-16 code units in a display name read from settings/catalogs.
pub const MAX_MODEL_NAME_UTF16: usize = 80;
/// Maximum UTF-16 code units inspected from a launch command string.
pub const MAX_COMMAND_UTF16: usize = 8_192;
/// Maximum lines parsed in a TOML configuration document.
pub const MAX_TOML_LINES: usize = 4_096;
/// Maximum sections retained in a parsed TOML configuration document.
pub const MAX_TOML_SECTIONS: usize = 256;
/// Maximum lines parsed in a YAML configuration document.
pub const MAX_YAML_LINES: usize = 4_096;
/// Maximum per-model reasoning effort entries read from Muse `settings.json`.
pub const MAX_MUSE_EFFORT_ENTRIES: usize = 100;
/// Maximum database rows inspected per session query.
pub const MAX_DB_ROWS: usize = 64;
/// Maximum scrollback lines scanned backwards for Claude `/model` switch output.
pub const MAX_SCROLLBACK_SWITCH_LINES: usize = 400;

const CLAUDE_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
const MUSE_EFFORTS: [&str; 8] = [
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];
const GROK_EFFORTS: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
const AMP_MODES: [&str; 5] = ["smart", "rush", "large", "deep", "free"];
const OWN_MODEL_NAMES: [&str; 4] = ["claude", "codex", "cursor", "antigravity"];

const CURSOR_EFFORT_WORDS: [(&str, &str); 7] = [
    ("extra high", "xhigh"),
    ("minimal", "minimal"),
    ("medium", "medium"),
    ("high", "high"),
    ("low", "low"),
    ("max", "max"),
    ("none", "none"),
];

/// Redacted wrapper around a process command line. Never implements `Serialize`
/// or prints raw command arguments in `Debug`.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct CommandLine<'a>(&'a str);

impl<'a> CommandLine<'a> {
    #[must_use]
    pub const fn new(raw: &'a str) -> Self {
        Self(raw)
    }

    #[must_use]
    pub fn bounded_str(self) -> &'a str {
        slice_utf16_prefix(self.0, MAX_COMMAND_UTF16)
    }
}

impl<'a> From<&'a str> for CommandLine<'a> {
    fn from(raw: &'a str) -> Self {
        Self::new(raw)
    }
}

impl fmt::Debug for CommandLine<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CommandLine(<redacted>)")
    }
}

/// Model, optional display name, optional effort tier, and optional epoch-ms timestamp
/// extracted by a pure source parser before provenance (`ModelSource`) is attached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundModel {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<i64>,
}

impl FoundModel {
    #[must_use]
    pub fn into_shown(self, source: ModelSource) -> ShownAgentModel {
        ShownAgentModel {
            model: self.model,
            name: self.name,
            effort: self.effort,
            chosen_effort: None,
            source,
            at: self
                .at
                .and_then(|t| u64::try_from(t).ok())
                .filter(|&t| t > 0),
        }
    }
}

/// Chronological `{ t, model, effort }` transition entry for conversation timelines
/// (`antigravityModelTimeline`, `museModelTimeline`, `piModelTimeline`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTimelinePoint {
    pub t: i64,
    pub model: String,
    pub effort: Option<String>,
}

/// Intermediate Kimi wire scan result (`kimiWireModel`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KimiWirePartial {
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub at: Option<i64>,
}

/// Intermediate Copilot chunk scan state (`copilotRecords` / `copilotMerge` / `copilotFinish`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CopilotScan {
    pub answer: Option<FoundModel>,
    pub change: Option<FoundModel>,
}

/// Parsed Muse `settings.json` model defaults and per-model reasoning efforts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MuseSettingsModel {
    pub model: Option<String>,
    pub effort: Option<String>,
    efforts: BTreeMap<String, String>,
    all_effort: Option<String>,
}

impl MuseSettingsModel {
    #[must_use]
    pub fn effort_for(&self, id: &str) -> Option<&str> {
        if !id.is_empty()
            && let Some(tier) = self.efforts.get(id)
        {
            return Some(tier.as_str());
        }
        self.all_effort.as_deref()
    }
}

/// Parsed OpenCode / Kilo `model.json` last-picked model with file mtime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedModel {
    pub model: String,
    pub effort: Option<String>,
    pub mtime_ms: i64,
}

/// Parsed Cline `providers.json` model with file mtime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClineSettingsModel {
    pub model: String,
    pub effort: Option<String>,
    pub mtime_ms: i64,
}

/// One message row from OpenCode / Kilo / ZCode's `message` SQLite table.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessageRow {
    pub data: String,
    #[serde(default)]
    pub time_created: Option<i64>,
}

impl fmt::Debug for OpenCodeMessageRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenCodeMessageRow")
            .field("data", &"<redacted>")
            .field("time_created", &self.time_created)
            .finish()
    }
}

/// One row from Cline's `sessions` SQLite table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClineSessionRow {
    pub pid: u32,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub is_subagent: bool,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// One model entry from Continue's `config.yaml` `models:` list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinueModelItem {
    pub name: String,
    pub model: String,
    pub chat: bool,
}

/// Parsed Grok `models_cache.json` entry (`grokCatalogEntry`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrokCatalogEntry {
    pub name: Option<String>,
    pub effort: Option<String>,
}

/// Scalar value extracted by [`toml_keys`].
#[derive(Clone, Debug, PartialEq)]
pub enum TomlScalar {
    Str(String),
    Bool(bool),
    Num(f64),
}

impl TomlScalar {
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Num(n) => Some(*n),
            _ => None,
        }
    }
}

/// Section-keyed TOML scalar map (`""` for top-level, `"models\n<alias>"` for dotted sections).
pub type TomlTable = BTreeMap<String, BTreeMap<String, TomlScalar>>;

/// Settings file content paired with its filesystem modification timestamp (ms).
/// Raw file contents are never serialized or printed in `Debug` (settings files
/// such as Cline `providers.json` can hold API keys alongside model names).
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedDocument {
    pub text: String,
    pub mtime_ms: i64,
}

impl fmt::Debug for TimedDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TimedDocument")
            .field("mtime_ms", &self.mtime_ms)
            .field("text", &"<redacted>")
            .finish()
    }
}

/// Pure, pre-fetched inputs for [`resolve_agent_model`].
#[derive(Clone, Default)]
pub struct AgentModelInputs<'a> {
    pub agent_id: &'a str,
    pub session: Option<FoundModel>,
    pub copilot_chunks_newest_first: Vec<String>,
    pub command: CommandLine<'a>,
    pub chosen_model: Option<&'a str>,
    pub launched_at_ms: Option<i64>,
    pub picked: Option<PickedModel>,
    pub claude_settings_docs: Vec<String>,
    pub openclaude_settings_json: Option<String>,
    pub codex_config_toml: Option<String>,
    pub cursor_cli_config_json: Option<String>,
    pub gemini_or_qwen_settings_docs: Vec<String>,
    pub opencode_config_docs: Vec<String>,
    pub kilo_config_docs: Vec<String>,
    pub copilot_settings_docs: Vec<String>,
    pub antigravity_settings_json: Option<String>,
    pub muse_settings_json: Option<String>,
    pub kimi_config_toml: Option<String>,
    pub pi_settings_json: Option<String>,
    pub openclaw_config_json: Option<String>,
    pub droid_settings_json: Option<String>,
    pub cline_providers_doc: Option<TimedDocument>,
    pub continue_selected_model: Option<String>,
    pub continue_config_yaml: Option<String>,
    pub crush_config_docs: Vec<String>,
    pub goose_config_yaml: Option<String>,
    pub aider_config_yamls: Vec<String>,
    pub vibe_config_tomls: Vec<String>,
    pub grok_config_toml: Option<String>,
    pub grok_settings_cache_json: Option<String>,
    pub grok_models_cache_json: Option<String>,
    pub ollama_ps_json: Option<String>,
}

impl fmt::Debug for AgentModelInputs<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentModelInputs")
            .field("agent_id", &self.agent_id)
            .field("session", &self.session)
            .field("command", &self.command)
            .field("chosen_model", &self.chosen_model)
            .field("launched_at_ms", &self.launched_at_ms)
            .field("picked", &self.picked)
            .finish_non_exhaustive()
    }
}

/// Output of [`resolve_agent_model`], matching `agentModel.js` (including Claude's
/// standalone `{ model: null, chosenEffort }` case when only a chosen effort is known).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelResolution {
    pub found: Option<ShownAgentModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen_effort: Option<String>,
}

/// Background worker poll result supplied to [`resolve_pane_header_model`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentModelPollOutcome {
    /// Worker timed out or account root was transiently unavailable (`{ keep: true }`).
    Keep,
    /// Worker completed with a resolution (`Some` or empty).
    Resolved(AgentModelResolution),
}

impl Default for AgentModelPollOutcome {
    fn default() -> Self {
        Self::Resolved(AgentModelResolution::default())
    }
}

/// Pure inputs for [`resolve_pane_header_model`], mirroring `TerminalPane.vue`'s `refreshModel`.
pub struct PaneHeaderModelInputs<'a> {
    pub node: PaneModelNode,
    pub catalog_models: &'a [Value],
    pub current_shown: Option<ShownAgentModel>,
    pub current_session_model: Option<String>,
    pub current_screen_context: Option<CursorContextOnScreen>,
    pub has_term: bool,
    pub screen_lines: &'a [String],
    pub top_screen_lines: &'a [String],
    pub scrollback_lines: &'a [String],
    pub poll: AgentModelPollOutcome,
}

impl fmt::Debug for PaneHeaderModelInputs<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaneHeaderModelInputs")
            .field("agent_id", &self.node.agent_id)
            .field("has_term", &self.has_term)
            .field("current_shown", &self.current_shown)
            .field("current_session_model", &self.current_session_model)
            .field("poll", &self.poll)
            .finish_non_exhaustive()
    }
}

/// Output of [`resolve_pane_header_model`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneHeaderModelResolution {
    /// Model shown in the pane header chip (`agentModel.value`).
    pub shown: Option<ShownAgentModel>,
    /// File-backed session model only (`source != Screen`), safe for Live Share host metadata.
    pub session_model: Option<String>,
    /// Updated Cursor context window state (`screenContext.value`).
    pub screen_context: Option<CursorContextOnScreen>,
    /// Updated optimistic `modelChoice` on the pane node (`None` once cleared/superseded).
    pub model_choice: Option<OptimisticModelChoice>,
    /// Persistence payload for `paneLastModel.js` (`None` when `source == Saved` or empty).
    pub to_save: Option<SavedPaneModel>,
    /// True when Cursor's interactive `/model` picker was open and froze the previous chip.
    pub froze_on_picker: bool,
}

// =============================================================================
// UTF-16 & linear-time JSON safety helpers
// =============================================================================

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

fn slice_utf16_prefix(s: &str, max_units: usize) -> &str {
    let mut used = 0usize;
    for (idx, ch) in s.char_indices() {
        let w = ch.len_utf16();
        if used + w > max_units {
            return &s[..idx];
        }
        used += w;
    }
    s
}

fn trimmed_non_empty(s: &str) -> Option<&str> {
    let t = crate::agent_env::trim(s);
    (!t.is_empty()).then_some(t)
}

fn str_val(v: &Value) -> Option<&str> {
    v.as_str().and_then(trimmed_non_empty)
}

/// Validates an agent model identifier (`<= 120` UTF-16 code units, non-empty after
/// trimming, and free of ASCII control characters `\u{0000}`..=`\u{001f}`).
#[must_use]
pub fn valid_model_id(raw: &str) -> Option<&str> {
    let s = trimmed_non_empty(raw)?;
    if utf16_len(s) > MAX_MODEL_ID_UTF16 || s.chars().any(|c| (c as u32) <= 0x1f) {
        return None;
    }
    Some(s)
}

fn model_id_val(v: &Value) -> Option<String> {
    v.as_str().and_then(valid_model_id).map(str::to_string)
}

/// Linear O(n) non-recursive check ensuring `{` / `[` nesting outside JSON string
/// literals never exceeds `max_depth`.
#[must_use]
pub fn within_json_depth(text: &str, max_depth: usize) -> bool {
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for b in text.bytes() {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > max_depth {
                    return false;
                }
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    true
}

fn parse_bounded_json_line(line: &str) -> Option<Value> {
    if line.len() > MAX_JSONL_LINE_BYTES || !within_json_depth(line, MAX_JSON_DEPTH) {
        return None;
    }
    serde_json::from_str(line).ok()
}

fn parse_bounded_json_doc(text: &str) -> Option<Value> {
    if text.len() > MAX_DOCUMENT_BYTES || !within_json_depth(text, MAX_JSON_DEPTH) {
        return None;
    }
    serde_json::from_str(text).ok()
}

fn strip_jsonc_comments_and_trailing_commas(text: &str) -> String {
    let mut no_comments = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            no_comments.push(c);
            while let Some(sc) = chars.next() {
                no_comments.push(sc);
                if sc == '\\' {
                    if let Some(esc) = chars.next() {
                        no_comments.push(esc);
                    }
                } else if sc == '"' {
                    break;
                }
            }
            continue;
        }
        if c == '/' {
            if chars.peek() == Some(&'/') {
                let _ = chars.next();
                while let Some(&nc) = chars.peek() {
                    if nc == '\n' {
                        break;
                    }
                    let _ = chars.next();
                }
                continue;
            }
            if chars.peek() == Some(&'*') {
                let _ = chars.next();
                while let Some(bc) = chars.next() {
                    if bc == '*' && chars.peek() == Some(&'/') {
                        let _ = chars.next();
                        break;
                    }
                }
                continue;
            }
        }
        no_comments.push(c);
    }

    let mut out = String::with_capacity(no_comments.len());
    let mut p = no_comments.chars().peekable();
    while let Some(c) = p.next() {
        if c == '"' {
            out.push(c);
            while let Some(sc) = p.next() {
                out.push(sc);
                if sc == '\\' {
                    if let Some(esc) = p.next() {
                        out.push(esc);
                    }
                } else if sc == '"' {
                    break;
                }
            }
            continue;
        }
        if c == ',' {
            let mut clone = p.clone();
            let mut drop_comma = false;
            while let Some(&nc) = clone.peek() {
                if crate::agent_env::js_space(nc) {
                    let _ = clone.next();
                } else {
                    drop_comma = nc == '}' || nc == ']';
                    break;
                }
            }
            if drop_comma {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Parses strict JSON or JSONC (line/block comments and trailing commas), bounded by
/// [`MAX_DOCUMENT_BYTES`] and [`MAX_JSON_DEPTH`].
#[must_use]
pub fn parse_loose_json(text: &str) -> Option<Value> {
    if text.is_empty() || text.len() > MAX_DOCUMENT_BYTES {
        return None;
    }
    if within_json_depth(text, MAX_JSON_DEPTH)
        && let Ok(v) = serde_json::from_str::<Value>(text)
    {
        return Some(v);
    }
    let plain = strip_jsonc_comments_and_trailing_commas(text);
    if !within_json_depth(&plain, MAX_JSON_DEPTH) {
        return None;
    }
    serde_json::from_str::<Value>(&plain).ok()
}

fn parse_loose_object(text: &str) -> Option<serde_json::Map<String, Value>> {
    match parse_loose_json(text)? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

fn parse_strict_object(text: &str) -> Option<serde_json::Map<String, Value>> {
    match parse_bounded_json_doc(text)? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// Canonical ECMA-262 array index (`0 <= n < u32::MAX` with no leading zeros or signs).
fn ecma_array_index(key: &str) -> Option<u32> {
    if key.is_empty() || key.len() > 10 {
        return None;
    }
    if key.len() > 1 && key.as_bytes()[0] == b'0' {
        return None;
    }
    if !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    key.parse::<u32>().ok().filter(|&n| n < u32::MAX)
}

/// Local insertion-ordered JSON object matching V8 `JSON.parse` + `Object.entries`
/// without enabling `serde_json/preserve_order` globally across the crate.
#[derive(Clone, Default)]
struct OrderedObject {
    entries: Vec<(String, OrderedJson)>,
    positions: std::collections::BTreeMap<String, usize>,
}

impl OrderedObject {
    fn with_capacity(cap: usize) -> Self {
        Self {
            entries: Vec::with_capacity(cap),
            positions: Default::default(),
        }
    }

    /// Matches `JSON.parse` (`CreateDataProperty`): exact duplicate keys update the
    /// existing property's value in place while keeping the first occurrence's slot.
    fn insert(&mut self, key: String, value: OrderedJson) {
        if let Some(&index) = self.positions.get(&key) {
            self.entries[index].1 = value;
        } else {
            self.positions.insert(key.clone(), self.entries.len());
            self.entries.push((key, value));
        }
    }

    fn get(&self, key: &str) -> Option<&OrderedJson> {
        self.positions.get(key).map(|&index| &self.entries[index].1)
    }

    /// Yields own entries in ECMA-262 `[[OwnPropertyKeys]]` order: canonical array
    /// indices `< u32::MAX` in ascending numeric order, then all remaining string
    /// keys in chronological first-insertion order.
    fn own_entries(&self) -> Vec<(&str, &OrderedJson)> {
        let mut indices: Vec<(u32, &str, &OrderedJson)> = Vec::new();
        let mut strings: Vec<(&str, &OrderedJson)> = Vec::with_capacity(self.entries.len());
        for (k, v) in &self.entries {
            if let Some(idx) = ecma_array_index(k) {
                indices.push((idx, k.as_str(), v));
            } else {
                strings.push((k.as_str(), v));
            }
        }
        indices.sort_by_key(|&(idx, _, _)| idx);
        let mut out = Vec::with_capacity(self.entries.len());
        for (_, k, v) in indices {
            out.push((k, v));
        }
        out.extend(strings);
        out
    }
}

#[derive(Clone)]
enum OrderedJson {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<OrderedJson>),
    Object(OrderedObject),
}

impl OrderedJson {
    fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    fn as_object(&self) -> Option<&OrderedObject> {
        match self {
            Self::Object(o) => Some(o),
            _ => None,
        }
    }

    fn to_scalar_value(&self) -> Option<Value> {
        match self {
            Self::Null => Some(Value::Null),
            Self::Bool(b) => Some(Value::Bool(*b)),
            Self::Number(n) => Some(Value::Number(n.clone())),
            Self::String(s) => Some(Value::String(s.clone())),
            Self::Array(_) | Self::Object(_) => None,
        }
    }

    /// Matches JS `val && typeof val === 'object' ? Object.entries(val) : null`.
    fn js_object_entries(&self) -> Option<Vec<(String, &OrderedJson)>> {
        match self {
            Self::Object(o) => Some(
                o.own_entries()
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            ),
            Self::Array(arr) => Some(
                arr.iter()
                    .enumerate()
                    .map(|(i, v)| (i.to_string(), v))
                    .collect(),
            ),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for OrderedJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct OrderedVisitor;

        impl<'de> serde::de::Visitor<'de> for OrderedVisitor {
            type Value = OrderedJson;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("any valid JSON value")
            }

            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
                Ok(OrderedJson::Bool(v))
            }

            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
                Ok(OrderedJson::Number(v.into()))
            }

            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
                Ok(OrderedJson::Number(v.into()))
            }

            fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E> {
                Ok(serde_json::Number::from_f64(v).map_or(OrderedJson::Null, OrderedJson::Number))
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(OrderedJson::String(v.to_owned()))
            }

            fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
                Ok(OrderedJson::String(v))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(OrderedJson::Null)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(OrderedJson::Null)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                Deserialize::deserialize(deserializer)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut vec = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(1_024));
                while let Some(elem) = seq.next_element()? {
                    vec.push(elem);
                }
                Ok(OrderedJson::Array(vec))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut obj = OrderedObject::with_capacity(map.size_hint().unwrap_or(0).min(1_024));
                while let Some((k, v)) = map.next_entry::<String, OrderedJson>()? {
                    obj.insert(k, v);
                }
                Ok(OrderedJson::Object(obj))
            }
        }

        deserializer.deserialize_any(OrderedVisitor)
    }
}

fn parse_loose_ordered_json(text: &str) -> Option<OrderedJson> {
    if text.is_empty() || text.len() > MAX_DOCUMENT_BYTES {
        return None;
    }
    if within_json_depth(text, MAX_JSON_DEPTH)
        && let Ok(v) = serde_json::from_str::<OrderedJson>(text)
    {
        return Some(v);
    }
    let plain = strip_jsonc_comments_and_trailing_commas(text);
    if !within_json_depth(&plain, MAX_JSON_DEPTH) {
        return None;
    }
    serde_json::from_str::<OrderedJson>(&plain).ok()
}

fn parse_loose_ordered_object(text: &str) -> Option<OrderedObject> {
    match parse_loose_ordered_json(text)? {
        OrderedJson::Object(map) => Some(map),
        _ => None,
    }
}

fn parse_strict_ordered_object(text: &str) -> Option<OrderedObject> {
    if text.len() > MAX_DOCUMENT_BYTES || !within_json_depth(text, MAX_JSON_DEPTH) {
        return None;
    }
    match serde_json::from_str::<OrderedJson>(text).ok()? {
        OrderedJson::Object(map) => Some(map),
        _ => None,
    }
}

/// Parses an ISO-8601 / RFC-3339 date string into epoch milliseconds (`Date.parse`).
#[must_use]
pub fn parse_iso_ms(raw: &str) -> Option<i64> {
    let s = trimmed_non_empty(raw)?;
    if s.len() > 64 {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        return Some(ndt.and_utc().timestamp_millis());
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(ndt.and_utc().timestamp_millis());
    }
    if let Ok(nd) = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        && let Some(ndt) = nd.and_hms_opt(0, 0, 0)
    {
        return Some(ndt.and_utc().timestamp_millis());
    }
    None
}

/// Converts a JSON timestamp value (seconds, milliseconds, or ISO string) to epoch ms
/// (`agentModelSources.js` `timeOf`).
#[must_use]
pub fn time_of_value(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_f64() {
        if n.is_finite() && n > 0.0 {
            let ms = if n < 1e11 { n * 1000.0 } else { n };
            return Some(ms as i64);
        }
        return None;
    }
    v.as_str().and_then(parse_iso_ms)
}

fn num_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f as i64)),
        Value::String(s) => trimmed_non_empty(s)
            .and_then(|t| t.parse::<f64>().ok())
            .filter(|f| f.is_finite())
            .map(|f| f as i64),
        _ => None,
    }
}

/// Converts a Muse `recorded_at` value (microseconds, milliseconds, seconds, numeric
/// string, or ISO timestamp) to epoch ms (`museSessions.js` `museTime`).
#[must_use]
pub fn muse_time_ms(v: &Value) -> i64 {
    fn from_num(n: f64) -> i64 {
        if !n.is_finite() || n <= 0.0 {
            return 0;
        }
        if n >= 1e14 {
            (n / 1000.0).floor() as i64
        } else if n < 1e11 {
            (n * 1000.0) as i64
        } else {
            n as i64
        }
    }
    match v {
        Value::Number(n) => n.as_f64().map_or(0, from_num),
        Value::String(s) => {
            let Some(t) = trimmed_non_empty(s) else {
                return 0;
            };
            if is_unsigned_decimal(t)
                && let Ok(n) = t.parse::<f64>()
            {
                return from_num(n);
            }
            parse_iso_ms(t).unwrap_or(0)
        }
        _ => 0,
    }
}

fn is_unsigned_decimal(s: &str) -> bool {
    let mut parts = s.split('.');
    let Some(int_part) = parts.next() else {
        return false;
    };
    if int_part.is_empty() || !int_part.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    match (parts.next(), parts.next()) {
        (None, None) => true,
        (Some(frac), None) => !frac.is_empty() && frac.bytes().all(|b| b.is_ascii_digit()),
        _ => false,
    }
}

/// Normalizes a directory path for case-insensitive workspace comparison (`fileRead.js` `normDir`).
#[must_use]
pub fn norm_dir(path: &str) -> String {
    let replaced = path.replace('/', "\\");
    let trimmed = replaced.trim_end_matches('\\');
    trimmed.to_lowercase()
}

/// Extracts the creation timestamp in epoch ms from a UUIDv7 identifier (`uuidV7Time` / `museSessionStart`).
#[must_use]
pub fn uuid_v7_time_ms(id: &str) -> Option<i64> {
    if id.len() != 36 {
        return None;
    }
    let bytes = id.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        let expect_dash = matches!(i, 8 | 13 | 18 | 23);
        if expect_dash {
            if b != b'-' {
                return None;
            }
        } else if !b.is_ascii_hexdigit() {
            return None;
        }
    }
    if bytes[14] != b'7' {
        return None;
    }
    let mut hex12 = String::with_capacity(12);
    hex12.push_str(&id[0..8]);
    hex12.push_str(&id[9..13]);
    let ms = i64::from_str_radix(&hex12, 16).ok()?;
    (ms > 0).then_some(ms)
}

// =============================================================================
// 1. Claude Code / OpenClaude / Qoder transcript parsers
// =============================================================================

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_claude_effort(s: &str) -> bool {
    CLAUDE_EFFORTS.contains(&s)
}

fn is_claude_model_token(s: &str) -> bool {
    !s.is_empty()
        && utf16_len(s) <= 80
        && s.bytes()
            .all(|b| is_word_byte(b) || matches!(b, b'.' | b'[' | b']' | b'-'))
}

#[derive(Clone, Debug)]
enum ClaudeCmdResult {
    Model { family: Option<String> },
    Effort,
}

#[derive(Clone, Debug, Default)]
struct ClaudeCmdParsed {
    result: Option<ClaudeCmdResult>,
    model: Option<String>,
    effort: Option<String>,
    at: Option<i64>,
}

#[derive(Clone, Debug, Default)]
struct ClaudePicked {
    model: Option<String>,
    effort: Option<String>,
    at: Option<i64>,
}

fn claude_family_prefix(s: &str) -> Option<String> {
    let t = crate::agent_env::trim(s);
    for fam in ["opus", "sonnet", "haiku", "fable"] {
        if t.len() >= fam.len()
            && t.is_char_boundary(fam.len())
            && t[..fam.len()].eq_ignore_ascii_case(fam)
            && t.as_bytes()
                .get(fam.len())
                .is_none_or(|&b| !is_word_byte(b))
        {
            return Some(t[..fam.len()].to_string());
        }
    }
    None
}

fn extract_tag<'a>(body: &'a str, open: &str, close: &str, max_utf16: usize) -> Option<&'a str> {
    let start = body.find(open)? + open.len();
    let rest = &body[start..];
    let end = rest.find(close)?;
    let inner = &rest[..end];
    if inner.contains('<') || utf16_len(inner) > max_utf16 {
        return None;
    }
    Some(inner)
}

fn claude_command_line(o: &Value, after: Option<&ClaudeCmdResult>) -> ClaudeCmdParsed {
    let body: String = match o.get("message").and_then(|m| m.get("content")) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|b| {
                if b.get("type").and_then(Value::as_str) == Some("text") {
                    b.get("text").and_then(Value::as_str).unwrap_or("")
                } else {
                    ""
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    let at = o
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_iso_ms);

    if let Some(pos) = body.find("<local-command-stdout>") {
        let tail = slice_utf16_prefix(&body[pos + "<local-command-stdout>".len()..], 400);
        let trimmed_start = tail.trim_start_matches(crate::agent_env::js_space);
        if let Some(after_set) = trimmed_start.strip_prefix("Set model to")
            && after_set.starts_with(crate::agent_env::js_space)
        {
            let mut arg = after_set.trim_start_matches(crate::agent_env::js_space);
            if let Some(stripped) = arg.strip_prefix('`') {
                arg = stripped;
            }
            let end = arg.find(['`', '\n']).unwrap_or(arg.len());
            let captured = slice_utf16_prefix(&arg[..end], 80);
            if !captured.is_empty() {
                return ClaudeCmdParsed {
                    result: Some(ClaudeCmdResult::Model {
                        family: claude_family_prefix(captured),
                    }),
                    ..Default::default()
                };
            }
        }
        if let Some(after_eff) = trimmed_start.strip_prefix("Set effort level to")
            && after_eff.starts_with(crate::agent_env::js_space)
        {
            let rest = after_eff.trim_start_matches(crate::agent_env::js_space);
            for tier in CLAUDE_EFFORTS {
                if rest.len() >= tier.len()
                    && rest.is_char_boundary(tier.len())
                    && rest[..tier.len()].eq_ignore_ascii_case(tier)
                    && rest
                        .as_bytes()
                        .get(tier.len())
                        .is_none_or(|&b| !is_word_byte(b))
                {
                    return ClaudeCmdParsed {
                        result: Some(ClaudeCmdResult::Effort),
                        model: None,
                        effort: Some(tier.to_string()),
                        at,
                    };
                }
            }
        }
        return ClaudeCmdParsed::default();
    }

    let name = extract_tag(&body, "<command-name>", "</command-name>", 41)
        .map(|s| s.strip_prefix('/').unwrap_or(s))
        .filter(|s| {
            !s.is_empty()
                && utf16_len(s) <= 40
                && s.bytes()
                    .all(|b| is_word_byte(b) || matches!(b, b':' | b'.' | b'-'))
        });
    let args = extract_tag(&body, "<command-args>", "</command-args>", 100)
        .map(crate::agent_env::trim)
        .unwrap_or("");

    if name == Some("model")
        && let Some(ClaudeCmdResult::Model { family }) = after
    {
        let model = if is_claude_model_token(args) {
            Some(args.to_string())
        } else {
            family.as_ref().map(|f| f.to_ascii_lowercase())
        };
        return ClaudeCmdParsed {
            result: None,
            model,
            effort: None,
            at,
        };
    }
    if name == Some("effort") && matches!(after, Some(ClaudeCmdResult::Effort)) {
        let lower = args.to_ascii_lowercase();
        if is_claude_effort(&lower) {
            return ClaudeCmdParsed {
                result: None,
                model: None,
                effort: Some(lower),
                at,
            };
        }
    }
    ClaudeCmdParsed::default()
}

fn picked_claude_turn(
    picked: ClaudePicked,
    base_model: Option<&str>,
    base_effort: Option<&str>,
) -> Option<FoundModel> {
    let next = picked
        .model
        .clone()
        .or_else(|| base_model.map(str::to_string))?;
    let kept = picked.effort.or_else(|| {
        if picked
            .model
            .as_deref()
            .is_some_and(|m| m.to_ascii_lowercase().contains("haiku"))
        {
            None
        } else {
            base_effort.map(str::to_string)
        }
    });
    Some(FoundModel {
        model: next,
        name: None,
        effort: kept,
        at: picked.at,
    })
}

/// Extracts `{ model, effort, at }` of the latest turn or `/model` / `/effort` slash
/// command from a Claude Code / OpenClaude / Qoder JSONL transcript slice.
#[must_use]
pub fn claude_turn_from_text(text: &str) -> Option<FoundModel> {
    if text.is_empty() || text.len() > MAX_STREAM_TEXT_BYTES {
        return None;
    }
    let mut result: Option<ClaudeCmdResult> = None;
    let mut picked = ClaudePicked::default();
    let mut has_picked = false;

    for raw_line in text.split('\n').rev().take(MAX_JSONL_LINES) {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.len() > MAX_JSONL_LINE_BYTES {
            result = None;
            continue;
        }
        let is_command = line.contains("<local-command-stdout>") || line.contains("<command-name>");
        if !line.contains("\"model\"") && !is_command {
            if !crate::agent_env::trim(line).is_empty() {
                result = None;
            }
            continue;
        }
        let Some(o) = parse_bounded_json_line(line) else {
            continue;
        };
        if is_command && o.get("type").and_then(Value::as_str) == Some("user") {
            let cmd = claude_command_line(&o, result.as_ref());
            result = cmd.result;
            if let Some(m) = cmd.model
                && picked.model.is_none()
            {
                picked.model = Some(m);
                if picked.at.is_none() {
                    picked.at = cmd.at;
                }
                has_picked = true;
            }
            if let Some(e) = cmd.effort
                && picked.effort.is_none()
            {
                picked.effort = Some(e);
                if picked.at.is_none() {
                    picked.at = cmd.at;
                }
                has_picked = true;
            }
            continue;
        }
        result = None;
        if o.get("type").and_then(Value::as_str) != Some("assistant")
            || o.get("isSidechain") == Some(&Value::Bool(true))
        {
            continue;
        }
        let Some(m) = o
            .get("message")
            .and_then(|msg| msg.get("model"))
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty() && !m.starts_with('<'))
            .and_then(valid_model_id)
        else {
            continue;
        };
        let eff = [o.get("perTurnEffort"), o.get("effort")]
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .find(|e| is_claude_effort(e));
        let at = o
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso_ms);
        if has_picked {
            return picked_claude_turn(picked, Some(m), eff);
        }
        return Some(FoundModel {
            model: m.to_string(),
            name: None,
            effort: eff.map(str::to_string),
            at,
        });
    }
    if has_picked {
        picked_claude_turn(picked, None, None)
    } else {
        None
    }
}

/// Extracts only the model identifier from a Claude transcript slice (`claudeModelFromText`).
#[must_use]
pub fn claude_model_from_text(text: &str) -> Option<String> {
    claude_turn_from_text(text).map(|t| t.model)
}

fn strip_1m_suffix(s: &str) -> &str {
    if s.len() >= 4
        && s.is_char_boundary(s.len() - 4)
        && s[s.len() - 4..].eq_ignore_ascii_case("[1m]")
    {
        &s[..s.len() - 4]
    } else {
        s
    }
}

/// Compares a Claude model full ID and alias (`"claude-fable-5-1"` vs `"fable"`), ignoring `[1m]`.
#[must_use]
pub fn same_claude_model(a: &str, b: &str) -> bool {
    let x = strip_1m_suffix(a).to_ascii_lowercase();
    let y = strip_1m_suffix(b).to_ascii_lowercase();
    if x.is_empty() || y.is_empty() {
        return false;
    }
    let is_alias = |full: &str, short: &str| -> bool {
        !short.is_empty()
            && short.bytes().all(|c| c.is_ascii_lowercase())
            && full.contains(&format!("-{short}-"))
    };
    x == y || is_alias(&x, &y) || is_alias(&y, &x)
}

// =============================================================================
// 2. Codex rollout & Copilot event stream parsers
// =============================================================================

/// Extracts `{ model, effort, at }` of the latest turn or `thread_settings_applied`
/// event in a Codex rollout JSONL slice (`codexModelFromText`).
#[must_use]
pub fn codex_model_from_text(text: &str) -> Option<FoundModel> {
    if text.is_empty() || text.len() > MAX_STREAM_TEXT_BYTES {
        return None;
    }
    for raw_line in text.split('\n').rev().take(MAX_JSONL_LINES) {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.len() > MAX_JSONL_LINE_BYTES || !line.contains("\"model\"") {
            continue;
        }
        let Some(o) = parse_bounded_json_line(line) else {
            continue;
        };
        let Some(p) = o.get("payload").and_then(Value::as_object) else {
            continue;
        };
        let at = o
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso_ms);
        if o.get("type").and_then(Value::as_str) == Some("turn_context")
            && let Some(model) = p
                .get("model")
                .and_then(Value::as_str)
                .filter(|m| !m.is_empty())
                .and_then(valid_model_id)
        {
            let mode_eff = p
                .get("collaboration_mode")
                .and_then(|cm| cm.get("settings"))
                .and_then(|st| st.get("reasoning_effort"));
            let effort = p
                .get("effort")
                .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                .or_else(|| {
                    p.get("reasoning_effort")
                        .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                })
                .or_else(|| mode_eff.filter(|v| v.as_str().is_some_and(|s| !s.is_empty())))
                .and_then(Value::as_str)
                .map(str::to_string);
            return Some(FoundModel {
                model: model.to_string(),
                name: None,
                effort,
                at,
            });
        }
        if p.get("type").and_then(Value::as_str) == Some("thread_settings_applied")
            && let Some(ts) = p.get("thread_settings").and_then(Value::as_object)
            && let Some(model) = ts
                .get("model")
                .and_then(Value::as_str)
                .filter(|m| !m.is_empty())
                .and_then(valid_model_id)
        {
            let effort = ts
                .get("reasoning_effort")
                .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                .or_else(|| {
                    ts.get("effort")
                        .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                })
                .and_then(Value::as_str)
                .map(str::to_string);
            return Some(FoundModel {
                model: model.to_string(),
                name: None,
                effort,
                at,
            });
        }
    }
    None
}

/// Scans a single Copilot `events.jsonl` chunk from newest to oldest (`copilotRecords`).
#[must_use]
pub fn copilot_records(text: &str) -> Option<CopilotScan> {
    if text.is_empty() || text.len() > MAX_STREAM_TEXT_BYTES {
        return None;
    }
    let mut answer: Option<FoundModel> = None;
    for raw_line in text.split('\n').rev().take(MAX_JSONL_LINES) {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.len() > MAX_JSONL_LINE_BYTES
            || !(line.contains("\"model\"") || line.contains("\"session.model_change\""))
            || !line.contains("\"type\":\"")
        {
            continue;
        }
        let Some(o) = parse_bounded_json_line(line) else {
            continue;
        };
        let Some(d) = o.get("data").and_then(Value::as_object) else {
            continue;
        };
        let at = o
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso_ms);
        match o.get("type").and_then(Value::as_str) {
            Some("assistant.message") => {
                if answer.is_none()
                    && let Some(m) = d
                        .get("model")
                        .and_then(Value::as_str)
                        .filter(|m| !m.is_empty())
                        .and_then(valid_model_id)
                {
                    answer = Some(FoundModel {
                        model: m.to_string(),
                        name: None,
                        effort: None,
                        at,
                    });
                }
            }
            Some("session.model_change") => {
                if let Some(m) = d
                    .get("newModel")
                    .and_then(Value::as_str)
                    .filter(|m| !m.is_empty())
                    .and_then(valid_model_id)
                {
                    let eff = d
                        .get("reasoningEffort")
                        .and_then(Value::as_str)
                        .and_then(effort_level)
                        .map(str::to_string);
                    return Some(CopilotScan {
                        answer,
                        change: Some(FoundModel {
                            model: m.to_string(),
                            name: None,
                            effort: eff,
                            at,
                        }),
                    });
                }
            }
            _ => {}
        }
    }
    answer.map(|a| CopilotScan {
        answer: Some(a),
        change: None,
    })
}

/// Merges a newer and older Copilot chunk scan (`copilotMerge`).
#[must_use]
pub fn copilot_merge(
    newer: Option<CopilotScan>,
    older: Option<CopilotScan>,
) -> Option<CopilotScan> {
    match (newer, older) {
        (None, o) => o,
        (Some(n), None) => Some(n),
        (Some(n), Some(o)) => {
            if n.change.is_some() {
                Some(n)
            } else {
                Some(CopilotScan {
                    answer: n.answer.or(o.answer),
                    change: o.change,
                })
            }
        }
    }
}

/// True once a `session.model_change` record has been observed (`done` predicate for `latestInFile`).
#[must_use]
pub const fn copilot_is_done(scan: &CopilotScan) -> bool {
    scan.change.is_some()
}

/// Produces the final Copilot `FoundModel` from accumulated chunk state (`copilotFinish`).
#[must_use]
pub fn copilot_finish(scan: Option<CopilotScan>) -> Option<FoundModel> {
    let s = scan?;
    if let Some(ans) = s.answer {
        return Some(FoundModel {
            model: ans.model,
            name: None,
            effort: s.change.and_then(|c| c.effort),
            at: ans.at,
        });
    }
    s.change
}

/// Extracts the latest Copilot model and reasoning effort from a single `events.jsonl` slice.
#[must_use]
pub fn copilot_model_from_text(text: &str) -> Option<FoundModel> {
    copilot_finish(copilot_records(text))
}

// =============================================================================
// 3. Antigravity CLI & Muse Code conversation parsers
// =============================================================================

fn strip_tag_blocks(text: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(open) {
        out.push_str(&rest[..pos]);
        let after_open = &rest[pos + open.len()..];
        if let Some(end) = after_open.find(close) {
            rest = &after_open[end + close.len()..];
        } else {
            return out;
        }
    }
    out.push_str(rest);
    out
}

fn collect_tag_bodies<'a>(text: &'a str, open: &str, close: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find(open) {
        let after_open = &rest[pos + open.len()..];
        let Some(end) = after_open.find(close) else {
            break;
        };
        out.push(&after_open[..end]);
        rest = &after_open[end + close.len()..];
    }
    out
}

fn collapse_js_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for ch in crate::agent_env::trim(s).chars() {
        if crate::agent_env::js_space(ch) {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            out.push(ch);
            in_ws = false;
        }
    }
    out
}

fn parse_antigravity_setting_change(change_body: &str) -> Option<String> {
    let collapsed = collapse_js_whitespace(change_body);
    let marker = "setting `Model Selection` from ";
    let start = collapsed.find(marker)? + marker.len();
    let after_from = &collapsed[start..];
    let mut to_pos: Option<usize> = None;
    for (idx, _) in after_from.match_indices(" to ") {
        let from_len = utf16_len(&after_from[..idx]);
        if (1..=120).contains(&from_len) {
            to_pos = Some(idx);
            break;
        }
    }
    let idx = to_pos?;
    let after_to = &after_from[idx + " to ".len()..];
    for (dot_idx, _) in after_to.match_indices('.') {
        let next_ok = after_to
            .as_bytes()
            .get(dot_idx + 1)
            .is_none_or(|&b| b == b' ');
        if !next_ok {
            continue;
        }
        let candidate = &after_to[..dot_idx];
        let cand_len = utf16_len(candidate);
        if (1..=120).contains(&cand_len) {
            return Some(crate::agent_env::trim(candidate).to_string());
        }
    }
    None
}

/// Extracts `{ model, name, effort, at }` from an Antigravity `transcript.jsonl` slice,
/// ignoring `<USER_REQUEST>` blocks so prompt text cannot spoof `<USER_SETTINGS_CHANGE>`.
#[must_use]
pub fn antigravity_model_from_text(text: &str) -> Option<FoundModel> {
    if text.is_empty() || text.len() > MAX_STREAM_TEXT_BYTES {
        return None;
    }
    for raw_line in text.split('\n').rev().take(MAX_JSONL_LINES) {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.len() > MAX_JSONL_LINE_BYTES || !line.contains("Model Selection") {
            continue;
        }
        let Some(o) = parse_bounded_json_line(line) else {
            continue;
        };
        if o.get("type").and_then(Value::as_str) != Some("USER_INPUT") {
            continue;
        }
        let Some(content) = o.get("content").and_then(Value::as_str) else {
            continue;
        };
        let notes = strip_tag_blocks(content, "<USER_REQUEST>", "</USER_REQUEST>");
        let changes =
            collect_tag_bodies(&notes, "<USER_SETTINGS_CHANGE>", "</USER_SETTINGS_CHANGE>");
        for body in changes.into_iter().rev() {
            let Some(label) = parse_antigravity_setting_change(body) else {
                continue;
            };
            let Some((name, effort)) = crate::agent_model_antigravity::label_parts(&label) else {
                continue;
            };
            let at = o
                .get("created_at")
                .and_then(Value::as_str)
                .and_then(parse_iso_ms);
            return Some(FoundModel {
                model: label,
                name: Some(name),
                effort: effort.map(str::to_string),
                at,
            });
        }
    }
    None
}

/// Chronological Antigravity model timeline (`antigravityModelTimeline`).
#[must_use]
pub fn antigravity_model_timeline<S: AsRef<str>>(lines: &[S]) -> Vec<ModelTimelinePoint> {
    let mut out = Vec::new();
    for line in lines.iter().take(MAX_JSONL_LINES) {
        let s = line.as_ref();
        if !s.contains("Model Selection") {
            continue;
        }
        if let Some(m) = antigravity_model_from_text(s)
            && let (Some(t), Some(name)) = (m.at, m.name)
        {
            out.push(ModelTimelinePoint {
                t,
                model: name,
                effort: m.effort,
            });
        }
    }
    out
}

fn valid_muse_model_id(raw: &str) -> Option<&str> {
    let t = trimmed_non_empty(raw)?;
    if utf16_len(t) > MAX_MODEL_ID_UTF16 {
        return None;
    }
    let ok = t.bytes().all(|b| {
        is_word_byte(b) || matches!(b, b'.' | b':' | b'/' | b'@' | b'[' | b']' | b'+' | b'-')
    });
    ok.then_some(t)
}

fn valid_muse_effort(raw: &str) -> Option<String> {
    let t = trimmed_non_empty(raw)?.to_ascii_lowercase();
    MUSE_EFFORTS.contains(&t.as_str()).then_some(t)
}

fn muse_effort_of_record(rec: &serde_json::Map<String, Value>) -> Option<String> {
    rec.get("reasoning_effort")
        .and_then(Value::as_str)
        .and_then(valid_muse_effort)
        .or_else(|| {
            rec.get("reasoningEffort")
                .and_then(Value::as_str)
                .and_then(valid_muse_effort)
        })
        .or_else(|| {
            rec.get("effort")
                .and_then(Value::as_str)
                .and_then(valid_muse_effort)
        })
}

struct MuseRecord {
    kind: String,
    at: i64,
    payload: serde_json::Map<String, Value>,
}

enum MuseFact {
    Configured {
        model: Option<String>,
        effort: Option<String>,
    },
    Used(Option<String>),
}

fn unwrap_muse_records(line_val: Value) -> Vec<MuseRecord> {
    let Value::Object(mut obj) = line_val else {
        return Vec::new();
    };
    let raw_items: Vec<Value> = if let Some(Value::Array(children)) = obj.remove("children") {
        children
            .into_iter()
            .take(256)
            .filter_map(|c| {
                let Value::Object(mut child_obj) = c else {
                    return None;
                };
                match child_obj.remove("record_json")? {
                    Value::String(s) => parse_bounded_json_line(&s),
                    other @ Value::Object(_) => Some(other),
                    _ => None,
                }
            })
            .collect()
    } else {
        vec![Value::Object(obj)]
    };

    raw_items
        .into_iter()
        .filter_map(|item| {
            let Value::Object(mut rec) = item else {
                return None;
            };
            let kind = rec
                .get("payload_type")
                .and_then(str_val)
                .unwrap_or("")
                .to_string();
            let at = rec.get("recorded_at").map_or(0, muse_time_ms);
            let payload = match rec.remove("payload") {
                Some(Value::Object(p)) => p,
                _ => serde_json::Map::new(),
            };
            Some(MuseRecord { kind, at, payload })
        })
        .collect()
}

fn muse_model_fact(rec: &MuseRecord) -> Option<MuseFact> {
    let record_obj = rec.payload.get("record").and_then(Value::as_object);
    let eff_obj = record_obj
        .and_then(|r| r.get("effective"))
        .and_then(Value::as_object);

    match rec.kind.as_str() {
        "runtime.session.metadata" | "run.model.configured" => {
            let model = record_obj
                .and_then(|r| r.get("model_id"))
                .and_then(Value::as_str)
                .and_then(valid_muse_model_id)
                .map(str::to_string);
            let effort = record_obj.and_then(muse_effort_of_record);
            Some(MuseFact::Configured { model, effort })
        }
        "runtime.model_reconfigure.completed" => {
            let model = eff_obj
                .and_then(|e| e.get("model_id"))
                .and_then(Value::as_str)
                .and_then(valid_muse_model_id)
                .or_else(|| {
                    record_obj
                        .and_then(|r| r.get("model_id"))
                        .and_then(Value::as_str)
                        .and_then(valid_muse_model_id)
                })
                .map(str::to_string);
            let effort = eff_obj
                .and_then(muse_effort_of_record)
                .or_else(|| record_obj.and_then(muse_effort_of_record));
            Some(MuseFact::Configured { model, effort })
        }
        "runtime.reasoning_effort_reconfigure"
        | "runtime.reasoning_effort_reconfigure.completed" => {
            let effort = eff_obj
                .and_then(muse_effort_of_record)
                .or_else(|| record_obj.and_then(muse_effort_of_record));
            Some(MuseFact::Configured {
                model: None,
                effort,
            })
        }
        "runtime.session" => {
            let ev = rec.payload.get("event").and_then(Value::as_object)?;
            if ev.get("kind").and_then(Value::as_str) == Some("model_completed") {
                let used = ev
                    .get("model")
                    .and_then(Value::as_str)
                    .and_then(valid_muse_model_id)
                    .map(str::to_string);
                Some(MuseFact::Used(used))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn muse_text_records(text: &str) -> Vec<MuseRecord> {
    if text.is_empty() || text.len() > MAX_STREAM_TEXT_BYTES {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut inspected = 0usize;
    for raw in text.split('\n') {
        let raw_line = raw.strip_suffix('\r').unwrap_or(raw);
        if raw_line.len() > MAX_JSONL_LINE_BYTES {
            continue;
        }
        let line = crate::agent_env::trim(raw_line);
        if !line.starts_with('{') || !(line.contains("model") || line.contains("effort")) {
            continue;
        }
        inspected += 1;
        if inspected > MAX_JSONL_LINES {
            break;
        }
        let Some(o) = parse_bounded_json_line(line) else {
            continue;
        };
        out.extend(unwrap_muse_records(o));
    }
    out
}

/// Extracts `{ model, effort, at }` in force at the end of a Muse `session.jsonl` slice.
#[must_use]
pub fn muse_model_from_text(text: &str) -> Option<FoundModel> {
    let mut model: Option<String> = None;
    let mut effort: Option<String> = None;
    let mut used: Option<String> = None;
    let mut at: i64 = 0;

    for rec in muse_text_records(text) {
        let Some(fact) = muse_model_fact(&rec) else {
            continue;
        };
        match fact {
            MuseFact::Used(u) => {
                if let Some(u) = u {
                    used = Some(u);
                }
            }
            MuseFact::Configured {
                model: fm,
                effort: fe,
            } => {
                let changed = fm.is_some() || fe.is_some();
                if let Some(m) = fm {
                    model = Some(m);
                }
                if let Some(e) = fe {
                    effort = Some(e);
                }
                if changed && rec.at != 0 {
                    at = rec.at;
                }
            }
        }
    }

    let final_model = model.or(used)?;
    Some(FoundModel {
        model: final_model,
        name: None,
        effort,
        at: (at != 0).then_some(at),
    })
}

/// Merges a Muse session log's tail and head slices (`museSession` in `agentModel.js`).
#[must_use]
pub fn muse_model_from_log(tail_text: &str, head_text: &str) -> Option<FoundModel> {
    let tail = muse_model_from_text(tail_text);
    if tail.as_ref().is_some_and(|t| t.effort.is_some()) {
        return tail;
    }
    let head = muse_model_from_text(head_text);
    match (tail, head) {
        (None, h) => h,
        (Some(mut t), Some(h)) => {
            if h.effort.is_some() {
                t.effort = h.effort;
            }
            Some(t)
        }
        (Some(t), None) => Some(t),
    }
}

/// Chronological Muse model and effort timeline (`museModelTimeline`).
#[must_use]
pub fn muse_model_timeline(text: &str) -> Vec<ModelTimelinePoint> {
    let mut out = Vec::new();
    let mut model = String::new();
    let mut effort: Option<String> = None;

    for rec in muse_text_records(text) {
        if rec.at == 0 {
            continue;
        }
        let Some(MuseFact::Configured {
            model: fm,
            effort: fe,
        }) = muse_model_fact(&rec)
        else {
            continue;
        };
        let next_model = fm.unwrap_or_else(|| model.clone());
        let next_effort = fe.or_else(|| effort.clone());
        if next_model.is_empty() || (next_model == model && next_effort == effort) {
            continue;
        }
        model = next_model.clone();
        effort.clone_from(&next_effort);
        out.push(ModelTimelinePoint {
            t: rec.at,
            model: next_model,
            effort: next_effort,
        });
    }
    out
}

// =============================================================================
// 4. Gemini, Qwen, Kimi, Pi/OMP/OpenClaw, OpenCode/Kilo/ZCode, Cline, Grok
// =============================================================================

/// Extracts the latest Gemini CLI answer model (`geminiModelFromText`).
#[must_use]
pub fn gemini_model_from_text(text: &str) -> Option<FoundModel> {
    if text.is_empty() || text.len() > MAX_STREAM_TEXT_BYTES {
        return None;
    }
    for raw_line in text.split('\n').rev().take(MAX_JSONL_LINES) {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.len() > MAX_JSONL_LINE_BYTES
            || !(line.contains("\"gemini\"") && line.contains("\"model\""))
        {
            continue;
        }
        let Some(o) = parse_bounded_json_line(line) else {
            continue;
        };
        if o.get("type").and_then(Value::as_str) != Some("gemini") {
            continue;
        }
        if let Some(model) = o.get("model").and_then(model_id_val) {
            return Some(FoundModel {
                model,
                name: None,
                effort: None,
                at: o.get("timestamp").and_then(time_of_value),
            });
        }
    }
    None
}

/// Extracts the latest Qwen Code assistant model (`qwenModelFromText`).
#[must_use]
pub fn qwen_model_from_text(text: &str) -> Option<FoundModel> {
    if text.is_empty() || text.len() > MAX_STREAM_TEXT_BYTES {
        return None;
    }
    for raw_line in text.split('\n').rev().take(MAX_JSONL_LINES) {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.len() > MAX_JSONL_LINE_BYTES
            || !(line.contains("\"assistant\"") && line.contains("\"model\""))
        {
            continue;
        }
        let Some(o) = parse_bounded_json_line(line) else {
            continue;
        };
        if o.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if let Some(model) = o.get("model").and_then(model_id_val) {
            return Some(FoundModel {
                model,
                name: None,
                effort: None,
                at: o.get("timestamp").and_then(time_of_value),
            });
        }
    }
    None
}

fn kimi_effort(v: Option<&str>) -> Option<String> {
    match v {
        None | Some("on") => None,
        Some(other) => effort_level(other).map(str::to_string),
    }
}

struct KimiRec {
    model_alias: Option<String>,
    thinking_effort: Option<String>,
    time: Option<i64>,
}

fn extract_json_string_field(head: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let start = head.find(&needle)? + needle.len();
    let rest = &head[start..];
    let mut escaped = false;
    let mut end_byte: Option<usize> = None;
    let mut units = 0usize;
    for (idx, ch) in rest.char_indices() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            end_byte = Some(idx);
            break;
        }
        units += ch.len_utf16();
        if units > 200 {
            return None;
        }
    }
    let idx = end_byte?;
    if idx == 0 {
        return None;
    }
    serde_json::from_str::<String>(&format!("\"{}\"", &rest[..idx])).ok()
}

fn kimi_record(line: &str) -> Option<KimiRec> {
    if !line.contains("\"profile.bind\"") && !line.contains("\"config.update\"") {
        return None;
    }
    if line.len() <= MAX_JSONL_LINE_BYTES
        && let Some(Value::Object(o)) = parse_bounded_json_line(line)
    {
        let ty = o.get("type").and_then(Value::as_str)?;
        if ty != "profile.bind" && ty != "config.update" {
            return None;
        }
        return Some(KimiRec {
            model_alias: o.get("modelAlias").and_then(model_id_val),
            thinking_effort: o
                .get("thinkingEffort")
                .and_then(Value::as_str)
                .map(str::to_string),
            time: o.get("time").and_then(time_of_value),
        });
    }
    let head = slice_utf16_prefix(line, 4096);
    if !(head.starts_with("{\"type\":\"profile.bind\"")
        || head.starts_with("{\"type\":\"config.update\""))
    {
        return None;
    }
    let alias = extract_json_string_field(head, "modelAlias")
        .and_then(|s| valid_model_id(&s).map(str::to_string));
    let thinking = extract_json_string_field(head, "thinkingEffort");
    Some(KimiRec {
        model_alias: alias,
        thinking_effort: thinking,
        time: None,
    })
}

/// Scans a Kimi `wire.jsonl` slice from newest to oldest (`kimiWireModel`).
#[must_use]
pub fn kimi_wire_model(text: &str) -> Option<KimiWirePartial> {
    if text.is_empty() || text.len() > MAX_STREAM_TEXT_BYTES {
        return None;
    }
    let mut model: Option<String> = None;
    let mut thinking: Option<String> = None;
    let mut at: Option<i64> = None;

    for raw_line in text.split('\n').rev().take(MAX_JSONL_LINES) {
        if model.is_some() && thinking.is_some() {
            break;
        }
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let Some(rec) = kimi_record(line) else {
            continue;
        };
        if thinking.is_none()
            && let Some(te) = rec.thinking_effort
        {
            thinking = Some(te);
            if at.is_none() {
                at = rec.time;
            }
        }
        if model.is_none()
            && let Some(ma) = rec.model_alias
        {
            model = Some(ma);
            if at.is_none() {
                at = rec.time;
            }
        }
    }
    if model.is_some() || thinking.is_some() {
        Some(KimiWirePartial {
            model,
            thinking,
            at,
        })
    } else {
        None
    }
}

/// Combines Kimi `wire.jsonl` tail and head slices (`kimiModelFromWire`).
#[must_use]
pub fn kimi_model_from_wire(tail_text: &str, head_text: &str) -> Option<FoundModel> {
    let tail = kimi_wire_model(tail_text);
    let head = if tail
        .as_ref()
        .is_some_and(|t| t.model.is_some() && t.thinking.is_some())
    {
        None
    } else {
        kimi_wire_model(head_text)
    };
    let model = tail
        .as_ref()
        .and_then(|t| t.model.clone())
        .or_else(|| head.as_ref().and_then(|h| h.model.clone()))?;
    let thinking = tail
        .as_ref()
        .and_then(|t| t.thinking.as_deref())
        .or_else(|| head.as_ref().and_then(|h| h.thinking.as_deref()));
    let at = tail
        .as_ref()
        .and_then(|t| t.at)
        .or_else(|| head.as_ref().and_then(|h| h.at));
    Some(FoundModel {
        model,
        name: None,
        effort: kimi_effort(thinking),
        at,
    })
}

enum PiFact {
    Level(String),
    Model(String),
}

fn pi_fact(o: &serde_json::Map<String, Value>) -> Option<PiFact> {
    let ty = o.get("type").and_then(Value::as_str)?;
    if ty == "thinking_level_change" {
        let lvl = o.get("thinkingLevel").and_then(Value::as_str)?;
        return Some(PiFact::Level(lvl.to_string()));
    }
    let (id, provider) = if ty == "model_change" {
        (
            o.get("modelId").and_then(model_id_val)?,
            o.get("provider").and_then(str_val),
        )
    } else if ty == "message" {
        let msg = o.get("message").and_then(Value::as_object)?;
        if msg.get("role").and_then(Value::as_str) != Some("assistant") {
            return None;
        }
        (
            msg.get("model").and_then(model_id_val)?,
            msg.get("provider").and_then(str_val),
        )
    } else {
        return None;
    };
    let full = match provider {
        Some(p) if !id.contains('/') => format!("{p}/{id}"),
        _ => id,
    };
    Some(PiFact::Model(full))
}

struct PiPartial {
    model: Option<String>,
    level: Option<String>,
    at: Option<i64>,
}

fn pi_records(text: &str) -> Option<PiPartial> {
    if text.is_empty() || text.len() > MAX_STREAM_TEXT_BYTES {
        return None;
    }
    let mut model: Option<String> = None;
    let mut level: Option<String> = None;
    let mut at: Option<i64> = None;

    for raw_line in text.split('\n').rev().take(MAX_JSONL_LINES) {
        if model.is_some() && level.is_some() {
            break;
        }
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.len() > MAX_JSONL_LINE_BYTES
            || !(line.contains("\"model") || line.contains("\"thinkingLevel\""))
        {
            continue;
        }
        let Some(Value::Object(o)) = parse_bounded_json_line(line) else {
            continue;
        };
        let Some(fact) = pi_fact(&o) else {
            continue;
        };
        match fact {
            PiFact::Level(lvl) => {
                if level.is_none() {
                    level = Some(lvl);
                    if at.is_none() {
                        at = o.get("timestamp").and_then(time_of_value);
                    }
                }
            }
            PiFact::Model(m) => {
                if model.is_none() {
                    model = Some(m);
                    if at.is_none() {
                        at = o.get("timestamp").and_then(time_of_value);
                    }
                }
            }
        }
    }
    if model.is_some() || level.is_some() {
        Some(PiPartial { model, level, at })
    } else {
        None
    }
}

/// Combines Pi / OMP / OpenClaw session JSONL tail and head slices (`piModelFromWire`).
#[must_use]
pub fn pi_model_from_wire(tail_text: &str, head_text: &str) -> Option<FoundModel> {
    let tail = pi_records(tail_text);
    let head = if tail
        .as_ref()
        .is_some_and(|t| t.model.is_some() && t.level.is_some())
    {
        None
    } else {
        pi_records(head_text)
    };
    let model = tail
        .as_ref()
        .and_then(|t| t.model.clone())
        .or_else(|| head.as_ref().and_then(|h| h.model.clone()))?;
    let level = tail
        .as_ref()
        .and_then(|t| t.level.as_deref())
        .or_else(|| head.as_ref().and_then(|h| h.level.as_deref()));
    let at = tail
        .as_ref()
        .and_then(|t| t.at)
        .or_else(|| head.as_ref().and_then(|h| h.at));
    Some(FoundModel {
        model,
        name: None,
        effort: level.and_then(effort_level).map(str::to_string),
        at,
    })
}

/// Extracts the latest Pi / OMP / OpenClaw model from a single JSONL slice (`piModelFromText`).
#[must_use]
pub fn pi_model_from_text(text: &str) -> Option<FoundModel> {
    pi_model_from_wire(text, "")
}

/// Chronological Pi / OMP / OpenClaw model and thinking-level timeline (`piModelTimeline`).
#[must_use]
pub fn pi_model_timeline<S: AsRef<str>>(lines: &[S]) -> Vec<ModelTimelinePoint> {
    let mut out = Vec::new();
    let mut model = String::new();
    let mut level: Option<String> = None;

    for raw in lines.iter().take(MAX_JSONL_LINES) {
        let line = raw.as_ref();
        if line.len() > MAX_JSONL_LINE_BYTES
            || !(line.contains("\"model") || line.contains("\"thinkingLevel\""))
        {
            continue;
        }
        let Some(Value::Object(o)) = parse_bounded_json_line(line) else {
            continue;
        };
        let Some(fact) = pi_fact(&o) else {
            continue;
        };
        let Some(t) = o.get("timestamp").and_then(time_of_value) else {
            continue;
        };
        let (next_model, next_level) = match fact {
            PiFact::Model(m) => (m, level.clone()),
            PiFact::Level(lvl) => (model.clone(), effort_level(&lvl).map(str::to_string)),
        };
        if next_model.is_empty() || (next_model == model && next_level == level) {
            continue;
        }
        model = next_model.clone();
        level.clone_from(&next_level);
        out.push(ModelTimelinePoint {
            t,
            model: next_model,
            effort: next_level,
        });
    }
    out
}

/// Parses OpenClaw's `sessions.json` index plus optional newest session JSONL (`openclawSession`).
#[must_use]
pub fn openclaw_index_model(
    index_json: &str,
    session_tail: Option<&str>,
    session_head: Option<&str>,
) -> Option<FoundModel> {
    let map = parse_strict_ordered_object(index_json)?;
    let mut best: Option<&OrderedObject> = None;
    let mut best_updated = i64::MIN;
    for (_, val) in map.own_entries().into_iter().take(1_024) {
        let Some(obj) = val.as_object() else {
            continue;
        };
        if obj
            .get("model")
            .and_then(OrderedJson::as_str)
            .and_then(valid_model_id)
            .is_none()
        {
            continue;
        }
        let updated = obj
            .get("updatedAt")
            .and_then(OrderedJson::to_scalar_value)
            .as_ref()
            .and_then(num_i64)
            .unwrap_or(0);
        if best.is_none() || updated > best_updated {
            best = Some(obj);
            best_updated = updated;
        }
    }
    let best = best?;
    let raw_model = best
        .get("model")
        .and_then(OrderedJson::as_str)
        .and_then(valid_model_id)
        .map(str::to_string)?;
    let provider = best
        .get("modelProvider")
        .and_then(OrderedJson::as_str)
        .and_then(trimmed_non_empty);
    let model = match provider {
        Some(p) if !raw_model.contains('/') => format!("{p}/{raw_model}"),
        _ => raw_model,
    };
    let from_file = session_tail.and_then(|t| pi_model_from_wire(t, session_head.unwrap_or("")));
    let at = best
        .get("updatedAt")
        .and_then(OrderedJson::to_scalar_value)
        .and_then(|v| time_of_value(&v));
    Some(FoundModel {
        model: from_file.as_ref().map_or(model, |f| f.model.clone()),
        name: None,
        effort: from_file.and_then(|f| f.effort),
        at,
    })
}

fn opencode_model_entry(
    id: Option<&Value>,
    provider_id: Option<&Value>,
    variant: Option<&Value>,
) -> Option<FoundModel> {
    let m = id.and_then(model_id_val)?;
    let p = provider_id.and_then(str_val);
    let model = match p {
        Some(prov) if !m.contains('/') => format!("{prov}/{m}"),
        _ => m,
    };
    let effort = match variant.and_then(Value::as_str) {
        Some("default") | None => None,
        Some(v) => effort_level(v).map(str::to_string),
    };
    Some(FoundModel {
        model,
        name: None,
        effort,
        at: None,
    })
}

/// Parses one OpenCode / Kilo / ZCode `message.data` JSON string (`opencodeMessageModel`).
#[must_use]
pub fn opencode_message_model(data_json: &str) -> Option<FoundModel> {
    let o = parse_strict_object(data_json)?;
    if o.get("role").and_then(Value::as_str) == Some("user") {
        let m = o.get("model").and_then(Value::as_object)?;
        return opencode_model_entry(
            m.get("modelID").or_else(|| m.get("id")),
            m.get("providerID"),
            m.get("variant"),
        );
    }
    opencode_model_entry(o.get("modelID"), o.get("providerID"), o.get("variant"))
}

/// Parses OpenCode / Kilo / ZCode's `session.model` JSON column (`opencodeSessionColumn`).
#[must_use]
pub fn opencode_session_column(col_json: &str) -> Option<FoundModel> {
    let o = parse_strict_object(col_json)?;
    opencode_model_entry(
        o.get("id").or_else(|| o.get("modelID")),
        o.get("providerID"),
        o.get("variant"),
    )
}

/// Resolves the active model of an OpenCode / Kilo / ZCode session from its `session.model`
/// column and newest message rows (`opencodeSessionModel`).
#[must_use]
pub fn opencode_session_model(
    column_json: Option<&str>,
    rows: &[OpenCodeMessageRow],
    updated_ms: Option<i64>,
) -> Option<FoundModel> {
    let col = column_json.and_then(opencode_session_column);
    for row in rows.iter().take(MAX_DB_ROWS) {
        let Some(m) = opencode_message_model(&row.data) else {
            continue;
        };
        let effort = m.effort.or_else(|| {
            col.as_ref()
                .filter(|c| c.model == m.model)
                .and_then(|c| c.effort.clone())
        });
        let at = row.time_created.filter(|&t| t != 0);
        return Some(FoundModel {
            model: m.model,
            name: None,
            effort,
            at,
        });
    }
    col.map(|mut c| {
        c.at = updated_ms.filter(|&t| t != 0);
        c
    })
}

/// Selects the active Cline session row for `cwd` among running non-subagent PIDs (`clineSessionModel`).
#[must_use]
pub fn cline_session_model(
    rows: &[ClineSessionRow],
    cwd: Option<&str>,
    mut is_pid_alive: impl FnMut(u32) -> bool,
) -> Option<FoundModel> {
    let want = cwd.map(norm_dir).unwrap_or_default();
    let mut best: Option<(bool, i64, String)> = None;

    for r in rows.iter().take(MAX_DB_ROWS) {
        if r.ended_at.as_deref().is_some_and(|s| !s.is_empty()) || r.is_subagent {
            continue;
        }
        let Some(model) = r
            .model
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(valid_model_id)
        else {
            continue;
        };
        if !is_pid_alive(r.pid) {
            continue;
        }
        let here = !want.is_empty() && r.cwd.as_deref().is_some_and(|rcwd| norm_dir(rcwd) == want);
        let at = r.updated_at.as_deref().and_then(parse_iso_ms).unwrap_or(0);
        let replace = match &best {
            None => true,
            Some((b_here, b_at, _)) => (here && !*b_here) || (here == *b_here && at > *b_at),
        };
        if replace {
            best = Some((here, at, model.to_string()));
        }
    }

    best.map(|(_, at, model)| FoundModel {
        model,
        name: None,
        effort: None,
        at: (at != 0).then_some(at),
    })
}

// =============================================================================
// 5. Command-line flags & TOML / YAML / JSON settings parsers
// =============================================================================

fn find_flag_value<'a>(text: &'a str, flags: &[&str]) -> Option<&'a str> {
    for (idx, ch) in text.char_indices() {
        if ch != '-' {
            continue;
        }
        if idx > 0 {
            let prev = text[..idx].chars().next_back()?;
            if !crate::agent_env::js_space(prev) {
                continue;
            }
        }
        let rest = &text[idx..];
        for flag in flags {
            let Some(after_flag) = rest.strip_prefix(flag) else {
                continue;
            };
            let val_part = if let Some(eq_rest) = after_flag.strip_prefix('=') {
                eq_rest
            } else if after_flag.starts_with(crate::agent_env::js_space) {
                after_flag.trim_start_matches(crate::agent_env::js_space)
            } else {
                continue;
            };
            if val_part.is_empty() {
                continue;
            }
            if let Some(q) = val_part.strip_prefix('"') {
                let end = q.find('"')?;
                if end > 0 {
                    return Some(&q[..end]);
                }
            } else if let Some(q) = val_part.strip_prefix('\'') {
                let end = q.find('\'')?;
                if end > 0 {
                    return Some(&q[..end]);
                }
            } else {
                let end = val_part
                    .find(|c: char| crate::agent_env::js_space(c) || c == '"' || c == '\'')
                    .unwrap_or(val_part.len());
                if end > 0 {
                    return Some(&val_part[..end]);
                }
            }
        }
    }
    None
}

fn find_flag_alpha_word<'a>(
    text: &'a str,
    flags: &[&str],
    allow_dash_underscore: bool,
) -> Option<&'a str> {
    for (idx, ch) in text.char_indices() {
        if ch != '-' {
            continue;
        }
        if idx > 0 {
            let prev = text[..idx].chars().next_back()?;
            if !crate::agent_env::js_space(prev) {
                continue;
            }
        }
        let rest = &text[idx..];
        for flag in flags {
            let Some(after_flag) = rest.strip_prefix(flag) else {
                continue;
            };
            let mut val_part = if let Some(eq_rest) = after_flag.strip_prefix('=') {
                eq_rest
            } else if after_flag.starts_with(crate::agent_env::js_space) {
                after_flag.trim_start_matches(crate::agent_env::js_space)
            } else {
                continue;
            };
            if let Some(stripped) = val_part
                .strip_prefix('"')
                .or_else(|| val_part.strip_prefix('\''))
            {
                val_part = stripped;
            }
            let end = val_part
                .find(|c: char| {
                    !(c.is_ascii_alphabetic() || (allow_dash_underscore && (c == '_' || c == '-')))
                })
                .unwrap_or(val_part.len());
            if end > 0 {
                let after = val_part[end..].chars().next();
                if after.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_')) {
                    return Some(&val_part[..end]);
                }
            }
        }
    }
    None
}

fn command_words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        rest = rest.trim_start_matches(crate::agent_env::js_space);
        if rest.is_empty() {
            break;
        }
        if let Some(after_quote) = rest.strip_prefix('"')
            && let Some(end) = after_quote.find('"')
        {
            out.push(&after_quote[..end]);
            rest = &after_quote[end + 1..];
            continue;
        }
        let end = rest.find(crate::agent_env::js_space).unwrap_or(rest.len());
        out.push(&rest[..end]);
        rest = &rest[end..];
    }
    out
}

fn is_ollama_bin(word: &str) -> bool {
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    base.eq_ignore_ascii_case("ollama") || base.eq_ignore_ascii_case("ollama.exe")
}

/// Extracts `--model` / `-m` or `ollama run <model>` from a launch command (`modelFromCommand`).
#[must_use]
pub fn model_from_command(cmd: CommandLine<'_>) -> Option<String> {
    let text = cmd.bounded_str();
    if let Some(m) = find_flag_value(text, &["--model", "-m"]) {
        return valid_model_id(m).map(str::to_string);
    }
    let words = command_words(text);
    for i in 0..words.len().saturating_sub(1) {
        if !is_ollama_bin(words[i]) || words[i + 1] != "run" {
            continue;
        }
        let mut j = i + 2;
        while j < words.len() {
            let t = words[j];
            if matches!(t, "--format" | "--keepalive" | "--dimensions" | "--think") {
                j += 2;
                continue;
            }
            if !t.starts_with('-') {
                return valid_model_id(t).map(str::to_string);
            }
            j += 1;
        }
    }
    None
}

/// Extracts Claude / Antigravity `--effort <low|medium|high|xhigh|max>` (`effortFromCommand`).
#[must_use]
pub fn effort_from_command(cmd: CommandLine<'_>) -> Option<String> {
    let raw = find_flag_alpha_word(cmd.bounded_str(), &["--effort"], false)?;
    let lower = raw.to_ascii_lowercase();
    is_claude_effort(&lower).then_some(lower)
}

/// Extracts Muse `--reasoning-effort` / `--effort` among Muse's 8 tiers (`museEffortFromCommand`).
#[must_use]
pub fn muse_effort_from_command(cmd: CommandLine<'_>) -> Option<String> {
    let raw = find_flag_alpha_word(
        cmd.bounded_str(),
        &["--reasoning-effort", "--effort"],
        false,
    )?;
    valid_muse_effort(raw)
}

/// Extracts general `--reasoning-effort`, `--effort`, `--thinking-level`, or `--thinking` (`effortFlag`).
#[must_use]
pub fn effort_flag(cmd: CommandLine<'_>) -> Option<String> {
    let raw = find_flag_alpha_word(
        cmd.bounded_str(),
        &[
            "--reasoning-effort",
            "--effort",
            "--thinking-level",
            "--thinking",
        ],
        true,
    )?;
    effort_level(raw).map(str::to_string)
}

/// Extracts Amp's `--mode <smart|rush|large|deep|free>` (`ampModeFromCommand`).
#[must_use]
pub fn amp_mode_from_command(cmd: CommandLine<'_>) -> Option<String> {
    let raw = find_flag_alpha_word(cmd.bounded_str(), &["--mode"], false)?;
    let lower = raw.to_ascii_lowercase();
    AMP_MODES.contains(&lower.as_str()).then_some(lower)
}

fn parse_toml_header_parts(inner: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut chars = inner.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c == '.' || crate::agent_env::js_space(c) {
            let _ = chars.next();
            continue;
        }
        if c == '"' {
            let _ = chars.next();
            let mut s = String::new();
            while let Some(sc) = chars.next() {
                if sc == '\\' {
                    if let Some(esc) = chars.next() {
                        s.push(esc);
                    }
                } else if sc == '"' {
                    break;
                } else {
                    s.push(sc);
                }
            }
            parts.push(s);
        } else if c == '\'' {
            let _ = chars.next();
            let mut s = String::new();
            for sc in chars.by_ref() {
                if sc == '\'' {
                    break;
                }
                s.push(sc);
            }
            parts.push(s);
        } else {
            let mut s = String::new();
            while let Some(&sc) = chars.peek() {
                if sc == '.' || sc == '"' || sc == '\'' || crate::agent_env::js_space(sc) {
                    break;
                }
                s.push(sc);
                let _ = chars.next();
            }
            if !s.is_empty() {
                parts.push(s);
            }
        }
    }
    parts
}

fn strip_toml_header_line(line: &str) -> Option<&str> {
    let after_open = line.strip_prefix('[')?;
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    let mut close_idx: Option<usize> = None;
    for (idx, ch) in after_open.char_indices() {
        if in_double {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
            }
            continue;
        }
        if in_single {
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        match ch {
            '"' => in_double = true,
            '\'' => in_single = true,
            ']' => close_idx = Some(idx),
            _ => {}
        }
    }
    let idx = close_idx?;
    let inner = &after_open[..idx];
    if inner.is_empty() {
        return None;
    }
    let trailing = crate::agent_env::trim(&after_open[idx + 1..]);
    if !trailing.is_empty() && !trailing.starts_with('#') {
        return None;
    }
    Some(inner)
}

fn parse_toml_kv(line: &str) -> Option<(String, TomlScalar)> {
    let (key, rest) = if let Some(after_q) = line.strip_prefix('"') {
        let end = after_q.find('"')?;
        if end == 0 {
            return None;
        }
        (after_q[..end].to_string(), &after_q[end + 1..])
    } else {
        let end = line
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(line.len());
        if end == 0 {
            return None;
        }
        (line[..end].to_string(), &line[end..])
    };
    let rest = rest.trim_start_matches(crate::agent_env::js_space);
    let rhs = crate::agent_env::trim(rest.strip_prefix('=')?);
    if rhs.is_empty() {
        return None;
    }
    if let Some(after_q) = rhs.strip_prefix('"') {
        let mut escaped = false;
        let mut end_idx: Option<usize> = None;
        for (idx, ch) in after_q.char_indices() {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                end_idx = Some(idx);
                break;
            }
        }
        let idx = end_idx?;
        let s = serde_json::from_str::<String>(&format!("\"{}\"", &after_q[..idx])).ok()?;
        return Some((key, TomlScalar::Str(s)));
    }
    if let Some(after_q) = rhs.strip_prefix('\'') {
        let idx = after_q.find('\'')?;
        return Some((key, TomlScalar::Str(after_q[..idx].to_string())));
    }
    let mut bare = rhs;
    for (idx, ch) in rhs.char_indices() {
        if ch == '#' && idx > 0 {
            let prev = rhs[..idx].chars().next_back()?;
            if crate::agent_env::js_space(prev) {
                bare = crate::agent_env::trim(&rhs[..idx]);
                break;
            }
        }
    }
    if bare == "true" {
        return Some((key, TomlScalar::Bool(true)));
    }
    if bare == "false" {
        return Some((key, TomlScalar::Bool(false)));
    }
    let unsigned = bare.strip_prefix('-').unwrap_or(bare);
    if is_unsigned_decimal(unsigned)
        && let Ok(n) = bare.parse::<f64>()
        && n.is_finite()
    {
        return Some((key, TomlScalar::Num(n)));
    }
    None
}

/// Bounded scalar-only TOML section reader (`agentModelSources.js` `tomlKeys`).
#[must_use]
pub fn toml_keys(text: &str) -> TomlTable {
    let mut out = TomlTable::new();
    out.insert(String::new(), BTreeMap::new());
    if text.is_empty() || text.len() > MAX_DOCUMENT_BYTES {
        return out;
    }
    let mut active_section: Option<String> = Some(String::new());
    for raw in text.split('\n').take(MAX_TOML_LINES) {
        let line = crate::agent_env::trim(raw);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("[[") {
            active_section = None;
            continue;
        }
        if let Some(inner) = strip_toml_header_line(line) {
            let name = parse_toml_header_parts(inner).join("\n");
            if !out.contains_key(&name) {
                if out.len() >= MAX_TOML_SECTIONS {
                    active_section = None;
                    continue;
                }
                out.insert(name.clone(), BTreeMap::new());
            }
            active_section = Some(name);
            continue;
        }
        let Some(ref sec_name) = active_section else {
            continue;
        };
        if let Some((k, v)) = parse_toml_kv(line)
            && let Some(sec) = out.get_mut(sec_name)
        {
            sec.insert(k, v);
        }
    }
    out
}

/// Parses Codex's top-level `model` and `model_reasoning_effort` before the first `[section]` (`codexModelFromToml`).
#[must_use]
pub fn codex_model_from_toml(text: &str) -> Option<FoundModel> {
    if text.is_empty() || text.len() > MAX_DOCUMENT_BYTES {
        return None;
    }
    let mut model: Option<String> = None;
    let mut effort: Option<String> = None;
    for raw in text.split('\n').take(MAX_TOML_LINES) {
        let trimmed = raw.trim_start_matches(crate::agent_env::js_space);
        if trimmed.starts_with('[') {
            break;
        }
        if let Some((k, TomlScalar::Str(v))) = parse_toml_kv(trimmed)
            && !v.is_empty()
        {
            if k == "model" && model.is_none() {
                model = valid_model_id(&v).map(str::to_string);
            } else if k == "model_reasoning_effort" && effort.is_none() {
                effort = Some(v);
            }
        }
    }
    Some(FoundModel {
        model: model?,
        name: None,
        effort,
        at: None,
    })
}

/// Parses Claude's `settings.local.json` / `settings.json` model (`settingsModel('claude', ...)`).
#[must_use]
pub fn claude_settings_model<S: AsRef<str>>(docs_in_priority_order: &[S]) -> Option<String> {
    for doc in docs_in_priority_order {
        let Some(o) = parse_loose_object(doc.as_ref()) else {
            continue;
        };
        if let Some(m) = o.get("model").and_then(model_id_val) {
            return Some(m);
        }
    }
    None
}

/// Resolves Claude's per-model `modelSettings[id].effortLevel` or top-level `effortLevel` (`claudeSettingsEffort`).
#[must_use]
pub fn claude_settings_effort<S: AsRef<str>>(
    model: Option<&str>,
    docs_in_priority_order: &[S],
) -> Option<String> {
    let id = model
        .map(|m| strip_1m_suffix(m).to_lowercase())
        .unwrap_or_default();
    let id_is_alpha = !id.is_empty() && id.bytes().all(|b| b.is_ascii_lowercase());
    let mut general: Option<String> = None;

    for doc in docs_in_priority_order {
        let Some(o) = parse_loose_ordered_object(doc.as_ref()) else {
            continue;
        };
        if !id.is_empty()
            && let Some(entries) = o
                .get("modelSettings")
                .and_then(OrderedJson::js_object_entries)
        {
            for (key, value) in entries {
                let level = value
                    .as_object()
                    .and_then(|vo| vo.get("effortLevel"))
                    .and_then(OrderedJson::as_str);
                let k = key.to_lowercase();
                let hit = k == id || (id_is_alpha && k.contains(&format!("-{id}-")));
                if hit && let Some(lvl) = level.filter(|l| is_claude_effort(l)) {
                    return Some(lvl.to_string());
                }
            }
        }
        if general.is_none()
            && let Some(lvl) = o
                .get("effortLevel")
                .and_then(OrderedJson::as_str)
                .filter(|l| is_claude_effort(l))
        {
            general = Some(lvl.to_string());
        }
    }
    general
}

/// Parses OpenClaude's `~/.openclaude/settings.json` (`settingsModel('openclaude', ...)`).
#[must_use]
pub fn openclaude_settings_model(json_text: &str) -> Option<FoundModel> {
    let o = parse_loose_object(json_text)?;
    let model = o.get("model").and_then(model_id_val)?;
    let effort = o
        .get("effortLevel")
        .and_then(Value::as_str)
        .filter(|l| is_claude_effort(l))
        .map(str::to_string);
    Some(FoundModel {
        model,
        name: None,
        effort,
        at: None,
    })
}

/// Parses Gemini CLI or Qwen Code `settings.json` (`model` string or `model.name` object).
#[must_use]
pub fn gemini_or_qwen_settings_model<S: AsRef<str>>(
    docs_in_priority_order: &[S],
) -> Option<FoundModel> {
    for doc in docs_in_priority_order {
        let Some(o) = parse_loose_object(doc.as_ref()) else {
            continue;
        };
        let picked = match o.get("model") {
            Some(Value::Object(mo)) => mo.get("name").and_then(model_id_val),
            Some(other) => model_id_val(other),
            None => None,
        };
        if let Some(model) = picked {
            return Some(FoundModel {
                model,
                name: None,
                effort: None,
                at: None,
            });
        }
    }
    None
}

fn plain_jsonc_docs_model<S: AsRef<str>>(docs: &[S]) -> Option<FoundModel> {
    for doc in docs {
        let Some(o) = parse_loose_object(doc.as_ref()) else {
            continue;
        };
        if let Some(model) = o.get("model").and_then(model_id_val) {
            return Some(FoundModel {
                model,
                name: None,
                effort: None,
                at: None,
            });
        }
    }
    None
}

/// Parses OpenCode `opencode.json` / `opencode.jsonc` settings (`settingsModel('opencode', ...)`).
#[must_use]
pub fn opencode_settings_model<S: AsRef<str>>(docs_in_priority_order: &[S]) -> Option<FoundModel> {
    plain_jsonc_docs_model(docs_in_priority_order)
}

/// Parses Kilo `kilo.json` / `kilo.jsonc` settings (`kiloConfigModel`).
#[must_use]
pub fn kilo_config_model<S: AsRef<str>>(docs_in_priority_order: &[S]) -> Option<FoundModel> {
    plain_jsonc_docs_model(docs_in_priority_order)
}

/// Parses Copilot `settings.json` / `config.json` (`settingsModel('copilot', ...)`).
#[must_use]
pub fn copilot_settings_model<S: AsRef<str>>(docs_in_priority_order: &[S]) -> Option<FoundModel> {
    plain_jsonc_docs_model(docs_in_priority_order)
}

/// Parses OpenCode / Kilo `model.json` last-picked model (`opencodePicked`).
#[must_use]
pub fn opencode_picked_model(json_text: &str, mtime_ms: i64) -> Option<PickedModel> {
    let o = parse_loose_object(json_text)?;
    let r = o.get("recent")?.as_array()?.first()?.as_object()?;
    let model_id = r.get("modelID").and_then(model_id_val)?;
    let model = match r.get("providerID").and_then(str_val) {
        Some(p) if !p.is_empty() => format!("{p}/{model_id}"),
        _ => model_id,
    };
    let variant = o
        .get("variant")
        .and_then(Value::as_object)
        .and_then(|vm| vm.get(&model))
        .and_then(Value::as_str);
    let effort = match variant {
        Some("default") | None => None,
        Some(v) => effort_level(v).map(str::to_string),
    };
    Some(PickedModel {
        model,
        effort,
        mtime_ms,
    })
}

fn find_cursor_effort_word(name: &str) -> Option<(usize, usize, &'static str)> {
    let lower = name.to_ascii_lowercase();
    for (word, value) in CURSOR_EFFORT_WORDS {
        let needle = format!(" {word}");
        let mut search_from = 0usize;
        while let Some(rel) = lower[search_from..].find(&needle) {
            let start = search_from + rel;
            let end = start + needle.len();
            if lower.as_bytes().get(end).is_none_or(|&b| b == b' ') {
                return Some((start, end, value));
            }
            search_from = start + 1;
        }
    }
    None
}

fn strip_cursor_context_tokens(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for token in name.split(' ') {
        if token.is_empty() {
            continue;
        }
        let is_ctx = if let Some(num) = token.strip_suffix(['K', 'M']) {
            is_unsigned_decimal(num)
        } else {
            false
        };
        if is_ctx && !out.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(token);
    }
    out
}

/// Parses Cursor's `~/.cursor/cli-config.json` (`cursorConfigModel`).
#[must_use]
pub fn cursor_config_model(text: &str) -> Option<FoundModel> {
    let o = parse_strict_object(text)?;
    let m = o.get("model")?.as_object()?;
    let id = m.get("modelId")?.as_str()?;
    if id.is_empty()
        || utf16_len(id) > 80
        || !id
            .bytes()
            .all(|b| is_word_byte(b) || b == b'.' || b == b'-')
    {
        return None;
    }
    let mut name = m
        .get("displayName")
        .and_then(Value::as_str)
        .map(|d| slice_utf16_prefix(&collapse_js_whitespace(d), MAX_MODEL_NAME_UTF16).to_string())
        .unwrap_or_default();
    let mut effort: Option<String> = None;
    if let Some((start, end, val)) = find_cursor_effort_word(&name) {
        effort = Some(val.to_string());
        name.replace_range(start..end, "");
    }
    let clean_name = crate::agent_env::trim(&strip_cursor_context_tokens(&name)).to_string();
    Some(FoundModel {
        model: id.to_string(),
        name: (!clean_name.is_empty()).then_some(clean_name),
        effort,
        at: None,
    })
}

/// Parses Antigravity's `~/.gemini/antigravity-cli/settings.json` (`antigravitySettings`).
#[must_use]
pub fn antigravity_settings_model(json_text: &str) -> Option<FoundModel> {
    let o = parse_loose_object(json_text)?;
    let label = o.get("model").and_then(model_id_val)?;
    let (name, effort) = crate::agent_model_antigravity::label_parts(&label)?;
    Some(FoundModel {
        model: label,
        name: Some(name),
        effort: effort.map(str::to_string),
        at: None,
    })
}

/// Parses Muse Code's `~/.config/muse/settings.json` (`museSettingsModel`).
#[must_use]
pub fn muse_settings_model(json_text: &str) -> Option<MuseSettingsModel> {
    let o = parse_strict_ordered_object(json_text)?;
    let nested = o.get("model").and_then(OrderedJson::as_object);
    let model = [
        o.get("model"),
        o.get("model_id"),
        o.get("default_model"),
        nested.and_then(|n| n.get("id")),
        nested.and_then(|n| n.get("model_id")),
        nested.and_then(|n| n.get("default")),
    ]
    .into_iter()
    .flatten()
    .filter_map(OrderedJson::as_str)
    .find_map(valid_muse_model_id)
    .map(str::to_string);

    let mut efforts = BTreeMap::new();
    if let Some(map) = o
        .get("model_reasoning_efforts")
        .and_then(OrderedJson::as_object)
    {
        for (id, tier) in map.own_entries().into_iter().take(MAX_MUSE_EFFORT_ENTRIES) {
            if let (Some(clean_id), Some(clean_tier)) = (
                valid_muse_model_id(id),
                tier.as_str().and_then(valid_muse_effort),
            ) {
                efforts.insert(clean_id.to_string(), clean_tier);
            }
        }
    }

    let all_effort = [
        o.get("reasoning_effort"),
        nested.and_then(|n| n.get("reasoning_effort")),
        nested.and_then(|n| n.get("effort")),
    ]
    .into_iter()
    .flatten()
    .filter_map(OrderedJson::as_str)
    .find_map(valid_muse_effort);

    if model.is_none() && efforts.is_empty() && all_effort.is_none() {
        return None;
    }

    let mut parsed = MuseSettingsModel {
        model,
        effort: None,
        efforts,
        all_effort,
    };
    if let Some(ref m) = parsed.model {
        parsed.effort = parsed.effort_for(m).map(str::to_string);
    }
    Some(parsed)
}

/// Looks up a Kimi model alias's `display_name` in parsed `config.toml` (`kimiDisplayName`).
#[must_use]
pub fn kimi_display_name(toml: &TomlTable, alias: &str) -> Option<String> {
    let sec = toml.get(&format!("models\n{alias}"))?;
    let raw = sec.get("display_name")?.as_str()?;
    let trimmed = trimmed_non_empty(raw)?;
    Some(slice_utf16_prefix(trimmed, MAX_MODEL_NAME_UTF16).to_string())
}

/// Parses Kimi's `config.toml` (`kimiConfigModel`).
#[must_use]
pub fn kimi_config_model(toml_text: &str) -> Option<FoundModel> {
    let toml = toml_keys(toml_text);
    let model = toml
        .get("")?
        .get("default_model")?
        .as_str()
        .and_then(valid_model_id)?
        .to_string();
    let thinking = toml.get("thinking");
    let effort = if thinking
        .and_then(|t| t.get("enabled"))
        .and_then(TomlScalar::as_bool)
        == Some(false)
    {
        Some("none".to_string())
    } else {
        kimi_effort(
            thinking
                .and_then(|t| t.get("effort"))
                .and_then(TomlScalar::as_str),
        )
    };
    let name = kimi_display_name(&toml, &model);
    Some(FoundModel {
        model,
        name,
        effort,
        at: None,
    })
}

/// Parses Pi's `settings.json` (`piSettingsModel`).
#[must_use]
pub fn pi_settings_model(o: &Value) -> Option<FoundModel> {
    let obj = o.as_object()?;
    let id = obj.get("defaultModel").and_then(model_id_val)?;
    let model = match obj.get("defaultProvider").and_then(str_val) {
        Some(p) if !id.contains('/') => format!("{p}/{id}"),
        _ => id,
    };
    let effort = obj
        .get("defaultThinkingLevel")
        .and_then(Value::as_str)
        .and_then(effort_level)
        .map(str::to_string);
    Some(FoundModel {
        model,
        name: None,
        effort,
        at: None,
    })
}

/// Parses OpenClaw's `openclaw.json` (`openclawConfigModel`).
#[must_use]
pub fn openclaw_config_model(o: &Value) -> Option<FoundModel> {
    let m = o
        .as_object()?
        .get("agents")?
        .as_object()?
        .get("defaults")?
        .as_object()?
        .get("model")?;
    let id = match m {
        Value::String(_) => model_id_val(m),
        Value::Object(mo) => mo.get("primary").and_then(model_id_val),
        _ => None,
    }?;
    Some(FoundModel {
        model: id,
        name: None,
        effort: None,
        at: None,
    })
}

/// Parses Droid's `settings.json` / `<id>.settings.json` (`droidSettingsModel`).
#[must_use]
pub fn droid_settings_model(o: &Value) -> Option<FoundModel> {
    let obj = o.as_object()?;
    let model = obj.get("model").and_then(model_id_val)?;
    let effort = obj
        .get("reasoningEffort")
        .and_then(Value::as_str)
        .and_then(effort_level)
        .map(str::to_string);
    Some(FoundModel {
        model,
        name: None,
        effort,
        at: None,
    })
}

/// Parses Cline's `providers.json` (`clineSettings`).
#[must_use]
pub fn cline_settings_model(json_text: &str, mtime_ms: i64) -> Option<ClineSettingsModel> {
    let o = parse_loose_object(json_text)?;
    let last_provider = o.get("lastUsedProvider")?.as_str()?;
    let settings = o
        .get("providers")?
        .as_object()?
        .get(last_provider)?
        .as_object()?
        .get("settings")?
        .as_object()?;
    let model = settings
        .get("model")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .and_then(valid_model_id)?
        .to_string();
    let effort = settings
        .get("reasoning")
        .and_then(Value::as_object)
        .and_then(|r| {
            if r.get("enabled") == Some(&Value::Bool(false)) {
                Some("none".to_string())
            } else {
                r.get("effort")
                    .and_then(Value::as_str)
                    .and_then(effort_level)
                    .map(str::to_string)
            }
        });
    Some(ClineSettingsModel {
        model,
        effort,
        mtime_ms,
    })
}

fn yaml_unquote(v: &str) -> String {
    let t = crate::agent_env::trim(v);
    if t.len() >= 2
        && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')))
    {
        crate::agent_env::trim(&t[1..t.len() - 1]).to_string()
    } else {
        t.to_string()
    }
}

fn parse_yaml_kv(s: &str) -> Option<(&str, &str)> {
    let colon = s.find(':')?;
    let key = &s[..colon];
    if key.is_empty() || !key.bytes().all(is_word_byte) {
        return None;
    }
    let val = s[colon + 1..].trim_start_matches(crate::agent_env::js_space);
    Some((key, val))
}

/// Parses Continue's `config.yaml` `models:` list (`continueModels`).
#[must_use]
pub fn continue_models(yaml: &str) -> Vec<ContinueModelItem> {
    if yaml.is_empty() || yaml.len() > MAX_DOCUMENT_BYTES {
        return Vec::new();
    }
    struct RawItem {
        name: String,
        model: String,
        roles: Option<Vec<String>>,
    }
    let mut items: Vec<RawItem> = Vec::new();
    let mut in_models = false;
    let mut in_item = false;
    let mut in_roles = false;

    for raw_line in yaml.split('\n').take(MAX_YAML_LINES) {
        let raw = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if raw
            .chars()
            .next()
            .is_some_and(|c| !crate::agent_env::js_space(c))
        {
            in_models = crate::agent_env::trim(raw) == "models:";
            in_item = false;
            in_roles = false;
            continue;
        }
        if !in_models {
            continue;
        }
        let leading_spaces = raw
            .chars()
            .take_while(|&c| crate::agent_env::js_space(c))
            .count();
        let trimmed = raw.trim_start_matches(crate::agent_env::js_space);

        if leading_spaces <= 4
            && let Some(after_dash) = trimmed.strip_prefix('-')
            && after_dash.starts_with(crate::agent_env::js_space)
        {
            let rest = after_dash.trim_start_matches(crate::agent_env::js_space);
            if let Some((k, v)) = parse_yaml_kv(rest) {
                let mut item = RawItem {
                    name: String::new(),
                    model: String::new(),
                    roles: None,
                };
                if k == "name" {
                    item.name = yaml_unquote(v);
                } else if k == "model" {
                    item.model = yaml_unquote(v);
                }
                items.push(item);
                in_item = true;
                in_roles = false;
                continue;
            }
        }
        if !in_item {
            continue;
        }
        if leading_spaces >= 1
            && let Some((k, v)) = parse_yaml_kv(trimmed)
        {
            let Some(cur) = items.last_mut() else {
                continue;
            };
            in_roles = k == "roles";
            if k == "name" {
                cur.name = yaml_unquote(v);
            } else if k == "model" {
                cur.model = yaml_unquote(v);
            }
            if in_roles {
                let vt = crate::agent_env::trim(v);
                if vt.starts_with('[') && vt.ends_with(']') {
                    cur.roles = Some(vt[1..vt.len() - 1].split(',').map(yaml_unquote).collect());
                } else {
                    cur.roles = Some(Vec::new());
                }
            }
            continue;
        }
        if leading_spaces >= 1
            && in_roles
            && let Some(after_dash) = trimmed.strip_prefix('-')
            && after_dash.starts_with(crate::agent_env::js_space)
            && let Some(cur) = items.last_mut()
            && let Some(ref mut roles) = cur.roles
        {
            let val = yaml_unquote(after_dash);
            if !val.is_empty() {
                roles.push(val);
            }
        }
    }

    items
        .into_iter()
        .filter(|m| !m.name.is_empty() || !m.model.is_empty())
        .map(|m| ContinueModelItem {
            name: m.name,
            model: m.model,
            chat: m
                .roles
                .as_ref()
                .is_none_or(|r| r.iter().any(|x| x == "chat")),
        })
        .collect()
}

/// Resolves Continue's selected or default chat model (`continueModel`).
#[must_use]
pub fn continue_model(selected: Option<&str>, yaml: &str) -> Option<FoundModel> {
    let models = continue_models(yaml);
    let pick = selected.and_then(trimmed_non_empty);
    let item = if let Some(p) = pick {
        models
            .into_iter()
            .find(|x| {
                let key = if x.name.is_empty() {
                    x.model.as_str()
                } else {
                    x.name.as_str()
                };
                key == p
            })
            .unwrap_or_else(|| ContinueModelItem {
                name: p.to_string(),
                model: String::new(),
                chat: true,
            })
    } else {
        models.into_iter().find(|x| x.chat)?
    };
    let name = trimmed_non_empty(&item.name).map(str::to_string);
    let model = match valid_model_id(&item.model) {
        Some(m) if m != "AUTODETECT" => m.to_string(),
        _ => name
            .clone()
            .and_then(|n| valid_model_id(&n).map(str::to_string))?,
    };
    Some(FoundModel {
        model,
        name: name.map(|n| slice_utf16_prefix(&n, MAX_MODEL_NAME_UTF16).to_string()),
        effort: None,
        at: None,
    })
}

/// Parses Crush's `crush.json` / `.crush.json` (`crushConfigModel`).
#[must_use]
pub fn crush_config_model(o: &Value) -> Option<FoundModel> {
    let large = o
        .as_object()?
        .get("models")?
        .as_object()?
        .get("large")?
        .as_object()?;
    let model = large.get("model").and_then(model_id_val)?;
    let effort = large
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .and_then(effort_level)
        .map(str::to_string);
    Some(FoundModel {
        model,
        name: None,
        effort,
        at: None,
    })
}

fn yaml_top_value(yaml: &str, key: &str) -> Option<String> {
    if yaml.is_empty() || yaml.len() > MAX_DOCUMENT_BYTES {
        return None;
    }
    let prefix = format!("{key}:");
    for raw_line in yaml.split('\n').take(MAX_YAML_LINES) {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let Some(after) = line.strip_prefix(&prefix) else {
            continue;
        };
        let mut v = after.trim_start_matches(crate::agent_env::js_space);
        if let Some(hash) = v.find('#') {
            v = &v[..hash];
        }
        v = crate::agent_env::trim(v);
        if let Some(stripped) = v.strip_prefix('"').or_else(|| v.strip_prefix('\'')) {
            v = stripped;
        }
        if let Some(stripped) = v.strip_suffix('"').or_else(|| v.strip_suffix('\'')) {
            v = stripped;
        }
        let clean = crate::agent_env::trim(v);
        if !clean.is_empty() {
            return Some(clean.to_string());
        }
    }
    None
}

/// Parses Goose's `config.yaml` (`gooseConfigModel`).
#[must_use]
pub fn goose_config_model(yaml: &str) -> Option<FoundModel> {
    let raw = yaml_top_value(yaml, "GOOSE_MODEL")?;
    let model = valid_model_id(&raw)?.to_string();
    Some(FoundModel {
        model,
        name: None,
        effort: None,
        at: None,
    })
}

/// Parses Aider's `.aider.conf.yml` (`aiderConfigModel`).
#[must_use]
pub fn aider_config_model(yaml: &str) -> Option<FoundModel> {
    let raw = yaml_top_value(yaml, "model")?;
    let model = valid_model_id(&raw)?.to_string();
    let effort =
        yaml_top_value(yaml, "reasoning-effort").and_then(|e| effort_level(&e).map(str::to_string));
    Some(FoundModel {
        model,
        name: None,
        effort,
        at: None,
    })
}

/// Parses Mistral Vibe's `config.toml` (`vibeConfigModel`).
#[must_use]
pub fn vibe_config_model(toml_text: &str) -> Option<FoundModel> {
    let toml = toml_keys(toml_text);
    let model = toml
        .get("")?
        .get("active_model")?
        .as_str()
        .and_then(valid_model_id)?
        .to_string();
    Some(FoundModel {
        model,
        name: None,
        effort: None,
        at: None,
    })
}

fn grok_effort(v: Option<&Value>) -> Option<String> {
    let s = v.and_then(str_val)?.to_ascii_lowercase();
    GROK_EFFORTS.contains(&s.as_str()).then_some(s)
}

/// Parses Grok Build's session `summary.json` (`grokSummaryModel`).
#[must_use]
pub fn grok_summary_model(o: &Value) -> Option<FoundModel> {
    let obj = o.as_object()?;
    let model = obj.get("current_model_id").and_then(model_id_val)?;
    let effort = grok_effort(obj.get("reasoning_effort"));
    let at = obj
        .get("updated_at")
        .and_then(time_of_value)
        .or_else(|| obj.get("last_active_at").and_then(time_of_value));
    Some(FoundModel {
        model,
        name: None,
        effort,
        at,
    })
}

/// Extracts a Grok model's display name and default effort from `models_cache.json` (`grokCatalogEntry`).
#[must_use]
pub fn grok_catalog_entry(cache: &Value, id: &str) -> Option<GrokCatalogEntry> {
    let info = cache
        .as_object()?
        .get("models")?
        .as_object()?
        .get(id)?
        .as_object()?
        .get("info")?
        .as_object()?;
    let picked_eff = info
        .get("reasoning_efforts")
        .and_then(Value::as_array)
        .and_then(|arr| {
            arr.iter().find_map(|e| {
                let eo = e.as_object()?;
                (eo.get("default") == Some(&Value::Bool(true))).then(|| eo.get("value"))?
            })
        });
    let effort = grok_effort(picked_eff).or_else(|| grok_effort(info.get("reasoning_effort")));
    let name = info
        .get("name")
        .and_then(str_val)
        .map(|n| slice_utf16_prefix(n, MAX_MODEL_NAME_UTF16).to_string());
    Some(GrokCatalogEntry { name, effort })
}

/// Parses Grok Build's `config.toml` `[models]` section (`grokConfigModel`).
#[must_use]
pub fn grok_config_model(toml_text: &str) -> Option<FoundModel> {
    let toml = toml_keys(toml_text);
    let models = toml.get("models")?;
    let model = models
        .get("default")?
        .as_str()
        .and_then(valid_model_id)?
        .to_string();
    let effort = models
        .get("default_reasoning_effort")
        .and_then(TomlScalar::as_str)
        .and_then(trimmed_non_empty)
        .map(str::to_ascii_lowercase)
        .filter(|s| GROK_EFFORTS.contains(&s.as_str()));
    Some(FoundModel {
        model,
        name: None,
        effort,
        at: None,
    })
}

/// Resolves Grok Build settings across `config.toml`, `settings_cache.json`, and `models_cache.json` (`grokSettings`).
#[must_use]
pub fn grok_settings_model(
    config_toml: Option<&str>,
    settings_cache_json: Option<&str>,
    models_cache_json: Option<&str>,
) -> Option<FoundModel> {
    let mut found = config_toml.and_then(grok_config_model);
    if found.is_none()
        && let Some(raw) = settings_cache_json
        && let Some(o) = parse_bounded_json_doc(raw)
    {
        let unwrapped = match o.get("payload").and_then(Value::as_str) {
            Some(payload_str) => parse_bounded_json_doc(payload_str),
            None => Some(o),
        };
        if let Some(model) = unwrapped
            .as_ref()
            .and_then(|u| u.get("settings"))
            .and_then(|s| s.get("default_model"))
            .and_then(model_id_val)
        {
            found = Some(FoundModel {
                model,
                name: None,
                effort: None,
                at: None,
            });
        }
    }
    let mut found = found?;
    if let Some(cache_raw) = models_cache_json
        && let Some(cache_val) = parse_bounded_json_doc(cache_raw)
        && let Some(entry) = grok_catalog_entry(&cache_val, &found.model)
    {
        if found.effort.is_none() {
            found.effort = entry.effort;
        }
        if entry.name.is_some() {
            found.name = entry.name;
        }
    }
    Some(found)
}

/// Extracts the single loaded Ollama model from `/api/ps` JSON (`ollamaRunningModel`).
#[must_use]
pub fn ollama_running_model_from_json(json_text: &str) -> Option<String> {
    let o = parse_strict_object(json_text)?;
    let models = o.get("models")?.as_array()?;
    if models.len() != 1 {
        return None;
    }
    models[0].as_object()?.get("name").and_then(model_id_val)
}

// =============================================================================
// 6. Pure end-to-end `agentModel` & `TerminalPane.vue` `refreshModel` orchestrators
// =============================================================================

fn resolve_settings_for_agent(inputs: &AgentModelInputs<'_>) -> Option<FoundModel> {
    match inputs.agent_id {
        "claude" => claude_settings_model(&inputs.claude_settings_docs).map(|model| FoundModel {
            model,
            name: None,
            effort: None,
            at: None,
        }),
        "openclaude" => inputs
            .openclaude_settings_json
            .as_deref()
            .and_then(openclaude_settings_model),
        "codex" => inputs
            .codex_config_toml
            .as_deref()
            .and_then(codex_model_from_toml),
        "cursor" => inputs
            .cursor_cli_config_json
            .as_deref()
            .and_then(cursor_config_model),
        "gemini" | "qwen" => gemini_or_qwen_settings_model(&inputs.gemini_or_qwen_settings_docs),
        "opencode" => opencode_settings_model(&inputs.opencode_config_docs),
        "copilot" => copilot_settings_model(&inputs.copilot_settings_docs),
        "kimi" => inputs
            .kimi_config_toml
            .as_deref()
            .and_then(kimi_config_model),
        "droid" => inputs
            .droid_settings_json
            .as_deref()
            .and_then(parse_bounded_json_doc)
            .and_then(|v| droid_settings_model(&v)),
        "pi" => inputs
            .pi_settings_json
            .as_deref()
            .and_then(parse_bounded_json_doc)
            .and_then(|v| pi_settings_model(&v)),
        "kilo" => kilo_config_model(&inputs.kilo_config_docs),
        "openclaw" => inputs
            .openclaw_config_json
            .as_deref()
            .and_then(parse_bounded_json_doc)
            .and_then(|v| openclaw_config_model(&v)),
        "continue" => inputs
            .continue_config_yaml
            .as_deref()
            .and_then(|y| continue_model(inputs.continue_selected_model.as_deref(), y)),
        "crush" => inputs.crush_config_docs.iter().find_map(|doc| {
            let v = parse_loose_json(doc)?;
            crush_config_model(&v)
        }),
        "goose" => inputs
            .goose_config_yaml
            .as_deref()
            .and_then(goose_config_model),
        "aider" => inputs
            .aider_config_yamls
            .iter()
            .find_map(|y| aider_config_model(y)),
        "vibe" => inputs
            .vibe_config_tomls
            .iter()
            .find_map(|t| vibe_config_model(t)),
        "grok" => grok_settings_model(
            inputs.grok_config_toml.as_deref(),
            inputs.grok_settings_cache_json.as_deref(),
            inputs.grok_models_cache_json.as_deref(),
        ),
        _ => None,
    }
}

fn resolve_agent_model_found(inputs: &AgentModelInputs<'_>) -> Option<ShownAgentModel> {
    let agent = inputs.agent_id;
    if agent.is_empty() {
        return None;
    }

    if agent == "copilot" {
        let session = inputs.session.clone().or_else(|| {
            if inputs.copilot_chunks_newest_first.is_empty() {
                return None;
            }
            let mut acc: Option<CopilotScan> = None;
            for chunk in inputs.copilot_chunks_newest_first.iter().take(256) {
                acc = copilot_merge(acc, copilot_records(chunk));
                if acc.as_ref().is_some_and(copilot_is_done) {
                    break;
                }
            }
            copilot_finish(acc)
        });
        if let Some(mut m) = session {
            if m.effort.is_none() {
                m.effort = effort_flag(inputs.command);
            }
            return Some(m.into_shown(ModelSource::Session));
        }
    }

    if agent == "cline" {
        let live = inputs.session.clone();
        let set = inputs
            .cline_providers_doc
            .as_ref()
            .and_then(|d| cline_settings_model(&d.text, d.mtime_ms));
        let effort_of = |model: &str| -> Option<String> {
            set.as_ref()
                .filter(|s| s.model == model)
                .and_then(|s| s.effort.clone())
        };
        if let Some(live) = live
            && !set
                .as_ref()
                .is_some_and(|s| s.mtime_ms > live.at.unwrap_or(0))
        {
            return Some(ShownAgentModel {
                model: live.model.clone(),
                name: None,
                effort: effort_of(&live.model),
                chosen_effort: None,
                source: ModelSource::Session,
                at: None,
            });
        }
        if let Some(flag) = model_from_command(inputs.command) {
            let eff = effort_flag(inputs.command).or_else(|| effort_of(&flag));
            return Some(ShownAgentModel {
                model: flag,
                name: None,
                effort: eff,
                chosen_effort: None,
                source: ModelSource::Command,
                at: None,
            });
        }
        return set.map(|s| ShownAgentModel {
            model: s.model,
            name: None,
            effort: s.effort,
            chosen_effort: None,
            source: ModelSource::Settings,
            at: None,
        });
    }

    if agent == "antigravity" {
        if let Some(session) = inputs.session.clone() {
            return Some(session.into_shown(ModelSource::Session));
        }
        let flag = model_from_command(inputs.command);
        let flag_effort = effort_from_command(inputs.command);
        let saved = inputs
            .antigravity_settings_json
            .as_deref()
            .and_then(antigravity_settings_model);
        if let Some(flag) = flag {
            return Some(ShownAgentModel {
                model: flag,
                name: None,
                effort: flag_effort,
                chosen_effort: None,
                source: ModelSource::Command,
                at: None,
            });
        }
        if let (Some(fe), Some(saved_ref)) = (flag_effort, saved.as_ref()) {
            let saved_name = saved_ref
                .name
                .clone()
                .unwrap_or_else(|| saved_ref.model.clone());
            return Some(ShownAgentModel {
                model: saved_name.clone(),
                name: Some(saved_name),
                effort: Some(fe),
                chosen_effort: None,
                source: ModelSource::Command,
                at: None,
            });
        }
        return saved.map(|s| s.into_shown(ModelSource::Settings));
    }

    if agent == "muse" {
        let flag_effort = muse_effort_from_command(inputs.command);
        let saved = inputs
            .muse_settings_json
            .as_deref()
            .and_then(muse_settings_model);
        let saved_for = |id: &str| -> Option<String> {
            saved
                .as_ref()
                .and_then(|s| s.effort_for(id).map(str::to_string))
        };
        if let Some(mut session) = inputs.session.clone() {
            session.effort = session
                .effort
                .or_else(|| flag_effort.clone())
                .or_else(|| saved_for(&session.model));
            return Some(session.into_shown(ModelSource::Session));
        }
        if let Some(flag) = model_from_command(inputs.command) {
            let eff = flag_effort.or_else(|| saved_for(&flag));
            return Some(ShownAgentModel {
                model: flag,
                name: None,
                effort: eff,
                chosen_effort: None,
                source: ModelSource::Command,
                at: None,
            });
        }
        if let Some(saved) = saved
            && let Some(model) = saved.model
        {
            return Some(ShownAgentModel {
                model,
                name: None,
                effort: flag_effort.or(saved.effort),
                chosen_effort: None,
                source: ModelSource::Settings,
                at: None,
            });
        }
        return None;
    }

    if let Some(mut turn) = inputs.session.clone() {
        match agent {
            "claude" => {
                let set = claude_settings_model(&inputs.claude_settings_docs);
                let big_family = set.as_deref().and_then(|sm| {
                    let base = strip_1m_suffix(sm);
                    (base.len() != sm.len() && base.bytes().all(is_word_byte))
                        .then(|| base.to_ascii_lowercase())
                });
                let base_m = turn.model.clone();
                if let Some(fam) = big_family
                    && base_m.to_ascii_lowercase().contains(&fam)
                    && strip_1m_suffix(&turn.model).len() == turn.model.len()
                {
                    turn.model.push_str("[1m]");
                }
                if turn.effort.is_none() {
                    turn.effort =
                        claude_settings_effort(Some(&base_m), &inputs.claude_settings_docs);
                }
                return Some(turn.into_shown(ModelSource::Session));
            }
            "openclaude" | "qoder" => {
                if turn.effort.is_none() {
                    turn.effort = effort_flag(inputs.command);
                }
                return Some(turn.into_shown(ModelSource::Session));
            }
            "codex" => {
                return Some(turn.into_shown(ModelSource::Session));
            }
            _ => {}
        }
    }

    let flag_effort = effort_flag(inputs.command);
    let picked = if agent == "opencode" || agent == "kilo" {
        inputs.picked.clone()
    } else {
        None
    };

    if !matches!(
        agent,
        "copilot" | "cline" | "antigravity" | "muse" | "claude" | "openclaude" | "qoder" | "codex"
    ) && let Some(mut session) = inputs.session.clone()
    {
        if agent == "kimi"
            && session.name.is_none()
            && let Some(ref toml_raw) = inputs.kimi_config_toml
        {
            session.name = kimi_display_name(&toml_keys(toml_raw), &session.model);
        } else if agent == "grok"
            && session.name.is_none()
            && let Some(ref cache_raw) = inputs.grok_models_cache_json
            && let Some(cache_val) = parse_bounded_json_doc(cache_raw)
            && let Some(entry) = grok_catalog_entry(&cache_val, &session.model)
        {
            session.name = entry.name;
        }
        let superseded_by_picked = picked
            .as_ref()
            .is_some_and(|p| session.at.is_some_and(|at| p.mtime_ms > at + 1000));
        if !superseded_by_picked {
            if session.effort.is_none() {
                session.effort.clone_from(&flag_effort);
            }
            return Some(session.into_shown(ModelSource::Session));
        }
    }

    if let Some(ref p) = picked
        && inputs
            .launched_at_ms
            .is_some_and(|launched| launched > 0 && p.mtime_ms >= launched)
    {
        return Some(ShownAgentModel {
            model: p.model.clone(),
            name: None,
            effort: p.effort.clone(),
            chosen_effort: None,
            source: ModelSource::Picked,
            at: None,
        });
    }

    let flag = if agent == "amp" {
        amp_mode_from_command(inputs.command)
    } else {
        None
    }
    .or_else(|| model_from_command(inputs.command));

    if let Some(flag) = flag {
        return Some(ShownAgentModel {
            model: flag,
            name: None,
            effort: flag_effort,
            chosen_effort: None,
            source: ModelSource::Command,
            at: None,
        });
    }

    if let Some(s) = resolve_settings_for_agent(inputs) {
        let effort = flag_effort.or(s.effort).or_else(|| {
            (agent == "claude")
                .then(|| claude_settings_effort(Some(&s.model), &inputs.claude_settings_docs))
                .flatten()
        });
        return Some(ShownAgentModel {
            model: s.model,
            name: s.name,
            effort,
            chosen_effort: None,
            source: ModelSource::Settings,
            at: None,
        });
    }

    if let Some(p) = picked {
        return Some(ShownAgentModel {
            model: p.model,
            name: None,
            effort: p.effort,
            chosen_effort: None,
            source: ModelSource::Picked,
            at: None,
        });
    }

    if agent == "ollama"
        && let Some(running) = inputs
            .ollama_ps_json
            .as_deref()
            .and_then(ollama_running_model_from_json)
    {
        return Some(ShownAgentModel {
            model: running,
            name: None,
            effort: None,
            chosen_effort: None,
            source: ModelSource::Running,
            at: None,
        });
    }

    None
}

/// Pure orchestrator matching `agentModel.js`'s `agentModel` + `agentModelLive` priority order:
/// `session` -> `picked` (when newer than launch/session) -> `command` -> `settings` -> `picked` -> `running`.
#[must_use]
pub fn resolve_agent_model(inputs: &AgentModelInputs<'_>) -> AgentModelResolution {
    let mut found = resolve_agent_model_found(inputs);
    let chosen = inputs.chosen_model.filter(|s| is_claude_model_token(s));

    if inputs.agent_id != "claude" {
        return AgentModelResolution {
            found,
            chosen_effort: None,
        };
    }

    let flag_effort = effort_from_command(inputs.command);
    if let Some(ref mut f) = found
        && f.effort.is_none()
    {
        f.effort = flag_effort
            .clone()
            .or_else(|| claude_settings_effort(Some(&f.model), &inputs.claude_settings_docs));
    }

    if let Some(chosen_id) = chosen {
        let same_model = found.as_ref().is_some_and(|f| {
            f.source == ModelSource::Session && same_claude_model(&f.model, chosen_id)
        });
        let chosen_effort = (if same_model {
            found.as_ref().and_then(|f| f.effort.clone())
        } else {
            None
        })
        .or(flag_effort)
        .or_else(|| claude_settings_effort(Some(chosen_id), &inputs.claude_settings_docs));

        if let Some(ref mut f) = found {
            f.chosen_effort.clone_from(&chosen_effort);
        }
        return AgentModelResolution {
            found,
            chosen_effort,
        };
    }

    AgentModelResolution {
        found,
        chosen_effort: None,
    }
}

fn tail_slice<T>(slice: &[T], count: usize) -> &[T] {
    let start = slice.len().saturating_sub(count);
    &slice[start..]
}

fn session_model_from_poll(
    poll: &AgentModelPollOutcome,
    current_session_model: Option<String>,
) -> Option<String> {
    match poll {
        AgentModelPollOutcome::Keep => current_session_model,
        AgentModelPollOutcome::Resolved(r) => r
            .found
            .as_ref()
            .filter(|f| !f.model.is_empty() && f.source != ModelSource::Screen)
            .map(|f| f.model.clone()),
    }
}

fn claude_switched_on_screen(
    has_term: bool,
    agent_id: &str,
    scrollback_lines: &[String],
) -> Option<String> {
    if !has_term || agent_id != "claude" {
        return None;
    }
    for line in tail_slice(scrollback_lines, MAX_SCROLLBACK_SWITCH_LINES)
        .iter()
        .rev()
    {
        if let Some(m) = model_from_switch_line(line) {
            return Some(m);
        }
    }
    None
}

fn shown_to_save(shown: Option<&ShownAgentModel>) -> Option<SavedPaneModel> {
    let val = serde_json::to_value(shown?).ok()?;
    pane_model_to_save(Some(&val))
}

/// Pure pane header model orchestrator matching `TerminalPane.vue`'s `refreshModel`:
/// - Cursor / Antigravity / Codex footer fast-paths clear optimistic `modelChoice` while
///   keeping `session_model` strictly file-backed (`source != Screen`) for Live Share.
/// - Cursor open `/model` picker freezes the previous shown chip (`froze_on_picker: true`).
/// - `AgentModelPollOutcome::Keep` preserves any existing shown model on transient failure,
///   or falls through to screen detection when nothing was shown yet.
/// - Applies `antigravity_listed`, non-`OWN_MODEL_NAMES` catalog labels, `muse_footer_model`,
///   `model_from_screen`, Claude scrollback `/model` switches over `Settings`,
///   `resolve_chosen_shown`, `with_model_version`, and `pane_model_to_save`.
#[must_use]
pub fn resolve_pane_header_model(inputs: &PaneHeaderModelInputs<'_>) -> PaneHeaderModelResolution {
    let mut node = inputs.node.clone();
    let agent_id = node.agent_id.clone();
    let current_shown = inputs.current_shown.clone();
    let mut screen_context = inputs.current_screen_context;

    if agent_id == "cursor" && inputs.has_term {
        let lines = tail_slice(inputs.screen_lines, 8);
        if let Some(ctx) = cursor_context_on_screen(lines, inputs.catalog_models) {
            screen_context = if ctx == CursorContextOnScreen::NoneYet {
                None
            } else {
                Some(ctx)
            };
        }
        if cursor_picker_shown(lines) && current_shown.is_some() {
            let to_save = shown_to_save(current_shown.as_ref());
            return PaneHeaderModelResolution {
                shown: current_shown,
                session_model: inputs.current_session_model.clone(),
                screen_context,
                model_choice: node.model_choice,
                to_save,
                froze_on_picker: true,
            };
        }
        if let Some(seen) = cursor_model_on_screen(lines, inputs.catalog_models) {
            node.model_choice = None;
            let shown = ShownAgentModel {
                model: seen.model,
                name: (!seen.name.is_empty()).then_some(seen.name),
                effort: seen.effort,
                chosen_effort: None,
                source: ModelSource::Screen,
                at: None,
            };
            let to_save = shown_to_save(Some(&shown));
            return PaneHeaderModelResolution {
                shown: Some(shown),
                session_model: session_model_from_poll(
                    &inputs.poll,
                    inputs.current_session_model.clone(),
                ),
                screen_context,
                model_choice: node.model_choice,
                to_save,
                froze_on_picker: false,
            };
        }
    }

    if agent_id == "antigravity"
        && inputs.has_term
        && let Some(seen) =
            antigravity_model_on_screen(tail_slice(inputs.screen_lines, 6), inputs.catalog_models)
    {
        node.model_choice = None;
        let shown = ShownAgentModel {
            model: seen.model,
            name: seen.name,
            effort: seen.effort,
            chosen_effort: None,
            source: ModelSource::Screen,
            at: None,
        };
        let to_save = shown_to_save(Some(&shown));
        return PaneHeaderModelResolution {
            shown: Some(shown),
            session_model: session_model_from_poll(
                &inputs.poll,
                inputs.current_session_model.clone(),
            ),
            screen_context,
            model_choice: node.model_choice,
            to_save,
            froze_on_picker: false,
        };
    }

    if agent_id == "codex"
        && inputs.has_term
        && let Some(seen) =
            codex_model_on_screen(tail_slice(inputs.screen_lines, 12), inputs.catalog_models)
    {
        node.model_choice = None;
        let shown = ShownAgentModel {
            model: seen.model,
            name: seen.name,
            effort: seen.effort,
            chosen_effort: None,
            source: ModelSource::Screen,
            at: None,
        };
        let to_save = shown_to_save(Some(&shown));
        return PaneHeaderModelResolution {
            shown: Some(shown),
            session_model: session_model_from_poll(
                &inputs.poll,
                inputs.current_session_model.clone(),
            ),
            screen_context,
            model_choice: node.model_choice,
            to_save,
            froze_on_picker: false,
        };
    }

    let (mut res, mut loose_chosen_effort) = match &inputs.poll {
        AgentModelPollOutcome::Keep => {
            if current_shown.as_ref().is_some_and(|m| !m.model.is_empty()) {
                let to_save = shown_to_save(current_shown.as_ref());
                return PaneHeaderModelResolution {
                    shown: current_shown,
                    session_model: inputs.current_session_model.clone(),
                    screen_context,
                    model_choice: node.model_choice,
                    to_save,
                    froze_on_picker: false,
                };
            }
            (None, None)
        }
        AgentModelPollOutcome::Resolved(r) => (r.found.clone(), r.chosen_effort.clone()),
    };

    let session_model = res
        .as_ref()
        .filter(|f| !f.model.is_empty() && f.source != ModelSource::Screen)
        .map(|f| f.model.clone());

    if agent_id == "antigravity"
        && let Some(ref mut f) = res
        && !f.model.is_empty()
        && let Some(listed) =
            antigravity_listed(inputs.catalog_models, &f.model, f.effort.as_deref())
    {
        f.model = listed.model;
        if listed.name.is_some() {
            f.name = listed.name;
        }
        f.effort = listed.effort;
    }

    if let Some(ref mut f) = res
        && !f.model.is_empty()
        && f.name.is_none()
        && !OWN_MODEL_NAMES.contains(&agent_id.as_str())
    {
        let found_label = inputs.catalog_models.iter().find_map(|row| {
            let obj = row.as_object()?;
            if obj.get("id").and_then(Value::as_str) != Some(f.model.as_str()) {
                return None;
            }
            let lbl = obj.get("label").and_then(str_val)?;
            (lbl != f.model).then(|| lbl.to_string())
        });
        if let Some(lbl) = found_label {
            f.name = Some(lbl);
        }
    }

    if agent_id == "muse"
        && inputs.has_term
        && let Some(MuseFooterModel {
            model: foot_model,
            effort: foot_effort,
        }) = muse_footer_model(tail_slice(inputs.screen_lines, 6))
    {
        if res.as_ref().is_none_or(|f| f.model.is_empty()) && node.model_choice.is_none() {
            res = Some(ShownAgentModel {
                model: foot_model,
                name: None,
                effort: Some(foot_effort),
                chosen_effort: None,
                source: ModelSource::Screen,
                at: None,
            });
        } else if let Some(ref mut f) = res
            && f.model == foot_model
            && f.effort.is_none()
        {
            f.effort = Some(foot_effort);
        }
    }

    if res.as_ref().is_none_or(|f| f.model.is_empty())
        && node.model_choice.is_none()
        && inputs.has_term
    {
        let bottom = tail_slice(inputs.screen_lines, 6);
        let top_len = inputs.top_screen_lines.len().min(30);
        let top = &inputs.top_screen_lines[..top_len];
        if let Some(seen) = model_from_screen(bottom, top) {
            let eff = res
                .as_ref()
                .and_then(|f| f.chosen_effort.clone())
                .or_else(|| loose_chosen_effort.take());
            res = Some(ShownAgentModel {
                model: seen,
                name: None,
                effort: eff,
                chosen_effort: None,
                source: ModelSource::Screen,
                at: None,
            });
        }
    }

    let switched = claude_switched_on_screen(inputs.has_term, &agent_id, inputs.scrollback_lines);
    if res
        .as_ref()
        .is_some_and(|f| f.source == ModelSource::Settings)
        && node.model_choice.is_none()
        && let Some(ref sw) = switched
    {
        let prev_effort = res.as_ref().and_then(|f| f.effort.clone());
        res = Some(ShownAgentModel {
            model: sw.clone(),
            name: None,
            effort: prev_effort,
            chosen_effort: None,
            source: ModelSource::Screen,
            at: None,
        });
    }

    let has_chosen_model = node
        .session_options
        .as_ref()
        .and_then(|o| o.get("model"))
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
        || node
            .model_choice
            .as_ref()
            .is_some_and(|c| !c.model.is_empty());

    let synthetic_chosen_effort = if res.is_none() && has_chosen_model {
        loose_chosen_effort.map(|ce| ShownAgentModel {
            model: String::new(),
            name: None,
            effort: None,
            chosen_effort: Some(ce),
            source: ModelSource::Settings,
            at: None,
        })
    } else {
        None
    };

    let observed_ref = res.as_ref().or(synthetic_chosen_effort.as_ref());
    let remote_host = node
        .remote_host_id
        .as_deref()
        .is_some_and(|s| !s.is_empty());
    let outcome = resolve_chosen_shown(
        node.session_options.as_ref(),
        node.model_choice.as_ref(),
        remote_host,
        observed_ref,
        inputs.catalog_models,
    );
    node.model_choice = outcome.model_choice;
    let mut shown = outcome.shown;
    if agent_id == "claude"
        && let Some(ref s) = shown
    {
        let mut reported: Vec<&str> = Vec::with_capacity(2);
        if let Some(m) = res
            .as_ref()
            .filter(|f| f.source == ModelSource::Session && !f.model.is_empty())
            .map(|f| f.model.as_str())
        {
            reported.push(m);
        }
        if let Some(ref sw) = switched {
            reported.push(sw.as_str());
        }
        shown = Some(with_model_version(s, &reported));
    }

    let to_save = shown_to_save(shown.as_ref());
    PaneHeaderModelResolution {
        shown,
        session_model,
        screen_context,
        model_choice: node.model_choice,
        to_save,
        froze_on_picker: false,
    }
}
