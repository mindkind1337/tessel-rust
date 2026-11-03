// Adapted from Tessel agentSessionOptions.js / Orca.
// Copyright (c) 2026 Lovecast Inc. MIT; see THIRD_PARTY_NOTICES.md.
//! Model-picker metadata copied from the Vue catalogue, never executable JavaScript.
//! Launch/composition functions are deliberately not represented by this data.
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub models: Vec<Value>,
    #[serde(default)]
    pub unknown_model_options: Vec<Value>,
    #[serde(default)]
    pub discovered_models_are_authoritative: bool,
    #[serde(default)]
    pub probed: bool,
    #[serde(default)]
    pub default_model_is_cli_default: bool,
    pub model_apply: Value,
}
#[derive(Deserialize)]
struct Source {
    catalogs: BTreeMap<String, Catalog>,
}

pub fn all() -> &'static BTreeMap<String, Catalog> {
    static DATA: OnceLock<Source> = OnceLock::new();
    &DATA
        .get_or_init(|| {
            serde_json::from_str(include_str!("agent_model_catalog.json"))
                .expect("checked-in model catalogue")
        })
        .catalogs
}
pub fn find(agent: &str) -> Option<&'static Catalog> {
    all().get(agent)
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
        _ => true,
    }
}
fn overlay(seed: &Value, live: &Value) -> Value {
    let mut out = seed.as_object().cloned().unwrap_or_default();
    if let Some(live) = live.as_object() {
        out.extend(live.clone());
    }
    Value::Object(out)
}

/// Keep source ordering and option menus while overlaying CLI discovery.
/// Claude discovery replaces the seed. Authoritative providers decide membership.
pub fn models_for(agent: &str, discovered: Option<&[Value]>) -> Vec<Value> {
    let Some(catalog) = find(agent) else {
        return Vec::new();
    };
    let Some(discovered) = discovered.filter(|models| !models.is_empty()) else {
        return catalog.models.clone();
    };
    if agent == "claude" {
        return discovered.to_vec();
    }
    if catalog.discovered_models_are_authoritative {
        let inherited = catalog
            .models
            .iter()
            .find(|m| truthy(&m["isDefault"]))
            .or(catalog.models.first())
            .map(|m| m["options"].clone())
            .unwrap_or_else(|| serde_json::json!([]));
        return discovered
            .iter()
            .map(|disc| {
                let seed = catalog.models.iter().find(|m| m["id"] == disc["id"]);
                let mut merged = seed.map_or_else(|| disc.clone(), |seed| overlay(seed, disc));
                let options = if let Some(seed) = seed {
                    if truthy(&disc["variants"]) {
                        disc["options"].clone()
                    } else {
                        seed["options"].clone()
                    }
                } else if disc["options"]
                    .as_array()
                    .is_some_and(|opts| !opts.is_empty())
                {
                    disc["options"].clone()
                } else if truthy(&disc["variants"]) {
                    serde_json::json!([])
                } else {
                    inherited.clone()
                };
                if let Some(map) = merged.as_object_mut() {
                    map.remove("isDefault");
                    if truthy(&disc["isDefault"]) {
                        map.insert("isDefault".into(), Value::Bool(true));
                    }
                    if options.is_null() {
                        map.remove("options");
                    } else {
                        map.insert("options".into(), options);
                    }
                }
                merged
            })
            .collect();
    }
    // JS Map keeps the first insertion's position and last duplicate's value.
    let mut remaining: Vec<Value> = Vec::new();
    for model in discovered {
        if let Some(old) = remaining.iter_mut().find(|old| old["id"] == model["id"]) {
            *old = model.clone();
        } else {
            remaining.push(model.clone());
        }
    }
    let mut merged: Vec<_> = catalog
        .models
        .iter()
        .map(|seed| {
            if let Some(index) = remaining.iter().position(|disc| disc["id"] == seed["id"]) {
                let disc = remaining.remove(index);
                let mut merged = overlay(seed, &disc);
                merged["options"] = seed["options"].clone();
                merged
            } else {
                seed.clone()
            }
        })
        .collect();
    merged.extend(remaining);
    merged
}

pub fn model_options<'a>(
    catalog: &'a Catalog,
    discovered: Option<&'a [Value]>,
    model: &str,
) -> &'a [Value] {
    if model.is_empty() {
        return &[];
    }
    discovered
        .unwrap_or(&catalog.models)
        .iter()
        .find(|m| m["id"] == model)
        .or_else(|| catalog.models.iter().find(|m| m["id"] == model))
        .map_or(&catalog.unknown_model_options, |m| {
            m["options"].as_array().map_or(&[], Vec::as_slice)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_catalogues_and_discovery_merge_match_reference() {
        assert_eq!(all().len(), 12);
        assert_eq!(all().values().map(|c| c.models.len()).sum::<usize>(), 39);
        assert!(find("not-an-agent").is_none());
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/agent_model_merge.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            assert_eq!(
                models_for(
                    case["agent"].as_str().unwrap(),
                    case["discovered"].as_array().map(Vec::as_slice)
                ),
                case["expected"].as_array().unwrap().clone(),
                "{}",
                case["name"]
            );
        }
        let codex = find("codex").unwrap();
        assert!(codex.discovered_models_are_authoritative);
        assert_eq!(model_options(codex, None, "unknown")[0]["id"], "effort");
        assert!(model_options(codex, None, "").is_empty());
    }
}
