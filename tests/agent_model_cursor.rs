//! Standalone differential tests for `src/agent_model_cursor.rs` across the full
//! `probe_parse -> cursor::group -> catalog::models_for -> values_on_listed_row -> launch::text`
//! pipeline against the reference 246-row `cursor-agent --list-models` fixture and synthetic cases.

#[allow(dead_code)]
#[path = "../src/agent_env.rs"]
mod agent_env;

#[allow(dead_code)]
#[path = "../src/agent_arg_options.rs"]
mod agent_arg_options;

#[allow(dead_code)]
#[path = "../src/agent_model_probe_parse.rs"]
mod agent_model_probe_parse;

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

#[path = "../src/agent_model_cursor.rs"]
mod agent_model_cursor;

use agent_model_cursor::{EFFORT_ORDER, ParsedId, group, parse_id, sort};
use agent_model_defaults_ui::{decompose_cursor_model, values_on_listed_row};
use serde_json::{Map, Value, json};
use std::collections::HashSet;

#[test]
fn end_to_end_cursor_probe_grouping_catalog_merge_decomposition_and_launch_match_vue() {
    assert_eq!(
        EFFORT_ORDER,
        ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
    );
    assert_eq!(
        parse_id("gpt-5.3-codex-high-fast"),
        ParsedId {
            base: "gpt-5.3-codex".into(),
            effort: "high",
            fast: true,
            thinking: false,
        }
    );

    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/agent_model_cursor.json")).unwrap();
    let real = &fixture["realList"];
    let raw_output = real["rawOutput"].as_str().unwrap();

    // 1. CLI output -> probe parser -> valid_listed sanitization.
    let parsed_rows = agent_model_probe_parse::parse("cursor", raw_output);
    assert_eq!(parsed_rows.len(), 246);
    assert_eq!(parsed_rows, *real["parsedRows"].as_array().unwrap());
    let sanitized_rows = agent_model_probe_parse::valid_listed(&Value::Array(parsed_rows.clone()));
    assert_eq!(sanitized_rows, parsed_rows);

    // 2. Grouped models (`group` and `sort` idempotence) -> 39 base rows.
    let grouped = group(&sanitized_rows);
    assert_eq!(grouped.len(), 39);
    assert_eq!(grouped, *real["grouped"].as_array().unwrap());
    assert_eq!(sort(&grouped), grouped);

    // 3. Authoritative catalog merge (`catalogModelsFor("cursor", grouped)`).
    let merged = agent_model_catalog::models_for("cursor", Some(&grouped));
    assert_eq!(merged.len(), 39);
    assert_eq!(merged, *real["mergedCatalog"].as_array().unwrap());

    // 4. Every single one of the 246 listed variant IDs decomposes onto its base row
    // and composes back to the exact listed `--model <variant_id>` launch text.
    let decomposed_expected = real["decomposed"].as_array().unwrap();
    assert_eq!(decomposed_expected.len(), 246);
    for entry in decomposed_expected {
        let variant_id = entry["id"].as_str().unwrap();
        assert!(
            decompose_cursor_model(&merged, variant_id).is_some(),
            "missing decompose_cursor_model for {variant_id}"
        );
        let raw_val = json!({ "model": variant_id });
        let shown = Value::Object(values_on_listed_row(&merged, Some(&raw_val)).unwrap());
        assert_eq!(shown, entry["shown"], "values_on_listed_row({variant_id})");
        let launch = agent_model_launch::text("cursor", &shown, "", Some(&merged));
        assert_eq!(
            launch,
            format!("--model {variant_id}"),
            "round-trip launch for {variant_id}"
        );
        assert_eq!(
            launch,
            entry["launchFromDecomposed"].as_str().unwrap(),
            "Vue parity launch for {variant_id}"
        );
    }

    // 5. Exhaustive cartesian product across all 39 models × efforts × fastMode × thinking:
    // `agent_model_launch::text` must NEVER emit an ID that Cursor did not list.
    let listed_ids: HashSet<&str> = parsed_rows
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    let effort_inputs: [Option<&str>; 9] = [
        None,
        Some("none"),
        Some("minimal"),
        Some("low"),
        Some("medium"),
        Some("high"),
        Some("xhigh"),
        Some("max"),
        Some("ultra"),
    ];
    let bool_inputs: [Option<bool>; 3] = [None, Some(true), Some(false)];

    for model in &merged {
        let model_id = model["id"].as_str().unwrap();
        for effort in effort_inputs {
            for fast_mode in bool_inputs {
                for thinking in bool_inputs {
                    let mut values = Map::new();
                    values.insert("model".into(), json!(model_id));
                    if let Some(e) = effort {
                        values.insert("effort".into(), json!(e));
                    }
                    if let Some(f) = fast_mode {
                        values.insert("fastMode".into(), json!(f));
                    }
                    if let Some(t) = thinking {
                        values.insert("thinking".into(), json!(t));
                    }
                    let launch = agent_model_launch::text(
                        "cursor",
                        &Value::Object(values),
                        "",
                        Some(&merged),
                    );
                    let emitted = launch
                        .strip_prefix("--model ")
                        .expect("non-empty --model launch flag");
                    assert!(
                        listed_ids.contains(emitted),
                        "unlisted Cursor model emitted for {model_id} ({effort:?}, {fast_mode:?}, {thinking:?}): {emitted}"
                    );
                }
            }
        }
    }

    // 6. Closest-variant fallback probes and user `--model` override parity.
    for probe in real["launchProbes"].as_array().unwrap() {
        let actual = agent_model_launch::text(
            "cursor",
            &probe["values"],
            probe["userArgs"].as_str().unwrap(),
            Some(&merged),
        );
        assert_eq!(
            actual,
            probe["expected"].as_str().unwrap(),
            "probe {:?}",
            probe["values"]
        );
    }
}
