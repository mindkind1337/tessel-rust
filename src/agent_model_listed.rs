// Adapted from Tessel shared/agentModelProbe.js / Orca.
// Copyright (c) 2026 Lovecast Inc. MIT; see THIRD_PARTY_NOTICES.md.
//! Convert sanitized discovery rows to picker metadata, without choosing defaults.
use serde_json::{Value, json};

/// Cursor's grouping is supplied separately; never expose its raw variants as
/// grouped rows while that conversion is unavailable. Callers retain seed data.
pub fn catalog(agent: &str, rows: &[Value]) -> Option<Vec<Value>> {
    if agent == "cursor" {
        return None;
    }
    let grouped;
    let rows = if agent == "antigravity" {
        grouped = crate::agent_model_antigravity::group(rows);
        &grouped
    } else {
        rows
    };
    Some(
        rows.iter()
            .map(|row| {
                let mut model = json!({"id":row["id"], "label":row["label"]});
                if let Some(description) = row["description"].as_str().filter(|s| !s.is_empty()) {
                    model["description"] = description.into();
                }
                if row["isDefault"] == true {
                    model["isDefault"] = true.into();
                }
                let levels_key = if agent == "antigravity" {
                    "efforts"
                } else {
                    "effortLevels"
                };
                let levels = row[levels_key].as_array().map_or(&[][..], Vec::as_slice);
                let options = if agent == "codex" && levels.is_empty() {
                    Vec::new()
                } else {
                    crate::agent_model_options::for_levels(
                        agent,
                        levels,
                        row["defaultEffort"].as_str(),
                        row["supportsFastMode"] == true,
                    )
                };
                model["options"] = options.into();
                if agent == "antigravity" && row["efforts"].is_array() {
                    model["efforts"] = row["efforts"].clone();
                }
                model
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn listed_catalogs_match_reference() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/agent_model_listed.json"))
                .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            assert_eq!(
                catalog(
                    case["agent"].as_str().unwrap(),
                    case["rows"].as_array().unwrap()
                )
                .unwrap(),
                *case["expected"].as_array().unwrap(),
                "{}",
                case["name"]
            );
        }
        assert!(catalog("cursor", &[]).is_none());
    }
}
