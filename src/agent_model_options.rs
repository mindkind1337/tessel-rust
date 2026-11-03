// Adapted from Tessel shared/agentSessionOptions.js / Orca.
// Copyright (c) 2026 Lovecast Inc. MIT; see THIRD_PARTY_NOTICES.md.
//! Option metadata for discovered models. Does not imply a user selection.
use serde_json::{Value, json};

/// Build advertised effort levels in source UI order. Codex preserves the
/// source fallback to minimal..xhigh when none of its levels are recognized.
/// Antigravity callers must pass the efforts of a grouped model, not a variant.
pub fn for_levels(agent: &str, levels: &[Value], default: Option<&str>, fast: bool) -> Vec<Value> {
    let order: &[&str] = match agent {
        "claude" | "antigravity" => &["low", "medium", "high", "xhigh", "max"],
        "codex" => &["minimal", "low", "medium", "high", "xhigh", "max", "ultra"],
        "muse" => &[
            "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
        ],
        "pi" => &["off", "low", "medium", "high", "xhigh"],
        _ => return Vec::new(),
    };
    let mut selected: Vec<_> = order
        .iter()
        .copied()
        .filter(|id| levels.iter().any(|v| v.as_str() == Some(id)))
        .collect();
    let codex_fallback = agent == "codex" && selected.is_empty();
    if codex_fallback {
        selected = vec!["minimal", "low", "medium", "high", "xhigh"];
    }
    let mut out = Vec::new();
    if !selected.is_empty() && (agent != "antigravity" || selected.len() >= 2) {
        let fallback = match agent {
            "claude" | "muse" => "high",
            "pi" => "low",
            _ => "medium",
        };
        let value = if matches!(agent, "codex" | "muse")
            && !codex_fallback
            && default.is_some_and(|d| selected.contains(&d))
        {
            default.unwrap()
        } else if agent == "pi" || selected.contains(&fallback) {
            fallback
        } else {
            selected[0]
        };
        let choices: Vec<_> = selected.iter().map(|id| json!({"value":id, "label": match *id {
            "off"=>"Off", "none"=>"None", "minimal"=>"Minimal", "low"=>"Low", "medium"=>"Medium", "high"=>"High", "xhigh"=>"Extra high", "max"=>"Max", _=>"Ultra"
        }})).collect();
        let mid_session = match agent {
            "codex" => json!({"kind":"agent-picker","command":"/model","delivery":"type"}),
            "pi" => json!({"kind":"agent-picker","command":"/thinking"}),
            _ => json!({"kind":"command"}),
        };
        out.push(json!({"id":"effort", "label":if agent == "claude" {"Effort"} else {"Reasoning effort"}, "category":"thought_level", "kind":{"type":"select","choices":choices,"defaultValue":value},"apply":{"midSession":mid_session}}));
    }
    if agent == "claude" && fast {
        out.push(json!({"id":"fastMode","label":"Fast mode","category":"mode","kind":{"type":"boolean","defaultValue":false},"apply":{"midSession":{"kind":"toggle-command","command":"/fast"}}}));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dynamic_options_match_source_for_all_level_subsets() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/agent_model_options.json"))
                .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            assert_eq!(
                for_levels(
                    case["agent"].as_str().unwrap(),
                    case["levels"].as_array().unwrap(),
                    case["default"].as_str(),
                    case["fast"].as_bool().unwrap()
                ),
                *case["expected"].as_array().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
}
