//! Differential, stream-budget, UTF-16 boundary, JSON-depth, and redaction tests
//! for `src/agent_model_readers.rs` against the reference fixture generated from
//! Tessel's Vue/JS modules (`tests/fixtures/agent_model_readers.json`).

#[allow(dead_code)]
#[path = "../src/agent_env.rs"]
mod agent_env;

#[allow(dead_code)]
#[path = "../src/agent_arg_options.rs"]
mod agent_arg_options;

#[allow(dead_code)]
#[path = "../src/agent_model_antigravity.rs"]
mod agent_model_antigravity;

#[allow(dead_code)]
#[path = "../src/agent_model_catalog.rs"]
mod agent_model_catalog;

#[allow(dead_code)]
mod workspace {
    use serde::{Deserialize, Serialize};
    use std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
    };

    pub type Result<T> = std::result::Result<T, String>;

    #[derive(Clone, Debug, Default, Serialize, Deserialize)]
    pub struct Workspace {
        #[serde(default)]
        pub agent_model_defaults: BTreeMap<String, crate::agent_model_defaults::Defaults>,
    }

    pub struct Store {
        file: PathBuf,
    }

    impl Store {
        pub fn open(dir: &Path) -> Result<(Self, Workspace)> {
            let file = dir.join("workspace.json");
            let state = match fs::read(&file) {
                Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
                Err(_) => Workspace::default(),
            };
            Ok((Self { file }, state))
        }

        pub fn save(&self, state: &Workspace) -> Result<()> {
            let bytes = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
            fs::write(&self.file, bytes).map_err(|e| e.to_string())
        }
    }
}

#[allow(dead_code)]
#[path = "../src/agent_model_defaults.rs"]
mod agent_model_defaults;

#[allow(dead_code)]
#[path = "../src/agent_model_launch.rs"]
mod agent_model_launch;

#[allow(dead_code)]
#[path = "../src/agent_model_defaults_ui.rs"]
mod agent_model_defaults_ui;

#[allow(dead_code)]
#[path = "../src/pane_session_model.rs"]
pub mod pane_session_model;

#[path = "../src/agent_model_readers.rs"]
pub mod agent_model_readers;

use agent_model_readers::{
    AgentModelInputs, AgentModelPollOutcome, AgentModelResolution, ClineSessionRow, CommandLine,
    FoundModel, GrokCatalogEntry, MAX_COMMAND_UTF16, MAX_DOCUMENT_BYTES, MAX_JSON_DEPTH,
    MAX_JSONL_LINE_BYTES, MAX_MODEL_ID_UTF16, ModelTimelinePoint, OpenCodeMessageRow,
    PaneHeaderModelInputs, PickedModel, TimedDocument, aider_config_model, amp_mode_from_command,
    antigravity_model_from_text, antigravity_model_timeline, antigravity_settings_model,
    claude_model_from_text, claude_settings_effort, claude_settings_model, claude_turn_from_text,
    cline_session_model, cline_settings_model, codex_model_from_text, codex_model_from_toml,
    continue_model, continue_models, copilot_finish, copilot_is_done, copilot_merge,
    copilot_model_from_text, copilot_records, copilot_settings_model, crush_config_model,
    cursor_config_model, droid_settings_model, effort_flag, effort_from_command,
    gemini_model_from_text, gemini_or_qwen_settings_model, goose_config_model, grok_catalog_entry,
    grok_config_model, grok_settings_model, grok_summary_model, kilo_config_model,
    kimi_config_model, kimi_display_name, kimi_model_from_wire, kimi_wire_model,
    model_from_command, muse_effort_from_command, muse_model_from_log, muse_model_from_text,
    muse_model_timeline, muse_settings_model, muse_time_ms, norm_dir,
    ollama_running_model_from_json, openclaude_settings_model, openclaw_config_model,
    openclaw_index_model, opencode_message_model, opencode_picked_model, opencode_session_column,
    opencode_session_model, opencode_settings_model, parse_iso_ms, parse_loose_json,
    pi_model_from_text, pi_model_from_wire, pi_model_timeline, pi_settings_model,
    qwen_model_from_text, resolve_agent_model, resolve_pane_header_model, same_claude_model,
    time_of_value, toml_keys, uuid_v7_time_ms, valid_model_id, vibe_config_model,
    within_json_depth,
};
use pane_session_model::{
    CursorContextOnScreen, ModelSource, OptimisticModelChoice, PaneModelNode, SavedPaneModel,
    ShownAgentModel,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureRoot {
    sources: Vec<SourceEntry>,
    catalogs: BTreeMap<String, Vec<Value>>,
    claude_turn_cases: Vec<ClaudeTurnCase>,
    codex_turn_cases: Vec<TextExpectedCase>,
    uuid_v7_cases: Vec<UuidV7Case>,
    copilot_cases: Vec<TextExpectedCase>,
    antigravity_cases: Vec<TextExpectedCase>,
    antigravity_timeline: LinesTimelineCase,
    muse_cases: Vec<WireExpectedCase>,
    muse_timeline: TextTimelineCase,
    muse_settings_cases: Vec<MuseSettingsCase>,
    gemini_cases: Vec<TextExpectedCase>,
    qwen_cases: Vec<TextExpectedCase>,
    kimi_wire_cases: Vec<WireExpectedCase>,
    kimi_config_cases: Vec<TomlExpectedCase>,
    pi_wire_cases: Vec<WireExpectedCase>,
    pi_timeline: LinesTimelineCase,
    opencode_session_cases: Vec<OpenCodeSessionCase>,
    cline_cases: Vec<ClineCase>,
    continue_cases: Vec<ContinueCase>,
    misc_config_cases: MiscConfigCases,
    command_cases: Vec<CommandCase>,
    same_claude_cases: Vec<SameClaudeCase>,
    codex_toml_cases: Vec<TomlExpectedCase>,
    cursor_config_cases: Vec<TextExpectedCase>,
    claude_settings_effort_cases: Vec<ClaudeSettingsEffortCase>,
    agent_model_cases: Vec<AgentModelCase>,
    header_refresh_cases: Vec<HeaderRefreshCase>,
}

#[derive(Debug, Deserialize)]
struct SourceEntry {
    file: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeTurnCase {
    name: String,
    text: String,
    expected_turn: Option<FoundModel>,
    expected_model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TextExpectedCase {
    name: String,
    text: String,
    expected: Option<FoundModel>,
}

#[derive(Debug, Deserialize)]
struct TomlExpectedCase {
    name: String,
    toml: String,
    expected: Option<FoundModel>,
}

#[derive(Debug, Deserialize)]
struct WireExpectedCase {
    name: String,
    tail: String,
    head: String,
    expected: Option<FoundModel>,
}

#[derive(Debug, Deserialize)]
struct UuidV7Case {
    id: String,
    expected: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct LinesTimelineCase {
    lines: Vec<String>,
    expected: Vec<ModelTimelinePoint>,
}

#[derive(Debug, Deserialize)]
struct TextTimelineCase {
    text: String,
    expected: Vec<ModelTimelinePoint>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MuseSettingsExpected {
    model: Option<String>,
    effort: Option<String>,
    efforts_by_probe: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MuseSettingsCase {
    name: String,
    json: String,
    probe_ids: Vec<String>,
    expected: Option<MuseSettingsExpected>,
}

#[derive(Debug, Deserialize)]
struct OpenCodeSessionCase {
    name: String,
    column: Option<String>,
    rows: Vec<OpenCodeMessageRow>,
    updated: Option<i64>,
    expected: Option<FoundModel>,
}

#[derive(Debug, Deserialize)]
struct ClineCase {
    name: String,
    cwd: String,
    expected: Option<FoundModel>,
}

#[derive(Debug, Deserialize)]
struct ContinueCase {
    name: String,
    selected: Option<String>,
    yaml: String,
    expected: Option<FoundModel>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MiscConfigCases {
    pi_settings: Option<FoundModel>,
    openclaw_string: Option<FoundModel>,
    openclaw_primary: Option<FoundModel>,
    droid_settings: Option<FoundModel>,
    crush_config: Option<FoundModel>,
    goose_config: Option<FoundModel>,
    aider_config: Option<FoundModel>,
    vibe_config: Option<FoundModel>,
    grok_summary: Option<FoundModel>,
    grok_catalog: Option<GrokCatalogEntry>,
    grok_config: Option<FoundModel>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommandCase {
    command: String,
    model: Option<String>,
    claude_effort: Option<String>,
    muse_effort: Option<String>,
    effort_flag: Option<String>,
    amp_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SameClaudeCase {
    a: String,
    b: String,
    expected: bool,
}

#[derive(Debug, Deserialize)]
struct ClaudeSettingsEffortCase {
    model: Option<String>,
    docs: Vec<String>,
    expected: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FixtureAgentModelInputs {
    agent_id: String,
    session: Option<FoundModel>,
    copilot_chunks_newest_first: Vec<String>,
    command: String,
    chosen_model: Option<String>,
    launched_at_ms: Option<i64>,
    picked: Option<PickedModel>,
    claude_settings_docs: Vec<String>,
    openclaude_settings_json: Option<String>,
    codex_config_toml: Option<String>,
    cursor_cli_config_json: Option<String>,
    gemini_or_qwen_settings_docs: Vec<String>,
    opencode_config_docs: Vec<String>,
    kilo_config_docs: Vec<String>,
    copilot_settings_docs: Vec<String>,
    antigravity_settings_json: Option<String>,
    muse_settings_json: Option<String>,
    kimi_config_toml: Option<String>,
    pi_settings_json: Option<String>,
    openclaw_config_json: Option<String>,
    droid_settings_json: Option<String>,
    cline_providers_doc: Option<TimedDocument>,
    continue_selected_model: Option<String>,
    continue_config_yaml: Option<String>,
    crush_config_docs: Vec<String>,
    goose_config_yaml: Option<String>,
    aider_config_yamls: Vec<String>,
    vibe_config_tomls: Vec<String>,
    grok_config_toml: Option<String>,
    grok_settings_cache_json: Option<String>,
    grok_models_cache_json: Option<String>,
    ollama_ps_json: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentModelCase {
    name: String,
    inputs: FixtureAgentModelInputs,
    expected: Option<AgentModelResolution>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeaderScenarioFixture {
    name: String,
    catalog_key: String,
    node: PaneModelNode,
    #[serde(default)]
    current_agent_model: Option<ShownAgentModel>,
    #[serde(default)]
    current_session_model: Option<String>,
    #[serde(default)]
    current_screen_context: Option<Value>,
    #[serde(default)]
    has_term: Option<bool>,
    #[serde(default)]
    screen_lines: Vec<String>,
    #[serde(default)]
    top_screen_lines: Vec<String>,
    #[serde(default)]
    scrollback_lines: Vec<String>,
    #[serde(default)]
    agent_model_result: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeaderExpectedFixture {
    agent_model: Option<ShownAgentModel>,
    session_model: Option<String>,
    screen_context: Option<Value>,
    model_choice: Option<OptimisticModelChoice>,
    to_save: Option<SavedPaneModel>,
    froze_on_picker: bool,
}

#[derive(Debug, Deserialize)]
struct HeaderRefreshCase {
    scenario: HeaderScenarioFixture,
    expected: HeaderExpectedFixture,
}

fn fixture() -> &'static FixtureRoot {
    static FIXTURE: OnceLock<FixtureRoot> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let raw = include_str!("fixtures/agent_model_readers.json");
        serde_json::from_str(raw).expect("parse tests/fixtures/agent_model_readers.json")
    })
}

fn parse_screen_context_fixture(val: Option<&Value>) -> Option<CursorContextOnScreen> {
    let obj = val?.as_object()?;
    if obj.get("none") == Some(&Value::Bool(true)) {
        return Some(CursorContextOnScreen::NoneYet);
    }
    let used = obj.get("usedTokens").and_then(Value::as_u64).unwrap_or(0);
    let window = obj
        .get("windowTokens")
        .and_then(Value::as_u64)
        .unwrap_or(272_000);
    let pct = obj.get("percentage").and_then(Value::as_u64).unwrap_or(0);
    Some(CursorContextOnScreen::Usage {
        used_tokens: used,
        window_tokens: window,
        percentage: pct,
    })
}

fn parse_poll_outcome_fixture(val: Option<&Value>) -> AgentModelPollOutcome {
    let Some(obj) = val.and_then(Value::as_object) else {
        return AgentModelPollOutcome::Resolved(AgentModelResolution::default());
    };
    if obj.get("keep") == Some(&Value::Bool(true)) {
        return AgentModelPollOutcome::Keep;
    }
    let chosen_effort = obj
        .get("chosenEffort")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let has_model = obj
        .get("model")
        .and_then(Value::as_str)
        .is_some_and(|m| !m.is_empty());
    let found = if has_model {
        Some(
            serde_json::from_value::<ShownAgentModel>(Value::Object(obj.clone()))
                .expect("valid ShownAgentModel in fixture"),
        )
    } else {
        None
    };
    AgentModelPollOutcome::Resolved(AgentModelResolution {
        found,
        chosen_effort,
    })
}

#[test]
fn fixture_provenance_hashes_are_recorded() {
    let f = fixture();
    assert!(
        f.sources.len() >= 15,
        "expected >= 15 upstream source files"
    );
    for src in &f.sources {
        assert!(!src.file.is_empty());
        assert_eq!(src.sha256.len(), 64, "invalid sha256 for {}", src.file);
        assert!(src.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}

#[test]
fn differential_claude_codex_copilot_antigravity_and_muse_streams() {
    let f = fixture();

    for c in &f.claude_turn_cases {
        assert_eq!(
            claude_turn_from_text(&c.text),
            c.expected_turn,
            "claude_turn_from_text mismatch on {}",
            c.name
        );
        assert_eq!(
            claude_model_from_text(&c.text),
            c.expected_model,
            "claude_model_from_text mismatch on {}",
            c.name
        );
    }

    for c in &f.codex_turn_cases {
        assert_eq!(
            codex_model_from_text(&c.text),
            c.expected,
            "codex_model_from_text mismatch on {}",
            c.name
        );
    }

    for u in &f.uuid_v7_cases {
        assert_eq!(
            uuid_v7_time_ms(&u.id),
            u.expected,
            "uuid_v7_time_ms mismatch on {}",
            u.id
        );
    }

    for c in &f.copilot_cases {
        assert_eq!(
            copilot_model_from_text(&c.text),
            c.expected,
            "copilot_model_from_text mismatch on {}",
            c.name
        );
    }

    for c in &f.antigravity_cases {
        assert_eq!(
            antigravity_model_from_text(&c.text),
            c.expected,
            "antigravity_model_from_text mismatch on {}",
            c.name
        );
    }
    assert_eq!(
        antigravity_model_timeline(&f.antigravity_timeline.lines),
        f.antigravity_timeline.expected
    );

    for c in &f.muse_cases {
        if c.head.is_empty() {
            assert_eq!(
                muse_model_from_text(&c.tail),
                c.expected,
                "muse_model_from_text mismatch on {}",
                c.name
            );
        }
        assert_eq!(
            muse_model_from_log(&c.tail, &c.head),
            c.expected,
            "muse_model_from_log mismatch on {}",
            c.name
        );
    }
    assert_eq!(
        muse_model_timeline(&f.muse_timeline.text),
        f.muse_timeline.expected
    );

    for c in &f.muse_settings_cases {
        let parsed = muse_settings_model(&c.json);
        match (&parsed, &c.expected) {
            (None, None) => {}
            (Some(actual), Some(exp)) => {
                assert_eq!(actual.model, exp.model, "muse settings model on {}", c.name);
                assert_eq!(
                    actual.effort, exp.effort,
                    "muse settings effort on {}",
                    c.name
                );
                for id in &c.probe_ids {
                    let want = exp.efforts_by_probe.get(id).and_then(Option::as_deref);
                    assert_eq!(
                        actual.effort_for(id),
                        want,
                        "muse effort_for({id}) mismatch on {}",
                        c.name
                    );
                }
            }
            _ => panic!(
                "muse_settings_model presence mismatch on {}: actual={parsed:?}, expected={:?}",
                c.name, c.expected
            ),
        }
    }
}

#[test]
fn differential_other_agent_sessions_configs_commands_and_efforts() {
    let f = fixture();

    for c in &f.gemini_cases {
        assert_eq!(
            gemini_model_from_text(&c.text),
            c.expected,
            "gemini mismatch on {}",
            c.name
        );
    }
    for c in &f.qwen_cases {
        assert_eq!(
            qwen_model_from_text(&c.text),
            c.expected,
            "qwen mismatch on {}",
            c.name
        );
    }
    for c in &f.kimi_wire_cases {
        assert_eq!(
            kimi_model_from_wire(&c.tail, &c.head),
            c.expected,
            "kimi wire mismatch on {}",
            c.name
        );
    }
    for c in &f.kimi_config_cases {
        assert_eq!(
            kimi_config_model(&c.toml),
            c.expected,
            "kimi config mismatch on {}",
            c.name
        );
    }
    for c in &f.pi_wire_cases {
        if c.head.is_empty() {
            assert_eq!(
                pi_model_from_text(&c.tail),
                c.expected,
                "pi text mismatch on {}",
                c.name
            );
        }
        assert_eq!(
            pi_model_from_wire(&c.tail, &c.head),
            c.expected,
            "pi wire mismatch on {}",
            c.name
        );
    }
    assert_eq!(
        pi_model_timeline(&f.pi_timeline.lines),
        f.pi_timeline.expected
    );

    for c in &f.opencode_session_cases {
        assert_eq!(
            opencode_session_model(c.column.as_deref(), &c.rows, c.updated),
            c.expected,
            "opencode_session_model mismatch on {}",
            c.name
        );
    }

    let cline_rows = vec![
        ClineSessionRow {
            pid: 999,
            model: Some("gone-model".into()),
            cwd: Some("C:\\Proj".into()),
            ended_at: None,
            is_subagent: false,
            updated_at: Some("2026-09-26T19:30:00Z".into()),
        },
        ClineSessionRow {
            pid: 200,
            model: Some("other-folder".into()),
            cwd: Some("C:\\Else".into()),
            ended_at: None,
            is_subagent: false,
            updated_at: Some("2026-09-26T19:29:00Z".into()),
        },
        ClineSessionRow {
            pid: 300,
            model: Some("deepseek/deepseek-v4-flash".into()),
            cwd: Some("C:\\Proj".into()),
            ended_at: None,
            is_subagent: false,
            updated_at: Some("2026-09-26T19:00:00Z".into()),
        },
        ClineSessionRow {
            pid: 100,
            model: Some("sub".into()),
            cwd: Some("C:\\Proj".into()),
            ended_at: None,
            is_subagent: true,
            updated_at: Some("2026-09-26T19:31:00Z".into()),
        },
    ];
    for c in &f.cline_cases {
        assert_eq!(
            cline_session_model(&cline_rows, Some(&c.cwd), |pid| pid != 999),
            c.expected,
            "cline_session_model mismatch on {}",
            c.name
        );
    }

    for c in &f.continue_cases {
        let items = continue_models(&c.yaml);
        assert_eq!(items.len(), 3);
        assert!(!items[0].chat && items[1].chat && items[2].chat);
        assert_eq!(
            continue_model(c.selected.as_deref(), &c.yaml),
            c.expected,
            "continue_model mismatch on {}",
            c.name
        );
    }

    let m = &f.misc_config_cases;
    assert_eq!(
        pi_settings_model(&json!({
            "defaultProvider": "anthropic",
            "defaultModel": "claude-sonnet-5",
            "defaultThinkingLevel": "medium"
        })),
        m.pi_settings
    );
    assert_eq!(
        openclaw_config_model(&json!({
            "agents": { "defaults": { "model": "anthropic/claude-opus-5" } }
        })),
        m.openclaw_string
    );
    assert_eq!(
        openclaw_config_model(&json!({
            "agents": { "defaults": { "model": { "primary": "openai-codex/gpt-5.4" } } }
        })),
        m.openclaw_primary
    );
    assert_eq!(
        droid_settings_model(&json!({
            "model": "gpt-5.1-codex",
            "reasoningEffort": "high"
        })),
        m.droid_settings
    );
    assert_eq!(
        crush_config_model(&json!({
            "models": { "large": { "model": "gpt-5.1", "provider": "openai", "reasoning_effort": "high" } }
        })),
        m.crush_config
    );
    assert_eq!(
        goose_config_model(
            "GOOSE_PROVIDER: anthropic\nGOOSE_MODEL: \"claude-sonnet-4-5\" # default\n"
        ),
        m.goose_config
    );
    assert_eq!(
        aider_config_model("model: o4-mini\nreasoning-effort: high\nauto-commits: false\n"),
        m.aider_config
    );
    assert_eq!(
        vibe_config_model(
            "active_model = \"devstral-2\"\n\n[[models]]\nname = \"mistral-medium\"\n"
        ),
        m.vibe_config
    );
    assert_eq!(
        grok_summary_model(&json!({
            "current_model_id": "grok-4-code",
            "reasoning_effort": "high",
            "updated_at": "2026-10-16T10:00:12.000Z"
        })),
        m.grok_summary
    );
    let grok_cache = json!({
        "models": {
            "grok-4-code": {
                "info": {
                    "name": "Grok 4 Code",
                    "reasoning_efforts": [
                        { "value": "low", "default": false },
                        { "value": "medium", "default": true }
                    ]
                }
            }
        }
    });
    assert_eq!(
        grok_catalog_entry(&grok_cache, "grok-4-code"),
        m.grok_catalog
    );
    assert_eq!(
        grok_config_model(
            "[models]\ndefault = \"grok-4-code\"\ndefault_reasoning_effort = \"xhigh\"\n"
        ),
        m.grok_config
    );

    for c in &f.command_cases {
        let cmd = CommandLine::new(&c.command);
        assert_eq!(
            model_from_command(cmd),
            c.model,
            "model_from_command on {:?}",
            c.command
        );
        assert_eq!(
            effort_from_command(cmd),
            c.claude_effort,
            "effort_from_command on {:?}",
            c.command
        );
        assert_eq!(
            muse_effort_from_command(cmd),
            c.muse_effort,
            "muse_effort_from_command on {:?}",
            c.command
        );
        assert_eq!(
            effort_flag(cmd),
            c.effort_flag,
            "effort_flag on {:?}",
            c.command
        );
        assert_eq!(
            amp_mode_from_command(cmd),
            c.amp_mode,
            "amp_mode_from_command on {:?}",
            c.command
        );
    }

    for c in &f.same_claude_cases {
        assert_eq!(
            same_claude_model(&c.a, &c.b),
            c.expected,
            "same_claude_model({:?}, {:?})",
            c.a,
            c.b
        );
    }
    for c in &f.codex_toml_cases {
        assert_eq!(
            codex_model_from_toml(&c.toml),
            c.expected,
            "codex_model_from_toml on {}",
            c.name
        );
    }
    for c in &f.cursor_config_cases {
        assert_eq!(
            cursor_config_model(&c.text),
            c.expected,
            "cursor_config_model on {}",
            c.name
        );
    }
    for c in &f.claude_settings_effort_cases {
        assert_eq!(
            claude_settings_effort(c.model.as_deref(), &c.docs),
            c.expected,
            "claude_settings_effort({:?})",
            c.model
        );
    }
}

#[test]
fn differential_resolve_agent_model_and_pane_header_orchestration() {
    let f = fixture();

    for c in &f.agent_model_cases {
        let inp = &c.inputs;
        let inputs = AgentModelInputs {
            agent_id: &inp.agent_id,
            session: inp.session.clone(),
            copilot_chunks_newest_first: inp.copilot_chunks_newest_first.clone(),
            command: CommandLine::from(inp.command.as_str()),
            chosen_model: inp.chosen_model.as_deref(),
            launched_at_ms: inp.launched_at_ms,
            picked: inp.picked.clone(),
            claude_settings_docs: inp.claude_settings_docs.clone(),
            openclaude_settings_json: inp.openclaude_settings_json.clone(),
            codex_config_toml: inp.codex_config_toml.clone(),
            cursor_cli_config_json: inp.cursor_cli_config_json.clone(),
            gemini_or_qwen_settings_docs: inp.gemini_or_qwen_settings_docs.clone(),
            opencode_config_docs: inp.opencode_config_docs.clone(),
            kilo_config_docs: inp.kilo_config_docs.clone(),
            copilot_settings_docs: inp.copilot_settings_docs.clone(),
            antigravity_settings_json: inp.antigravity_settings_json.clone(),
            muse_settings_json: inp.muse_settings_json.clone(),
            kimi_config_toml: inp.kimi_config_toml.clone(),
            pi_settings_json: inp.pi_settings_json.clone(),
            openclaw_config_json: inp.openclaw_config_json.clone(),
            droid_settings_json: inp.droid_settings_json.clone(),
            cline_providers_doc: inp.cline_providers_doc.clone(),
            continue_selected_model: inp.continue_selected_model.clone(),
            continue_config_yaml: inp.continue_config_yaml.clone(),
            crush_config_docs: inp.crush_config_docs.clone(),
            goose_config_yaml: inp.goose_config_yaml.clone(),
            aider_config_yamls: inp.aider_config_yamls.clone(),
            vibe_config_tomls: inp.vibe_config_tomls.clone(),
            grok_config_toml: inp.grok_config_toml.clone(),
            grok_settings_cache_json: inp.grok_settings_cache_json.clone(),
            grok_models_cache_json: inp.grok_models_cache_json.clone(),
            ollama_ps_json: inp.ollama_ps_json.clone(),
        };
        let actual = resolve_agent_model(&inputs);
        let expected = c.expected.clone().unwrap_or_default();
        assert_eq!(
            actual, expected,
            "resolve_agent_model mismatch on {}",
            c.name
        );
    }

    for hc in &f.header_refresh_cases {
        let sc = &hc.scenario;
        let catalog = f
            .catalogs
            .get(&sc.catalog_key)
            .expect("catalog present in fixture");
        let inputs = PaneHeaderModelInputs {
            node: sc.node.clone(),
            catalog_models: catalog,
            current_shown: sc.current_agent_model.clone(),
            current_session_model: sc.current_session_model.clone(),
            current_screen_context: parse_screen_context_fixture(
                sc.current_screen_context.as_ref(),
            ),
            has_term: sc.has_term.unwrap_or(true),
            screen_lines: &sc.screen_lines,
            top_screen_lines: &sc.top_screen_lines,
            scrollback_lines: &sc.scrollback_lines,
            poll: parse_poll_outcome_fixture(sc.agent_model_result.as_ref()),
        };
        let actual = resolve_pane_header_model(&inputs);
        assert_eq!(
            actual.shown, hc.expected.agent_model,
            "header shown mismatch on {}",
            sc.name
        );
        assert_eq!(
            actual.session_model, hc.expected.session_model,
            "header session_model mismatch on {}",
            sc.name
        );
        assert_eq!(
            actual.screen_context.map(CursorContextOnScreen::to_value),
            hc.expected.screen_context,
            "header screen_context mismatch on {}",
            sc.name
        );
        assert_eq!(
            actual.model_choice, hc.expected.model_choice,
            "header model_choice mismatch on {}",
            sc.name
        );
        assert_eq!(
            actual.to_save, hc.expected.to_save,
            "header to_save mismatch on {}",
            sc.name
        );
        assert_eq!(
            actual.froze_on_picker, hc.expected.froze_on_picker,
            "header froze_on_picker mismatch on {}",
            sc.name
        );
    }
}

#[test]
fn oversized_tool_result_over_8mib_preserves_recent_model_in_all_stream_parsers() {
    // Mirrors `pane_model_log::latest` reconstructing a chunk containing a > 8 MiB
    // tool-output line alongside a small recent model line: the oversized line must
    // be skipped in O(1) without rejecting the surrounding chunk or returning an
    // older stale model.
    let payload_bytes = 8 * 1024 * 1024 + 512 * 1024; // 8.5 MiB (> 8 MiB, > MAX_JSONL_LINE_BYTES)
    assert!(payload_bytes > MAX_JSONL_LINE_BYTES);
    let mut big_tool_line = String::with_capacity(payload_bytes + 64);
    big_tool_line
        .push_str("{\"type\":\"tool_result\",\"model\":\"stale-from-huge-line\",\"output\":\"");
    big_tool_line.extend(std::iter::repeat_n('x', payload_bytes));
    big_tool_line.push_str("\"}");

    let assert_both_orders = |label: &str,
                              old_line: &str,
                              recent_line: &str,
                              parse: fn(&str) -> Option<FoundModel>,
                              want_model: &str| {
        let mut before =
            String::with_capacity(old_line.len() + big_tool_line.len() + recent_line.len() + 4);
        before.push_str(old_line);
        before.push('\n');
        before.push_str(&big_tool_line);
        before.push('\n');
        before.push_str(recent_line);
        let got_after_big = parse(&before)
            .unwrap_or_else(|| panic!("{label}: None when recent follows 8.5 MiB line"));
        assert_eq!(
            got_after_big.model, want_model,
            "{label}: wrong model when recent follows 8.5 MiB line"
        );

        before.clear();
        before.push_str(old_line);
        before.push('\n');
        before.push_str(recent_line);
        before.push('\n');
        before.push_str(&big_tool_line);
        let got_before_big = parse(&before)
            .unwrap_or_else(|| panic!("{label}: None when 8.5 MiB line trails recent"));
        assert_eq!(
            got_before_big.model, want_model,
            "{label}: wrong model when 8.5 MiB line trails recent"
        );
    };

    assert_both_orders(
        "codex",
        r#"{"type":"turn_context","payload":{"model":"gpt-5.2","effort":"low"}}"#,
        r#"{"type":"turn_context","payload":{"model":"gpt-6-astra","effort":"high"}}"#,
        codex_model_from_text,
        "gpt-6-astra",
    );
    assert_both_orders(
        "claude",
        r#"{"type":"assistant","effort":"low","message":{"model":"claude-sonnet-4-5"}}"#,
        r#"{"type":"assistant","effort":"high","message":{"model":"claude-opus-5-5"}}"#,
        claude_turn_from_text,
        "claude-opus-5-5",
    );
    assert_both_orders(
        "copilot",
        r#"{"type":"assistant.message","data":{"model":"gpt-5-mini"}}"#,
        r#"{"type":"assistant.message","data":{"model":"gpt-6-luna"}}"#,
        copilot_model_from_text,
        "gpt-6-luna",
    );
    assert_both_orders(
        "antigravity",
        r#"{"type":"USER_INPUT","created_at":"2026-10-15T12:00:00Z","content":"<USER_SETTINGS_CHANGE>setting `Model Selection` from None to Gemini 3.8 Flash (Low).</USER_SETTINGS_CHANGE>"}"#,
        r#"{"type":"USER_INPUT","created_at":"2026-10-15T12:05:00Z","content":"<USER_SETTINGS_CHANGE>setting `Model Selection` from Gemini 3.8 Flash (Low) to Gemini 4 Argon (High).</USER_SETTINGS_CHANGE>"}"#,
        antigravity_model_from_text,
        "Gemini 4 Argon (High)",
    );
    assert_both_orders(
        "muse",
        r#"{"payload_type":"run.model.configured","recorded_at":"2026-10-15T14:00:00Z","payload":{"record":{"model_id":"muse-spark-1.2","reasoning_effort":"low"}}}"#,
        r#"{"payload_type":"run.model.configured","recorded_at":"2026-10-15T14:05:00Z","payload":{"record":{"model_id":"muse-wonder-1.0","reasoning_effort":"high"}}}"#,
        muse_model_from_text,
        "muse-wonder-1.0",
    );
    assert_both_orders(
        "gemini",
        r#"{"type":"gemini","model":"gemini-2.5-flash"}"#,
        r#"{"type":"gemini","model":"gemini-3-pro-preview"}"#,
        gemini_model_from_text,
        "gemini-3-pro-preview",
    );
    assert_both_orders(
        "qwen",
        r#"{"type":"assistant","model":"qwen2.5-coder"}"#,
        r#"{"type":"assistant","model":"qwen3-coder-plus"}"#,
        qwen_model_from_text,
        "qwen3-coder-plus",
    );
    assert_both_orders(
        "pi",
        r#"{"type":"model_change","provider":"openai","modelId":"gpt-5.2"}"#,
        r#"{"type":"model_change","provider":"anthropic","modelId":"claude-opus-5-5"}"#,
        pi_model_from_text,
        "anthropic/claude-opus-5-5",
    );

    let kimi_chunk = format!(
        "{}\n{}\n{}",
        r#"{"type":"profile.bind","modelAlias":"kimi-old","thinkingEffort":"low"}"#,
        big_tool_line,
        r#"{"type":"config.update","modelAlias":"kimi-new","thinkingEffort":"high"}"#,
    );
    let kw = kimi_wire_model(&kimi_chunk).expect("kimi_wire_model after 8.5 MiB line");
    assert_eq!(kw.model.as_deref(), Some("kimi-new"));
    assert_eq!(kw.thinking.as_deref(), Some("high"));
}

#[test]
fn remaining_settings_parsers_chunks_and_scalar_helpers_match_js() {
    assert_eq!(
        claude_settings_model(&["{\"effortLevel\":\"high\"}", "{\"model\":\"opus[1m]\"}"])
            .as_deref(),
        Some("opus[1m]")
    );
    assert_eq!(
        openclaude_settings_model(r#"{"model":"claude-opus-5-5","effortLevel":"max"}"#),
        Some(FoundModel {
            model: "claude-opus-5-5".into(),
            name: None,
            effort: Some("max".into()),
            at: None,
        })
    );
    assert_eq!(
        gemini_or_qwen_settings_model(&[r#"{"model":{"name":"gemini-3-pro"}}"#])
            .map(|f| f.model)
            .as_deref(),
        Some("gemini-3-pro")
    );
    assert_eq!(
        opencode_settings_model(&["// c\n{\"model\":\"opencode/mimo\",}"])
            .map(|f| f.model)
            .as_deref(),
        Some("opencode/mimo")
    );
    assert_eq!(
        kilo_config_model(&[r#"{"model":"kilo/sonnet"}"#])
            .map(|f| f.model)
            .as_deref(),
        Some("kilo/sonnet")
    );
    assert_eq!(
        copilot_settings_model(&[r#"{"model":"gpt-6-luna"}"#])
            .map(|f| f.model)
            .as_deref(),
        Some("gpt-6-luna")
    );
    assert_eq!(
        opencode_picked_model(
            r#"{"recent":[{"providerID":"opencode","modelID":"big-pickle"}],"variant":{"opencode/big-pickle":"high"}}"#,
            1234,
        ),
        Some(PickedModel {
            model: "opencode/big-pickle".into(),
            effort: Some("high".into()),
            mtime_ms: 1234,
        })
    );
    assert_eq!(
        antigravity_settings_model(r#"{"model":"Gemini 4 Argon (High)"}"#),
        Some(FoundModel {
            model: "Gemini 4 Argon (High)".into(),
            name: Some("Gemini 4 Argon".into()),
            effort: Some("high".into()),
            at: None,
        })
    );
    assert!(cline_settings_model(
        r#"{"lastUsedProvider":"p","providers":{"p":{"settings":{"model":"x","reasoning":{"enabled":false}}}}}"#,
        99
    )
    .is_some_and(|c| c.effort.as_deref() == Some("none") && c.mtime_ms == 99));

    let oc_idx = r#"{
        "older": {"model":"gpt-5.2","modelProvider":"openai","updatedAt":100},
        "newer": {"model":"claude-opus-5-5","modelProvider":"anthropic","updatedAt":200}
    }"#;
    let oc = openclaw_index_model(
        oc_idx,
        Some(r#"{"type":"thinking_level_change","thinkingLevel":"high"}"#),
        Some(r#"{"type":"model_change","provider":"anthropic","modelId":"claude-opus-5-5"}"#),
    )
    .expect("openclaw_index_model");
    assert_eq!(oc.model, "anthropic/claude-opus-5-5");
    assert_eq!(oc.effort.as_deref(), Some("high"));
    assert_eq!(oc.at, Some(200_000));

    assert!(
        opencode_message_model(r#"{"role":"assistant","modelID":"m1","providerID":"p1"}"#)
            .is_some_and(|m| m.model == "p1/m1")
    );
    assert!(
        opencode_session_column(r#"{"id":"m2","providerID":"p2","variant":"low"}"#)
            .is_some_and(|m| m.model == "p2/m2" && m.effort.as_deref() == Some("low"))
    );

    let grok = grok_settings_model(
        None,
        Some(r#"{"payload":"{\"settings\":{\"default_model\":\"grok-4-code\"}}"}"#),
        Some(r#"{"models":{"grok-4-code":{"info":{"name":"Grok 4 Code","reasoning_effort":"high"}}}}"#),
    )
    .expect("grok_settings_model from wrapped cache");
    assert_eq!(grok.model, "grok-4-code");
    assert_eq!(grok.name.as_deref(), Some("Grok 4 Code"));
    assert_eq!(grok.effort.as_deref(), Some("high"));

    assert_eq!(
        ollama_running_model_from_json(r#"{"models":[{"name":"qwen3:32b"}]}"#).as_deref(),
        Some("qwen3:32b")
    );
    assert_eq!(
        ollama_running_model_from_json(r#"{"models":[{"name":"a"},{"name":"b"}]}"#),
        None
    );

    let toml =
        toml_keys("flag = true\ncount = 42\n[models.\"my/alias\"]\ndisplay_name = \"My Alias\"\n");
    assert_eq!(toml[""]["flag"].as_bool(), Some(true));
    assert_eq!(toml[""]["count"].as_f64(), Some(42.0));
    assert_eq!(
        toml["models\nmy/alias"]["display_name"].as_str(),
        Some("My Alias")
    );
    assert_eq!(
        kimi_display_name(&toml, "my/alias").as_deref(),
        Some("My Alias")
    );

    // Copilot chunk primitives
    let newer = copilot_records(r#"{"type":"assistant.message","data":{"model":"gpt-6-luna"}}"#);
    let older = copilot_records(
        r#"{"type":"session.model_change","data":{"newModel":"auto","reasoningEffort":"medium"}}"#,
    );
    assert!(!copilot_is_done(newer.as_ref().expect("newer")));
    let merged = copilot_merge(newer, older);
    assert!(copilot_is_done(merged.as_ref().expect("merged")));
    let fin = copilot_finish(merged).expect("finished");
    assert_eq!(fin.model, "gpt-6-luna");
    assert_eq!(fin.effort.as_deref(), Some("medium"));

    let shown = fin.into_shown(ModelSource::Session);
    assert_eq!(shown.source, ModelSource::Session);

    assert_eq!(norm_dir("C:/Users/Test/Proj///"), "c:\\users\\test\\proj");
    assert_eq!(parse_iso_ms("2026-10-16"), Some(1_792_108_800_000));
    assert_eq!(
        time_of_value(&json!(1_700_000_000)),
        Some(1_700_000_000_000)
    );
    assert_eq!(
        muse_time_ms(&json!(1_760_529_600_000_000_i64)),
        1_760_529_600_000
    );
    assert_eq!(muse_time_ms(&json!("1760529600")), 1_760_529_600_000);
}

#[test]
fn adversarial_json_depth_utf16_surrogate_caps_and_debug_redaction() {
    // 1. Deeply nested JSON is rejected in linear time before serde_json recursion.
    let deep_arrays = format!(
        "{}{}",
        "[".repeat(MAX_JSON_DEPTH + 10),
        "]".repeat(MAX_JSON_DEPTH + 10)
    );
    assert!(!within_json_depth(&deep_arrays, MAX_JSON_DEPTH));
    assert_eq!(parse_loose_json(&deep_arrays), None);
    assert_eq!(codex_model_from_text(&deep_arrays), None);

    // Braces/brackets inside JSON string literals must NOT trip the depth guard.
    let braces_in_string = format!(
        r#"{{"type":"turn_context","payload":{{"model":"gpt-6-astra","note":"{}"}}}}"#,
        "{[".repeat(200)
    );
    assert!(within_json_depth(&braces_in_string, MAX_JSON_DEPTH));
    assert_eq!(
        codex_model_from_text(&braces_in_string)
            .map(|f| f.model)
            .as_deref(),
        Some("gpt-6-astra")
    );

    // Oversized config document (> 512 KiB) is rejected cleanly.
    let huge_doc = format!(
        "{{\"model\":\"gpt-6\",\"pad\":\"{}\"}}",
        "a".repeat(MAX_DOCUMENT_BYTES)
    );
    assert_eq!(parse_loose_json(&huge_doc), None);

    // 2. Surrogate-pair emojis (4 UTF-8 bytes, 2 UTF-16 code units) on exact caps never panic.
    let sixty_emojis = "😀".repeat(MAX_MODEL_ID_UTF16 / 2); // 120 UTF-16 units
    assert_eq!(valid_model_id(&sixty_emojis), Some(sixty_emojis.as_str()));
    let sixty_one_emojis = format!("{sixty_emojis}a"); // 121 UTF-16 units
    assert_eq!(valid_model_id(&sixty_one_emojis), None);
    assert_eq!(valid_model_id("bad\u{0007}control"), None);

    let display_emojis = format!("GPT {}", "😀".repeat(60));
    let cursor_cfg = json!({
        "model": { "modelId": "gpt-5.6-luna", "displayName": display_emojis }
    })
    .to_string();
    let parsed_cursor = cursor_config_model(&cursor_cfg).expect("cursor emoji displayName");
    assert!(
        parsed_cursor
            .name
            .as_deref()
            .is_some_and(|n| n.encode_utf16().count() <= 80)
    );

    let long_emoji_cmd = format!("claude --model opus {}", "😀".repeat(MAX_COMMAND_UTF16));
    let bounded = CommandLine::new(&long_emoji_cmd).bounded_str();
    assert!(bounded.encode_utf16().count() <= MAX_COMMAND_UTF16);
    assert_eq!(
        model_from_command(CommandLine::new(&long_emoji_cmd)).as_deref(),
        Some("opus")
    );

    // 3. CommandLine and input structs never leak embedded secrets in Debug output.
    let secret_raw = "claude --model opus --api-key sk-live-super-secret-token-9999";
    let cmd = CommandLine::new(secret_raw);
    let dbg_cmd = format!("{cmd:?}");
    assert!(!dbg_cmd.contains("sk-live-super-secret"));
    assert!(dbg_cmd.contains("<redacted>"));

    let am_inputs = AgentModelInputs {
        agent_id: "claude",
        command: cmd,
        ..Default::default()
    };
    assert!(!format!("{am_inputs:?}").contains("sk-live-super-secret"));

    let timed_secret = TimedDocument {
        text: r#"{"apiKey":"sk-cline-provider-secret-777","model":"x"}"#.into(),
        mtime_ms: 1_760_000_000_000,
    };
    let dbg_timed = format!("{timed_secret:?}");
    assert!(!dbg_timed.contains("sk-cline-provider-secret"));
    assert!(dbg_timed.contains("1760000000000"));
    assert!(dbg_timed.contains("<redacted>"));

    let row_secret = OpenCodeMessageRow {
        data: r#"{"role":"user","token":"sk-opencode-row-secret-888"}"#.into(),
        time_created: Some(42),
    };
    assert!(!format!("{row_secret:?}").contains("sk-opencode-row-secret"));

    let ph_inputs = PaneHeaderModelInputs {
        node: PaneModelNode {
            agent_id: "claude".into(),
            agent_command: Some(secret_raw.into()),
            ..Default::default()
        },
        catalog_models: &[],
        current_shown: None,
        current_session_model: None,
        current_screen_context: None,
        has_term: false,
        screen_lines: &[],
        top_screen_lines: &[],
        scrollback_lines: &[],
        poll: AgentModelPollOutcome::default(),
    };
    assert!(!format!("{ph_inputs:?}").contains("sk-live-super-secret"));
}

#[test]
fn claude_muse_and_openclaw_preserve_js_insertion_and_duplicate_key_order() {
    // 1. Case-variant keys in Claude `modelSettings`: first inserted entry wins.
    let lower_then_upper = [
        r#"{"modelSettings":{"claude-opus-4-8":{"effortLevel":"high"},"CLAUDE-OPUS-4-8":{"effortLevel":"low"}}}"#,
    ];
    assert_eq!(
        claude_settings_effort(Some("claude-opus-4-8"), &lower_then_upper).as_deref(),
        Some("high")
    );
    let upper_then_lower = [
        r#"{"modelSettings":{"CLAUDE-OPUS-4-8":{"effortLevel":"low"},"claude-opus-4-8":{"effortLevel":"high"}}}"#,
    ];
    assert_eq!(
        claude_settings_effort(Some("claude-opus-4-8"), &upper_then_lower).as_deref(),
        Some("low")
    );

    // 2. Family alias (`opus`) matches the first inserted matching family entry (`claude-opus-5-5`),
    //    not the lexicographically smallest (`claude-opus-4-8`).
    let alias_order = [
        r#"{"modelSettings":{"claude-opus-5-5":{"effortLevel":"max"},"claude-opus-4-8":{"effortLevel":"low"}}}"#,
    ];
    assert_eq!(
        claude_settings_effort(Some("opus"), &alias_order).as_deref(),
        Some("max")
    );

    // 3. Exact duplicate JSON keys update the first occurrence's slot in place with the last value.
    let duplicate_wins_in_first_slot = [
        r#"{"modelSettings":{"claude-opus-4-8":{"effortLevel":"high"},"CLAUDE-OPUS-4-8":{"effortLevel":"low"},"claude-opus-4-8":{"effortLevel":"max"}}}"#,
    ];
    assert_eq!(
        claude_settings_effort(Some("claude-opus-4-8"), &duplicate_wins_in_first_slot).as_deref(),
        Some("max")
    );

    // 4. Exact duplicate JSON key overwritten with an invalid effort skips slot 0 and falls through to slot 1.
    let duplicate_invalid_falls_through = [
        r#"{"modelSettings":{"claude-opus-4-8":{"effortLevel":"max"},"CLAUDE-OPUS-4-8":{"effortLevel":"low"},"claude-opus-4-8":{"effortLevel":"bogus"}}}"#,
    ];
    assert_eq!(
        claude_settings_effort(Some("claude-opus-4-8"), &duplicate_invalid_falls_through)
            .as_deref(),
        Some("low")
    );

    // 5. ECMA-262 integer indices (`< u32::MAX`, canonical digits) precede normal keys in numeric order,
    //    while `"01"` and `"4294967295"` remain normal insertion-ordered string keys.
    let ecma_indices = [
        r#"{"modelSettings":{"01-opus-1":{"effortLevel":"low"},"4294967295-opus-1":{"effortLevel":"medium"},"10":{"effortLevel":"high"},"2":{"effortLevel":"max"}}}"#,
    ];
    assert_eq!(
        claude_settings_effort(Some("opus"), &ecma_indices).as_deref(),
        Some("low")
    );
    assert_eq!(
        claude_settings_effort(Some("2"), &ecma_indices).as_deref(),
        Some("max")
    );

    // 6. Muse `model_reasoning_efforts` preserves insertion order for trimmed-key collisions and 100-entry cap.
    let muse_trimmed = r#"{"model":"muse-spark-1.3","model_reasoning_efforts":{"muse-spark-1.3":"max"," muse-spark-1.3 ":"low"}}"#;
    let parsed_muse = muse_settings_model(muse_trimmed).expect("valid muse settings");
    assert_eq!(parsed_muse.effort.as_deref(), Some("low"));

    // 7. OpenClaw `sessions.json` tie on `updatedAt` keeps the first inserted session (`z-first`).
    let openclaw_tie = r#"{"z-first":{"model":"claude-opus-5-5","updatedAt":100},"a-second":{"model":"gpt-5.4","updatedAt":100}}"#;
    assert_eq!(
        openclaw_index_model(openclaw_tie, None, None)
            .map(|f| f.model)
            .as_deref(),
        Some("claude-opus-5-5")
    );
}
