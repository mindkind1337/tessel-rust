//! Differential and stateful unit tests for `src/pane_session_model.rs` against
//! the 197-vector reference fixture generated from Tessel's Vue/JS modules
//! (`tests/fixtures/pane_session_model.json`), plus observer state-machine,
//! redaction, custom-hasher, UTF-8 boundary, and bounded screen-reader checks.

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

#[path = "../src/pane_session_model.rs"]
mod pane_session_model;

use pane_session_model::{
    AgentPermissionMode, AgentPrefEntry, CLAUDE_SWITCH_TIMEOUT_MS, ClaudeModelSwitchObserver,
    ClaudeObserverStep, ClaudeSwitchOutcome, CommandDelivery, CursorContextOnScreen,
    LaunchSignature, LaunchSpec, MAX_CLAUDE_OBSERVED_UTF16, MAX_OPTION_KEY_LEN,
    MAX_OPTION_VALUE_UTF16, MAX_PANE_OPTION_ENTRIES, MAX_SAVED_MODEL_UTF16, MAX_SCREEN_LINE_UTF16,
    MAX_SCREEN_LINES, MAX_SWITCH_LINE_UTF16, ModelPickOutcome, ModelSource, OptimisticModelChoice,
    PaneModelNode, PanePickContext, PaneSessionOptions, ShownAgentModel, ToastKind,
    antigravity_listed, antigravity_model_on_screen, apply_claude_switch_outcome,
    apply_restart_title, chip_aria_label, codex_model_on_screen, compose_cursor_row_variant,
    composed_model_id, cursor_context_on_screen, cursor_model_from_status_line,
    cursor_model_on_screen, cursor_picker_filter, cursor_picker_shown, effective_agent_launch,
    effective_model_id, effective_model_id_of, effort_level, has_claude_model_switch_confirmation,
    has_claude_model_switch_rejection, has_claude_model_switch_success, header_effort_label,
    header_model_text, is_launch_stale, launch_session_values, launch_signature,
    listed_model_values, model_busy_reason, model_default_label, model_from_screen,
    model_from_switch_line, model_in_line, model_label, model_menu_note, model_text, model_title,
    muse_footer_model, newest_claude_of, next_pane_values, pane_menu_values, pane_model_to_save,
    persisted_defaults_from_json, plan_chat_view_set_option, plan_model_action, plan_model_pick,
    resolve_chosen_shown, restart_apply_badge_text, same_pane_model, saved_pane_model,
    session_choice_error, strip_ansi, toast_for_outcome, valid_launch_signature, valid_option_key,
    valid_pane_session_options, valid_pane_session_options_value, with_model_version,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn load_fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/pane_session_model.json"))
        .expect("valid pane_session_model.json fixture")
}

/// Test-only helper: hashes a raw Vue JS `launchSignature` string from the fixture
/// into its canonical 64-character lowercase hex SHA-256 [`LaunchSignature`].
fn hash_raw_fixture_signature(raw_js_sig: &str) -> LaunchSignature {
    let digest = format!("{:x}", Sha256::digest(raw_js_sig.as_bytes()));
    LaunchSignature::parse(&digest).expect("SHA-256 hex digest is valid LaunchSignature")
}

/// Test-only helper: converts a Vue JS fixture `node` (whose `launchSig` is raw JSON)
/// into the canonical SHA-256 `launchSig` representation enforced by [`PaneModelNode`].
fn convert_fixture_node_to_hashed(raw_node: &Value) -> Value {
    let mut out = raw_node.clone();
    if let Some(obj) = out.as_object_mut()
        && let Some(raw_sig) = obj.get("launchSig").and_then(Value::as_str)
        && !valid_launch_signature(raw_sig)
    {
        let hashed = hash_raw_fixture_signature(raw_sig);
        obj.insert("launchSig".into(), Value::String(hashed.into_string()));
    }
    out
}

fn json_lines(arr: &Value) -> Vec<&str> {
    arr.as_array()
        .expect("JSON string array")
        .iter()
        .map(|v| v.as_str().expect("string line"))
        .collect()
}

#[test]
fn constants_and_option_bounds_match_spec() {
    assert_eq!(MAX_PANE_OPTION_ENTRIES, 10);
    assert_eq!(MAX_OPTION_KEY_LEN, 40);
    assert_eq!(MAX_OPTION_VALUE_UTF16, 160);
    assert_eq!(MAX_SAVED_MODEL_UTF16, 120);
    assert_eq!(CLAUDE_SWITCH_TIMEOUT_MS, 5_000);
    assert_eq!(MAX_CLAUDE_OBSERVED_UTF16, 65_536);
    assert_eq!(MAX_SCREEN_LINES, 64);
    assert_eq!(MAX_SCREEN_LINE_UTF16, 512);
    assert_eq!(MAX_SWITCH_LINE_UTF16, 1_024);

    assert!(valid_option_key("effort"));
    assert!(valid_option_key("fastMode_2"));
    assert!(!valid_option_key(""));
    assert!(!valid_option_key("1effort"));
    assert!(!valid_option_key("fast-mode"));

    assert!(PaneSessionOptions::new("--yolo").is_none());
    let mut opts = PaneSessionOptions::new("claude-opus-5-5").expect("valid model");
    assert_eq!(opts.model(), "claude-opus-5-5");
    assert!(opts.effort().is_none());
    assert!(!opts.set_option("model", Some(json!("sonnet"))));
    assert!(opts.set_option("effort", Some(json!("high"))));
    assert_eq!(opts.effort(), Some("high"));
    assert_eq!(opts.get("effort"), Some(&json!("high")));
    assert_eq!(opts.as_map().len(), 2);
    assert!(opts.set_option("effort", None));
    assert!(opts.effort().is_none());

    let as_val = opts.to_value();
    let round_trip: PaneSessionOptions = serde_json::from_value(as_val.clone()).unwrap();
    assert_eq!(
        PaneSessionOptions::from_value(&as_val),
        Some(round_trip.clone())
    );
    assert_eq!(
        round_trip.into_value(),
        json!({ "model": "claude-opus-5-5" })
    );
}

#[test]
fn differential_persistence_launch_and_cli_validation_match_vue() {
    let fixture = load_fixture();

    // 1. validPaneCases (22 vectors)
    let valid_cases = fixture["validPaneCases"].as_array().unwrap();
    assert_eq!(valid_cases.len(), 22);
    for case in valid_cases {
        let name = case["name"].as_str().unwrap();
        let actual = serde_json::to_value(valid_pane_session_options(&case["input"])).unwrap();
        assert_eq!(actual, case["expected"], "validPaneCases: {name}");
        let direct = valid_pane_session_options_value(&case["input"]).unwrap_or(Value::Null);
        assert_eq!(
            direct, case["expected"],
            "valid_pane_session_options_value: {name}"
        );
    }

    // 2. launchValuesCases (6 vectors) + snake_case persisted defaults support
    let launch_cases = fixture["launchValuesCases"].as_array().unwrap();
    assert_eq!(launch_cases.len(), 6);
    for case in launch_cases {
        let name = case["name"].as_str().unwrap();
        let persisted = persisted_defaults_from_json(Some(&case["persisted"]));
        let pane_choice = (!case["paneChoice"].is_null()).then_some(&case["paneChoice"]);
        let agent_id = case["agentId"].as_str().unwrap();
        let actual =
            serde_json::to_value(launch_session_values(pane_choice, &persisted, agent_id)).unwrap();
        assert_eq!(actual, case["expected"], "launchValuesCases: {name}");
    }
    let snake_defaults = persisted_defaults_from_json(Some(&json!({
        "claude": { "model": "opus", "values_by_model": { "opus": { "effort": "max" } } }
    })));
    assert_eq!(
        launch_session_values(None, &snake_defaults, "claude"),
        Some(json!({ "model": "opus", "effort": "max" }))
    );

    // 3. launchSignatureCases (3 vectors) + launch_fingerprint.json compatibility + Debug redaction
    let sig_cases = fixture["launchSignatureCases"].as_array().unwrap();
    assert_eq!(sig_cases.len(), 3);
    for case in sig_cases {
        let name = case["name"].as_str().unwrap();
        let spec: LaunchSpec = serde_json::from_value(case["launch"].clone()).unwrap();
        let sig = launch_signature(&spec);
        assert!(valid_launch_signature(sig.as_str()));
        assert_eq!(
            sig,
            hash_raw_fixture_signature(case["expected"].as_str().unwrap()),
            "launchSignatureCases: {name}"
        );
        let debug_text = format!("{spec:?}");
        assert!(debug_text.contains("<redacted>"));
        assert!(debug_text.contains("env_count"));
        if !spec.command.is_empty() {
            assert!(
                !debug_text.contains(&spec.command),
                "LaunchSpec Debug leaked command in {name}: {debug_text}"
            );
        }
        if !spec.args.is_empty() {
            assert!(
                !debug_text.contains(&spec.args),
                "LaunchSpec Debug leaked args in {name}: {debug_text}"
            );
        }
        for secret in spec.env.values() {
            if !secret.is_empty() {
                assert!(
                    !debug_text.contains(secret),
                    "LaunchSpec Debug leaked env value in {name}: {debug_text}"
                );
            }
        }
    }

    // Verify 100% byte-for-byte identity with `tests/fixtures/launch_fingerprint.json`.
    let fp_fixture: Value =
        serde_json::from_str(include_str!("fixtures/launch_fingerprint.json")).unwrap();
    for case in fp_fixture["cases"].as_array().unwrap() {
        let spec: LaunchSpec = serde_json::from_value(case["input"].clone()).unwrap();
        let sig = launch_signature(&spec);
        assert_eq!(sig, case["expected"].as_str().unwrap());
        assert!(valid_launch_signature(sig.as_str()));
    }

    let pref = AgentPrefEntry {
        enabled: Some(true),
        command: "C:\\secret-wrapper\\claude.exe".into(),
        args: "--api-key sk-arg-secret-456".into(),
        env: "ANTHROPIC_API_KEY=sk-secret-value-123".into(),
    };
    assert_eq!(pref.enabled, Some(true));
    let pref_dbg = format!("{pref:?}");
    assert!(pref_dbg.contains("<redacted>"));
    assert!(!pref_dbg.contains("secret-wrapper"));
    assert!(!pref_dbg.contains("sk-arg-secret-456"));
    assert!(!pref_dbg.contains("sk-secret-value-123"));

    // 4. effectiveAgentCases (5 vectors) + launch_stale helper
    let changed_sig = LaunchSignature::of("other-cli", "--flag", &BTreeMap::new());
    let effective_cases = fixture["effectiveAgentCases"].as_array().unwrap();
    assert_eq!(effective_cases.len(), 5);
    for case in effective_cases {
        let name = case["name"].as_str().unwrap();
        let agent_id = case["agent"]["id"].as_str().unwrap();
        let default_cmd = case["agent"]["command"].as_str().unwrap();
        let prefs: BTreeMap<String, AgentPrefEntry> =
            serde_json::from_value(case["prefs"].clone()).unwrap();
        let perms = AgentPermissionMode::from_str_lossy(case["permissions"].as_str().unwrap());
        let values = (!case["values"].is_null()).then_some(&case["values"]);
        let models = agent_model_catalog::models_for(agent_id, None);
        let spec = effective_agent_launch(
            agent_id,
            default_cmd,
            prefs.get(agent_id),
            perms,
            values,
            Some(&models),
        );
        assert!(!spec.command.is_empty());
        assert_eq!(
            serde_json::to_value(&spec).unwrap(),
            case["expectedLaunch"],
            "effectiveAgentCases launch: {name}"
        );
        let sig = launch_signature(&spec);
        assert!(valid_launch_signature(sig.as_str()));
        assert_eq!(
            sig,
            hash_raw_fixture_signature(case["expectedSignature"].as_str().unwrap()),
            "effectiveAgentCases signature: {name}"
        );
        assert!(!is_launch_stale(Some(&sig), true, false, &sig));
        assert!(is_launch_stale(Some(&sig), true, false, &changed_sig));
        assert!(!is_launch_stale(Some(&sig), true, true, &changed_sig));
        assert!(!is_launch_stale(None, true, false, &changed_sig));
    }
    assert_eq!(
        AgentPermissionMode::from_str_lossy("manual"),
        AgentPermissionMode::Manual
    );
    assert_eq!(
        AgentPermissionMode::from_str_lossy("yolo"),
        AgentPermissionMode::Yolo
    );

    // 5. savedModelCases (11 vectors) + same_pane_model
    let saved_cases = fixture["savedModelCases"].as_array().unwrap();
    assert_eq!(saved_cases.len(), 11);
    for case in saved_cases {
        let name = case["name"].as_str().unwrap();
        let input = (!case["input"].is_null()).then_some(&case["input"]);
        let saved = saved_pane_model(input);
        let to_save = pane_model_to_save(input);
        if let Some(ref s) = saved {
            assert!(!s.model.is_empty());
            let _ = (&s.name, &s.effort);
        }
        assert!(same_pane_model(saved.as_ref(), saved.as_ref()));
        assert_eq!(
            serde_json::to_value(&saved).unwrap(),
            case["expectedSaved"],
            "savedModelCases saved: {name}"
        );
        assert_eq!(
            serde_json::to_value(&to_save).unwrap(),
            case["expectedToSave"],
            "savedModelCases toSave: {name}"
        );
    }

    // 6. sessionChoiceErrorCases (10 vectors × EN & FR) + Cursor composition helpers
    let cursor_models = fixture["cursorFixtureModels"].as_array().unwrap();
    let codex_row = cursor_models
        .iter()
        .find(|m| m["id"] == "gpt-5.3-codex")
        .unwrap();
    let mut compose_vals = serde_json::Map::new();
    compose_vals.insert("effort".into(), json!("high"));
    compose_vals.insert("fastMode".into(), json!(true));
    assert_eq!(
        compose_cursor_row_variant(codex_row, &compose_vals),
        "gpt-5.3-codex-high-fast"
    );
    assert_eq!(
        listed_model_values(cursor_models, "gpt-5.3-codex-high-fast").map(Value::Object),
        Some(json!({ "model": "gpt-5.3-codex", "effort": "high", "fastMode": true }))
    );
    assert_eq!(
        composed_model_id(
            "cursor",
            Some(&json!({ "model": "gpt-5.3-codex", "effort": "high", "fastMode": true })),
            Some(cursor_models),
        )
        .as_deref(),
        Some("gpt-5.3-codex-high-fast")
    );

    let err_cases = fixture["sessionChoiceErrorCases"].as_array().unwrap();
    assert_eq!(err_cases.len(), 10);
    for case in err_cases {
        let name = case["name"].as_str().unwrap();
        let agent_id = case["agentId"].as_str().unwrap();
        let model = case["model"].as_str().unwrap();
        let effort = case["effort"].as_str().unwrap();
        let models = case["useCursorFixtureModels"]
            .as_bool()
            .unwrap()
            .then_some(cursor_models.as_slice());
        assert_eq!(
            session_choice_error(agent_id, model, effort, models, false).as_deref(),
            case["expectedEn"].as_str(),
            "sessionChoiceErrorCases EN: {name}"
        );
        assert_eq!(
            session_choice_error(agent_id, model, effort, models, true).as_deref(),
            case["expectedFr"].as_str(),
            "sessionChoiceErrorCases FR: {name}"
        );
    }
}

#[test]
fn differential_labels_headers_chosen_shown_and_version_resolution_match_vue() {
    let fixture = load_fixture();

    // 7. effortLevelCases (19 vectors) + header_effort_label
    let effort_cases = fixture["effortLevelCases"].as_array().unwrap();
    assert_eq!(effort_cases.len(), 19);
    for case in effort_cases {
        let input = case["input"].as_str().unwrap();
        assert_eq!(
            effort_level(input),
            case["expected"].as_str(),
            "effortLevel({input:?})"
        );
    }
    assert_eq!(header_effort_label("xhigh", false), "Extra high");
    assert_eq!(header_effort_label("xhigh", true), "Très élevé");
    assert_eq!(header_effort_label("high", true), "Élevé");

    // 8. modelLabelCases (28 vectors)
    let label_cases = fixture["modelLabelCases"].as_array().unwrap();
    assert_eq!(label_cases.len(), 28);
    for case in label_cases {
        let input = case["input"].as_str().unwrap();
        assert_eq!(
            model_label(input),
            case["expected"].as_str().unwrap(),
            "modelLabel({input:?})"
        );
    }

    // 9. headerTitleCases (9 vectors × EN & FR) covering all 8 ModelSource variants
    for src in [
        ModelSource::Session,
        ModelSource::Command,
        ModelSource::Picked,
        ModelSource::Chosen,
        ModelSource::Screen,
        ModelSource::Saved,
        ModelSource::Running,
        ModelSource::Settings,
    ] {
        assert!(!src.description(false).is_empty());
        assert!(!src.description(true).is_empty());
    }

    let header_cases = fixture["headerTitleCases"].as_array().unwrap();
    assert_eq!(header_cases.len(), 9);
    for case in header_cases {
        let name = case["name"].as_str().unwrap();
        let is_agent = case["isAgent"].as_bool().unwrap();
        let has_model_choice = case["hasModelChoice"].as_bool().unwrap();
        let guest_runs = case["guestRuns"].as_bool().unwrap();
        let guest_model = case.get("guestModel").and_then(Value::as_str);
        let agent_id = case["agentId"].as_str().unwrap();
        let pane_running = case["paneRunning"].as_bool().unwrap();
        let pane_busy = case["paneBusy"].as_bool().unwrap();
        let models = case["models"].as_array().unwrap();
        let persisted = persisted_defaults_from_json(Some(&case["persistedDefaults"]));
        let agent_model: Option<ShownAgentModel> =
            serde_json::from_value(case["agentModel"].clone()).unwrap();
        if let Some(ref m) = agent_model {
            let _ = (&m.name, &m.chosen_effort, &m.at);
        }

        for (french, expected) in [(false, &case["expectedEn"]), (true, &case["expectedFr"])] {
            let loc = if french { "FR" } else { "EN" };
            let m_text = model_text(agent_model.as_ref(), french);
            let h_text = header_model_text(
                is_agent,
                guest_runs,
                guest_model,
                agent_model.as_ref(),
                french,
            );
            let aria = chip_aria_label(&h_text, has_model_choice, guest_runs, french);
            let title = model_title(agent_model.as_ref(), french);
            let def_lbl = model_default_label(agent_id, models, &persisted, french);
            let note = model_menu_note(agent_id, pane_running, french);
            let busy = model_busy_reason(pane_running, pane_busy, french);
            let apply_t = apply_restart_title(french);
            let badge = restart_apply_badge_text(french);

            assert_eq!(
                m_text,
                expected["modelText"].as_str().unwrap(),
                "{name} {loc} modelText"
            );
            assert_eq!(
                h_text,
                expected["headerModelText"].as_str().unwrap(),
                "{name} {loc} headerModelText"
            );
            assert_eq!(
                aria,
                expected["chipLabel"].as_str().unwrap(),
                "{name} {loc} chipLabel"
            );
            assert_eq!(
                title,
                expected["modelTitle"].as_str().unwrap(),
                "{name} {loc} modelTitle"
            );
            assert_eq!(
                def_lbl,
                expected["modelDefaultLabel"].as_str().unwrap(),
                "{name} {loc} modelDefaultLabel"
            );
            assert_eq!(
                note,
                expected["modelMenuNote"].as_str().unwrap(),
                "{name} {loc} modelMenuNote"
            );
            assert_eq!(
                busy,
                expected["modelBusyReason"].as_str().unwrap(),
                "{name} {loc} modelBusyReason"
            );
            assert_eq!(
                apply_t,
                expected["applyTitle"].as_str().unwrap(),
                "{name} {loc} applyTitle"
            );
            assert_eq!(
                badge,
                expected["restartApplyBadge"].as_str().unwrap(),
                "{name} {loc} restartApplyBadge"
            );
        }
    }

    // 10. chosenShownCases (6 vectors)
    let chosen_cases = fixture["chosenShownCases"].as_array().unwrap();
    assert_eq!(chosen_cases.len(), 6);
    for case in chosen_cases {
        let name = case["name"].as_str().unwrap();
        let node: PaneModelNode = serde_json::from_value(case["node"].clone()).unwrap();
        let res: Option<ShownAgentModel> = serde_json::from_value(case["res"].clone()).unwrap();
        let models = case["models"].as_array().unwrap();
        let outcome = resolve_chosen_shown(
            node.session_options.as_ref(),
            node.model_choice.as_ref(),
            node.remote_host_id.is_some(),
            res.as_ref(),
            models,
        );
        if let Some(OptimisticModelChoice {
            ref model,
            ref effort,
            at,
            ref seen,
        }) = outcome.model_choice
        {
            assert!(!model.is_empty());
            let _ = (effort, at, seen);
        }
        assert_eq!(
            serde_json::to_value(&outcome.shown).unwrap(),
            case["expected"]["shown"],
            "chosenShownCases shown: {name}"
        );
        assert_eq!(
            serde_json::to_value(&outcome.model_choice).unwrap(),
            case["expected"]["modelChoice"],
            "chosenShownCases modelChoice: {name}"
        );
    }

    // 11. withModelVersionCases (7 vectors) + newest_claude_of + effective_model_id helpers
    assert_eq!(newest_claude_of("opus"), Some("claude-opus-5-5"));
    assert_eq!(newest_claude_of("unknown"), None);
    let claude_models = agent_model_catalog::models_for("claude", None);
    assert_eq!(
        effective_model_id_of(&claude_models, "claude-opus-5-5"),
        Some("opus")
    );
    assert_eq!(effective_model_id(&claude_models, None), Some("sonnet"));
    assert_eq!(
        pane_menu_values(
            "claude",
            true,
            &claude_models,
            None,
            Some(&json!({ "model": "opus" }))
        ),
        Some(json!({ "model": "opus" }))
    );
    assert_eq!(
        next_pane_values(
            "claude",
            &claude_models,
            Some(&json!({ "model": "opus", "effort": "high" })),
            Some("opus"),
            "model",
            Some(&json!("sonnet")),
        ),
        Some(json!({ "model": "sonnet", "effort": "high" }))
    );

    let ver_cases = fixture["withModelVersionCases"].as_array().unwrap();
    assert_eq!(ver_cases.len(), 7);
    for (idx, case) in ver_cases.iter().enumerate() {
        let shown: ShownAgentModel = serde_json::from_value(case["shown"].clone()).unwrap();
        let reported: Vec<&str> = case["reported"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let actual = with_model_version(&shown, &reported);
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            case["expected"],
            "withModelVersionCases[{idx}]"
        );
    }
}

#[test]
fn differential_live_pick_action_chat_view_and_deferred_claude_match_vue() {
    let fixture = load_fixture();
    let cursor_models = fixture["cursorFixtureModels"].as_array().unwrap();
    let live_cases = fixture["livePickCases"].as_array().unwrap();
    assert_eq!(live_cases.len(), 28);

    assert!(toast_for_outcome(ModelPickOutcome::Saved, false).is_none());
    let err_toast = toast_for_outcome(ModelPickOutcome::Rejected, false).unwrap();
    assert_eq!(err_toast.kind, ToastKind::Error);
    assert!(err_toast.timeout.is_none());
    assert!(!err_toast.text.is_empty());

    let empty_prefs = BTreeMap::new();
    let empty_defaults = BTreeMap::new();

    for entry in live_cases {
        let sc = &entry["scenario"];
        let name = sc["name"].as_str().unwrap();
        // Raw JS `launchSig` in fixture must be rejected by `PaneModelNode` deserialization.
        if sc["node"]
            .get("launchSig")
            .and_then(Value::as_str)
            .is_some()
        {
            assert!(
                serde_json::from_value::<PaneModelNode>(sc["node"].clone()).is_err(),
                "{name}: PaneModelNode must reject raw unhashed launchSig"
            );
        }
        let hashed_input = convert_fixture_node_to_hashed(&sc["node"]);
        let node: PaneModelNode = serde_json::from_value(hashed_input).unwrap();
        let _ = (node.detected, node.chat_view);
        let agent_model: Option<ShownAgentModel> = sc
            .get("agentModel")
            .cloned()
            .map(|v| serde_json::from_value(v).unwrap());
        let discovered: Option<&[Value]> = if sc.get("useCursorFixtureModels") == Some(&json!(true))
        {
            Some(cursor_models.as_slice())
        } else {
            sc.get("discovered")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
        };
        let option_id = sc["optionId"].as_str().unwrap();
        let value = sc.get("value").filter(|v| !v.is_null());
        let claude_outcome: Option<ClaudeSwitchOutcome> = sc
            .get("claudeOutcome")
            .cloned()
            .map(|v| serde_json::from_value(v).unwrap());
        let mode = sc.get("mode").and_then(Value::as_str).unwrap_or("pick");

        for (french, expected) in [(false, &entry["expectedEn"]), (true, &entry["expectedFr"])] {
            let loc = if french { "FR" } else { "EN" };
            let mut expected_node = convert_fixture_node_to_hashed(&expected["node"]);
            // Deliberate persistence boundary: the source's raw custom command
            // remains available to the runtime planner but is never exported.
            if let Some(object) = expected_node.as_object_mut() {
                object.remove("agentCommand");
            }
            let ctx = PanePickContext {
                is_agent: sc.get("isAgent").and_then(Value::as_bool).unwrap_or(true),
                pane_running: sc
                    .get("paneRunning")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                pane_busy: sc.get("paneBusy").and_then(Value::as_bool).unwrap_or(false),
                pending: sc.get("pending").and_then(Value::as_bool).unwrap_or(false),
                user_typing: sc
                    .get("userTyping")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                chat_view_available: sc
                    .get("chatViewAvailable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                discovered_models: discovered,
                agent_model: agent_model.as_ref(),
                agent_prefs: &empty_prefs,
                persisted_defaults: &empty_defaults,
                permissions: AgentPermissionMode::Manual,
                now_ms: 1_700_000_000_000,
                french,
            };

            match mode {
                "action" => {
                    let plan = plan_model_action(&node, &ctx, option_id);
                    if plan.outcome == Some(ModelPickOutcome::ActionSent) {
                        assert_eq!(plan.typed_commands.len(), 1);
                        let cmd0 = &plan.typed_commands[0];
                        assert!(matches!(
                            cmd0.delivery,
                            CommandDelivery::Write | CommandDelivery::Type
                        ));
                        assert!(cmd0.expected_model_label.is_none());
                        assert!(!cmd0.command.is_empty());
                    }
                    assert_eq!(
                        serde_json::to_value(plan.outcome).unwrap(),
                        expected["outcome"],
                        "{name} {loc} action outcome"
                    );
                    assert_eq!(
                        serde_json::to_value(&plan.typed_commands).unwrap(),
                        expected["typedCommands"],
                        "{name} {loc} action typedCommands"
                    );
                    assert_eq!(
                        serde_json::to_value(&plan.toast).unwrap(),
                        expected["toast"],
                        "{name} {loc} action toast"
                    );
                    assert_eq!(
                        serde_json::to_value(&plan.node).unwrap(),
                        expected_node,
                        "{name} {loc} action node"
                    );
                }
                "chat" => {
                    let plan =
                        plan_chat_view_set_option(&node, &ctx, option_id, value, claude_outcome);
                    let _ = (
                        plan.chat_result.ok,
                        plan.chat_result.picker,
                        &plan.chat_result.error,
                        plan.hold_claude_default_model,
                        &plan.pending_next_choice,
                    );
                    if let Some(ref sig) = plan.node.launch_sig {
                        assert!(valid_launch_signature(sig.as_str()));
                    }
                    assert_eq!(
                        serde_json::to_value(&plan.chat_result).unwrap(),
                        expected["chatResult"],
                        "{name} {loc} chatResult"
                    );
                    assert_eq!(
                        serde_json::to_value(&plan.typed_commands).unwrap(),
                        expected["typedCommands"],
                        "{name} {loc} chat typedCommands"
                    );
                    assert_eq!(
                        serde_json::to_value(&plan.node).unwrap(),
                        expected_node,
                        "{name} {loc} chat node"
                    );
                }
                _ => {
                    let plan = plan_model_pick(&node, &ctx, option_id, value, claude_outcome);
                    if let Some(ref sig) = plan.node.launch_sig {
                        assert!(valid_launch_signature(sig.as_str()));
                    }
                    assert_eq!(
                        serde_json::to_value(plan.outcome).unwrap(),
                        expected["outcome"],
                        "{name} {loc} outcome"
                    );
                    assert_eq!(
                        serde_json::to_value(&plan.typed_commands).unwrap(),
                        expected["typedCommands"],
                        "{name} {loc} typedCommands"
                    );
                    assert_eq!(
                        serde_json::to_value(&plan.toast).unwrap(),
                        expected["toast"],
                        "{name} {loc} toast"
                    );
                    assert_eq!(
                        plan.hold_claude_default_model,
                        expected["holdClaudeCalled"].as_bool().unwrap(),
                        "{name} {loc} holdClaudeCalled"
                    );
                    assert_eq!(
                        plan.launch_stale_after,
                        expected["launchStaleAfter"].as_bool().unwrap(),
                        "{name} {loc} launchStaleAfter"
                    );
                    assert_eq!(
                        serde_json::to_value(&plan.pane_menu_values_before).unwrap(),
                        expected["paneMenuValuesBefore"],
                        "{name} {loc} paneMenuValuesBefore"
                    );
                    assert_eq!(
                        serde_json::to_value(&plan.effective_id_before).unwrap(),
                        expected["effectiveIdBefore"],
                        "{name} {loc} effectiveIdBefore"
                    );
                    assert_eq!(
                        serde_json::to_value(&plan.node).unwrap(),
                        expected_node,
                        "{name} {loc} node"
                    );

                    // Verify 2-phase deferred Claude confirmation matches direct settlement 100%.
                    if let Some(settled) = claude_outcome {
                        let deferred = plan_model_pick(&node, &ctx, option_id, value, None);
                        assert_eq!(
                            deferred.outcome,
                            Some(ModelPickOutcome::AwaitClaudeConfirmation)
                        );
                        assert!(deferred.hold_claude_default_model);
                        assert!(deferred.pending_next_choice.is_some());
                        let (final_out, final_toast, final_stale, final_node) =
                            apply_claude_switch_outcome(
                                &deferred.node,
                                deferred.pending_next_choice,
                                settled,
                                &ctx,
                            );
                        assert_eq!(
                            Some(final_out),
                            plan.outcome,
                            "{name} {loc} deferred outcome"
                        );
                        assert_eq!(final_toast, plan.toast, "{name} {loc} deferred toast");
                        assert_eq!(
                            final_stale, plan.launch_stale_after,
                            "{name} {loc} deferred stale"
                        );
                        assert_eq!(final_node, plan.node, "{name} {loc} deferred node");
                    }
                }
            }
        }
    }
}

#[test]
fn opaque_launch_signature_enforces_sha256_and_censors_debug() {
    // Reject non-SHA256 inputs on parse, TryFrom, and PaneModelNode deserialization.
    assert!(LaunchSignature::parse("[\"claude\",\"\",[]]").is_none());
    assert!(LaunchSignature::parse("plaintext").is_none());
    assert!(LaunchSignature::parse(&"A".repeat(64)).is_none());
    assert!(LaunchSignature::parse(&"a".repeat(63)).is_none());
    assert!(LaunchSignature::parse(&"a".repeat(65)).is_none());
    assert!(LaunchSignature::try_from("[\"claude\",\"\",[]]").is_err());
    assert!(LaunchSignature::try_from("raw".to_owned()).is_err());
    assert!(
        serde_json::from_value::<PaneModelNode>(json!({
            "agentId": "claude",
            "launchSig": "[\"claude\",\"\",[]]"
        }))
        .is_err()
    );

    let mut env = BTreeMap::new();
    env.insert("TOKEN".into(), "sk-secret-999".into());
    let spec = LaunchSpec {
        command: "C:\\secret-cli\\grok.exe".into(),
        args: "--api-key sk-arg-888 -m grok-4.5 --reasoning-effort high".into(),
        env: env.clone(),
    };
    let sig = LaunchSignature::from_spec(&spec);
    assert_eq!(sig, LaunchSignature::of(&spec.command, &spec.args, &env));
    assert!(valid_launch_signature(&sig));
    assert_eq!(sig.as_ref(), sig.as_str());
    let hex_string = String::from(sig.clone());
    assert_eq!(sig, hex_string);
    assert_eq!(sig.to_string(), hex_string);
    assert_eq!(
        LaunchSignature::try_from(hex_string.as_str()),
        Ok(sig.clone())
    );
    assert_eq!(
        LaunchSignature::try_from(hex_string.clone()),
        Ok(sig.clone())
    );
    assert!(!hex_string.contains("secret-cli"));
    assert!(!hex_string.contains("sk-arg-888"));
    assert!(!hex_string.contains("sk-secret-999"));

    let sig_dbg = format!("{sig:?}");
    assert!(sig_dbg.starts_with("LaunchSignature("));
    assert!(!sig_dbg.contains("secret-cli"));

    // PaneModelNode Debug censors `agent_command` and keeps `launch_sig` opaque.
    let mut prefs_map = BTreeMap::new();
    prefs_map.insert(
        "grok".into(),
        AgentPrefEntry {
            enabled: Some(true),
            command: "C:\\secret-cli\\grok.exe".into(),
            args: String::new(),
            env: "TOKEN=sk-secret-999".into(),
        },
    );
    let empty_defaults = BTreeMap::new();
    let ctx = PanePickContext {
        is_agent: true,
        pane_running: true,
        pane_busy: false,
        pending: false,
        user_typing: false,
        chat_view_available: true,
        discovered_models: None,
        agent_model: None,
        agent_prefs: &prefs_map,
        persisted_defaults: &empty_defaults,
        permissions: AgentPermissionMode::Manual,
        now_ms: 42_000,
        french: false,
    };

    let grok_init_spec = effective_agent_launch(
        "grok",
        "C:\\secret-cli\\grok.exe",
        prefs_map.get("grok"),
        AgentPermissionMode::Manual,
        Some(&json!({ "model": "grok-4.5", "effort": "high" })),
        Some(&agent_model_catalog::models_for("grok", None)),
    );
    let grok_node = PaneModelNode {
        agent_id: "grok".into(),
        agent_command: Some("C:\\secret-cli\\grok.exe".into()),
        session_options: Some(json!({ "model": "grok-4.5", "effort": "high" })),
        launch_sig: Some(launch_signature(&grok_init_spec)),
        ..PaneModelNode::default()
    };
    let node_dbg = format!("{grok_node:?}");
    assert!(node_dbg.contains("<redacted>"));
    assert!(!node_dbg.contains("secret-cli"));
    assert!(!node_dbg.contains("sk-secret-999"));

    let serialized_node = serde_json::to_string(&grok_node).unwrap();
    assert!(!serialized_node.contains("secret-cli"));
    assert!(!serialized_node.contains("sk-secret-999"));

    // Default `plan_model_pick` + `plan_chat_view_set_option` update SHA-256 `launch_sig` on live switch.
    let grok_next_spec = effective_agent_launch(
        "grok",
        "C:\\secret-cli\\grok.exe",
        prefs_map.get("grok"),
        AgentPermissionMode::Manual,
        Some(&json!({ "model": "grok-4.6", "effort": "high" })),
        Some(&agent_model_catalog::models_for("grok", None)),
    );
    let expected_next_sig = launch_signature(&grok_next_spec);

    let live_plan = plan_model_pick(&grok_node, &ctx, "model", Some(&json!("grok-4.6")), None);
    assert_eq!(live_plan.outcome, Some(ModelPickOutcome::Sent));
    assert!(!live_plan.launch_stale_after);
    assert_eq!(live_plan.node.launch_sig.as_ref(), Some(&expected_next_sig));

    let chat_plan =
        plan_chat_view_set_option(&grok_node, &ctx, "model", Some(&json!("grok-4.6")), None);
    assert!(chat_plan.chat_result.ok);
    assert_eq!(chat_plan.node.launch_sig.as_ref(), Some(&expected_next_sig));

    // Restart-only (`codex`) preserves initial SHA-256 `launch_sig` and sets `launch_stale_after = true`.
    let codex_init_sig = launch_signature(&effective_agent_launch(
        "codex",
        "codex",
        None,
        AgentPermissionMode::Manual,
        None,
        Some(&agent_model_catalog::models_for("codex", None)),
    ));
    let codex_node = PaneModelNode {
        agent_id: "codex".into(),
        agent_command: Some("codex".into()),
        launch_sig: Some(codex_init_sig.clone()),
        ..PaneModelNode::default()
    };
    let saved_plan = plan_model_pick(
        &codex_node,
        &ctx,
        "model",
        Some(&json!("gpt-5.6-sol")),
        None,
    );
    assert_eq!(saved_plan.outcome, Some(ModelPickOutcome::Saved));
    assert!(saved_plan.launch_stale_after);
    assert_eq!(saved_plan.node.launch_sig.as_ref(), Some(&codex_init_sig));

    // Observer Debug censors raw observed terminal chunks.
    let mut obs = ClaudeModelSwitchObserver::new("Opus 5.5", 5_000);
    obs.arm();
    let _ = obs.observe("sensitive-terminal-line-token", 100);
    let obs_dbg = format!("{obs:?}");
    assert!(obs_dbg.contains("observed_utf16_len"));
    assert!(!obs_dbg.contains("sensitive-terminal-line-token"));
}

#[test]
fn differential_claude_ansi_checks_and_stateful_observer_lifecycle() {
    let fixture = load_fixture();

    // 13. claudeSwitchCheckInputs (7 vectors)
    let checks = fixture["claudeSwitchCheckInputs"].as_array().unwrap();
    assert_eq!(checks.len(), 7);
    for case in checks {
        let name = case["name"].as_str().unwrap();
        let buf = case["buffer"].as_str().unwrap();
        let label = case["modelLabel"].as_str().unwrap();
        assert_eq!(
            strip_ansi(buf),
            case["stripped"].as_str().unwrap(),
            "{name} stripped"
        );
        assert_eq!(
            has_claude_model_switch_confirmation(buf),
            case["confirmation"].as_bool().unwrap(),
            "{name} confirmation"
        );
        assert_eq!(
            has_claude_model_switch_success(buf, label),
            case["success"].as_bool().unwrap(),
            "{name} success"
        );
        assert_eq!(
            has_claude_model_switch_rejection(buf),
            case["rejection"].as_bool().unwrap(),
            "{name} rejection"
        );
    }

    // Stateful observer: unarmed ignored -> arm + start_detection -> cached prompt -> split success
    let mut obs = ClaudeModelSwitchObserver::new("Opus", 0);
    assert_eq!(obs.deadline_ms(), None);
    assert_eq!(obs.outcome(), None);
    assert_eq!(
        obs.observe("Set model to Opus 5.5", 500),
        ClaudeObserverStep::Pending
    );

    obs.arm();
    obs.start_detection(1_000);
    assert_eq!(obs.deadline_ms(), Some(6_000));
    assert_eq!(obs.check_timeout(5_999), None);

    // Cached conversation confirmation prompt auto-submits once and resets timeout window.
    assert_eq!(
        obs.observe("\u{1b}[1mSwitch model?\u{1b}[0m\r\n", 2_200),
        ClaudeObserverStep::Pending
    );
    assert_eq!(
        obs.observe("This conversation is cached for the current model.", 2_300),
        ClaudeObserverStep::SubmitEnter
    );
    assert_eq!(obs.deadline_ms(), Some(7_300));
    // Re-emitting the prompt never triggers a second Enter submission.
    assert_eq!(
        obs.observe(
            "Switch model? This conversation is cached for the current model.",
            2_400
        ),
        ClaudeObserverStep::Pending
    );

    // Sliding UTF-16 cap (64 KiB code units) + surrogate-pair emoji safety + split ANSI echo.
    let filler = "🔥".repeat(MAX_CLAUDE_OBSERVED_UTF16);
    assert_eq!(obs.observe(&filler, 3_000), ClaudeObserverStep::Pending);
    assert_eq!(
        obs.observe("\u{1b}[2KSet mod", 3_100),
        ClaudeObserverStep::Pending
    );
    assert_eq!(
        obs.observe("el to Opus 5.5 (1M context)\r\n", 3_200),
        ClaudeObserverStep::Settled(ClaudeSwitchOutcome::Applied)
    );
    assert_eq!(obs.outcome(), Some(ClaudeSwitchOutcome::Applied));
    assert_eq!(obs.deadline_ms(), None);

    // Timeout expiration -> Unknown; dispose -> Unknown.
    let mut timeout_obs = ClaudeModelSwitchObserver::new("Sonnet", 2_500);
    timeout_obs.arm();
    timeout_obs.start_detection(10_000);
    assert_eq!(timeout_obs.check_timeout(12_499), None);
    assert_eq!(
        timeout_obs.check_timeout(12_500),
        Some(ClaudeSwitchOutcome::Unknown)
    );

    let mut disposed = ClaudeModelSwitchObserver::new("Haiku", 5_000);
    disposed.arm();
    assert_eq!(disposed.dispose(), ClaudeSwitchOutcome::Unknown);
    assert_eq!(
        disposed.observe("Set model to Haiku 4.5", 1_000),
        ClaudeObserverStep::Settled(ClaudeSwitchOutcome::Unknown)
    );
}

#[test]
fn differential_screen_readers_and_bounded_utf8_safety_match_vue() {
    let fixture = load_fixture();
    let cursor_models = fixture["cursorFixtureModels"].as_array().unwrap();

    // 14. cursorFilterCases (5 vectors)
    let filter_cases = fixture["cursorFilterCases"].as_array().unwrap();
    assert_eq!(filter_cases.len(), 5);
    for (idx, case) in filter_cases.iter().enumerate() {
        let row = (!case["row"].is_null()).then_some(&case["row"]);
        assert_eq!(
            cursor_picker_filter(row),
            case["expected"].as_str().unwrap(),
            "cursorFilterCases[{idx}]"
        );
    }

    // 15. cursorScreenCases (5 vectors) + direct cursor_model_from_status_line & CursorContextOnScreen variants
    let status_parsed =
        cursor_model_from_status_line("Codex 5.3 272K High Fast", cursor_models).unwrap();
    assert_eq!(status_parsed.model, "gpt-5.3-codex-high-fast");
    assert_eq!(status_parsed.row, "gpt-5.3-codex");
    assert_eq!(status_parsed.name, "Codex 5.3 Fast");
    assert_eq!(status_parsed.effort.as_deref(), Some("high"));
    assert!(status_parsed.fast_mode);
    assert!(!status_parsed.thinking);

    let cursor_screen_cases = fixture["cursorScreenCases"].as_array().unwrap();
    assert_eq!(cursor_screen_cases.len(), 5);
    for case in cursor_screen_cases {
        let name = case["name"].as_str().unwrap();
        let lines = json_lines(&case["lines"]);
        assert_eq!(
            cursor_picker_shown(&lines),
            case["pickerShown"].as_bool().unwrap(),
            "cursorPickerShown: {name}"
        );
        assert_eq!(
            serde_json::to_value(cursor_model_on_screen(&lines, cursor_models)).unwrap(),
            case["modelOnScreen"],
            "cursorModelOnScreen: {name}"
        );
        let ctx_screen = cursor_context_on_screen(&lines, cursor_models);
        match ctx_screen {
            Some(CursorContextOnScreen::Usage {
                used_tokens,
                window_tokens,
                percentage,
            }) => assert!(used_tokens > 0 && window_tokens > 0 && percentage > 0),
            Some(none_yet @ CursorContextOnScreen::NoneYet) => {
                assert_eq!(none_yet.to_value(), json!({ "none": true }));
            }
            None => {}
        }
        assert_eq!(
            serde_json::to_value(ctx_screen.map(CursorContextOnScreen::to_value)).unwrap(),
            case["contextOnScreen"],
            "cursorContextOnScreen: {name}"
        );
    }

    // 16. codexScreenCases (5 vectors)
    let codex_cases = fixture["codexScreenCases"].as_array().unwrap();
    assert_eq!(codex_cases.len(), 5);
    for case in codex_cases {
        let name = case["name"].as_str().unwrap();
        let lines = json_lines(&case["lines"]);
        let models = case["models"].as_array().unwrap();
        let actual = codex_model_on_screen(&lines, models);
        if let Some(ref m) = actual {
            assert!(!m.model.is_empty());
            let _ = (&m.name, &m.effort);
        }
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            case["expected"],
            "codexScreenCases: {name}"
        );
    }

    // 17 & 18. antigravityScreenCases (4 vectors) & antigravityListedCases (5 vectors)
    let agy_screen_cases = fixture["antigravityScreenCases"].as_array().unwrap();
    assert_eq!(agy_screen_cases.len(), 4);
    for case in agy_screen_cases {
        let name = case["name"].as_str().unwrap();
        let lines = json_lines(&case["lines"]);
        let models = case["models"].as_array().unwrap();
        assert_eq!(
            serde_json::to_value(antigravity_model_on_screen(&lines, models)).unwrap(),
            case["expected"],
            "antigravityScreenCases: {name}"
        );
    }

    let agy_listed_cases = fixture["antigravityListedCases"].as_array().unwrap();
    assert_eq!(agy_listed_cases.len(), 5);
    for (idx, case) in agy_listed_cases.iter().enumerate() {
        let models = case["models"].as_array().unwrap();
        let id_or_label = case["idOrLabel"].as_str().unwrap();
        let effort = case["effort"].as_str();
        assert_eq!(
            serde_json::to_value(antigravity_listed(models, id_or_label, effort)).unwrap(),
            case["expected"],
            "antigravityListedCases[{idx}]"
        );
    }

    // 19. screenModelCases (5 vectors) + model_in_line
    assert_eq!(
        model_in_line("Status · anthropic/claude-sonnet-4-5 · ready").as_deref(),
        Some("anthropic/claude-sonnet-4-5")
    );
    let screen_cases = fixture["screenModelCases"].as_array().unwrap();
    assert_eq!(screen_cases.len(), 5);
    for case in screen_cases {
        let name = case["name"].as_str().unwrap();
        let bottom = json_lines(&case["bottom"]);
        let top = json_lines(&case["top"]);
        assert_eq!(
            model_from_screen(&bottom, &top).as_deref(),
            case["expected"].as_str(),
            "screenModelCases: {name}"
        );
    }

    // 20. museFooterCases (3 vectors) & switchLineCases (4 vectors)
    let muse_cases = fixture["museFooterCases"].as_array().unwrap();
    assert_eq!(muse_cases.len(), 3);
    for (idx, case) in muse_cases.iter().enumerate() {
        let lines = json_lines(&case["lines"]);
        let actual = muse_footer_model(&lines);
        if let Some(ref mf) = actual {
            assert!(!mf.model.is_empty() && !mf.effort.is_empty());
        }
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            case["expected"],
            "museFooterCases[{idx}]"
        );
    }

    let switch_cases = fixture["switchLineCases"].as_array().unwrap();
    assert_eq!(switch_cases.len(), 4);
    for (idx, case) in switch_cases.iter().enumerate() {
        let line = case["line"].as_str().unwrap();
        assert_eq!(
            model_from_switch_line(line).as_deref(),
            case["expected"].as_str(),
            "switchLineCases[{idx}]"
        );
    }

    // Boundedness & multi-byte UTF-8 non-ASCII safety checks:
    // - Lines beyond `MAX_SCREEN_LINES` outside the bounded tail/head are ignored.
    let empty_lines: [&str; 0] = [];
    let mut deep_bottom = vec!["status | kimi-k2.5-preview | idle".to_owned()];
    deep_bottom.extend((0..MAX_SCREEN_LINES).map(|_| "─── • • • 🔥🚀✨ ───".to_owned()));
    assert_eq!(model_from_screen(&deep_bottom, &empty_lines), None);

    let mut deep_top = (0..MAX_SCREEN_LINES)
        .map(|_| "─── • • • 🔥🚀✨ ───".to_owned())
        .collect::<Vec<_>>();
    deep_top.push("Welcome (o3-mini)".to_owned());
    assert_eq!(model_from_screen(&empty_lines, &deep_top), None);

    // - Overlong single lines (`> MAX_SCREEN_LINE_UTF16` and `> MAX_SWITCH_LINE_UTF16`) are rejected.
    let overlong_screen = format!("gpt-5.5 {}", "x".repeat(MAX_SCREEN_LINE_UTF16));
    assert_eq!(model_in_line(&overlong_screen), None);
    let overlong_switch = format!("Set model to opus {}", "x".repeat(MAX_SWITCH_LINE_UTF16));
    assert_eq!(model_from_switch_line(&overlong_switch), None);

    // - Non-ASCII box-drawing / bullet / emoji strings never panic at non-char byte indices.
    for multibyte in ["───", "••••••", "🔥🚀✨", "claude-───", "opus───"]
    {
        let _ = model_label(multibyte);
        let shown = ShownAgentModel {
            model: multibyte.into(),
            name: None,
            effort: None,
            chosen_effort: None,
            source: ModelSource::Screen,
            at: None,
        };
        let _ = with_model_version(&shown, &[multibyte]);
        let _ = model_in_line(multibyte);
    }
}
