// Adapted from Tessel shared/antigravityModels.js / Orca.
// Copyright (c) 2026 Lovecast Inc. MIT; see THIRD_PARTY_NOTICES.md.
//! Antigravity lists effort variants separately. Only combine matching labels.
use crate::agent_env::{js_space, trim};
use serde_json::{Value, json};

const ORDER: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

pub fn label_parts(label: &str) -> Option<(String, Option<&'static str>)> {
    let text = label
        .split(js_space)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() || text.encode_utf16().count() > 120 {
        return None;
    }
    let mut name = text.as_str();
    let mut effort = None;
    if let Some(without_close) = text.strip_suffix(')')
        && let Some((before, inside)) = without_close.rsplit_once('(')
        && !trim(before).is_empty()
        && !inside.contains(['(', ')'])
        && (1..=20).contains(&inside.encode_utf16().count())
    {
        effort = match trim(inside).to_ascii_lowercase().as_str() {
            "low" => Some("low"),
            "medium" => Some("medium"),
            "high" => Some("high"),
            "extra high" | "xhigh" => Some("xhigh"),
            "max" | "maximum" => Some("max"),
            _ => None,
        };
        if effort.is_some() {
            name = before.trim_end();
        }
    }
    let clean: String = name.chars().filter(|c| !matches!(c, '(' | ')')).collect();
    if !(1..=80).contains(&clean.len())
        || !clean.as_bytes()[0].is_ascii_alphanumeric()
        || !clean
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_ .+:/-".contains(&b))
    {
        return None;
    }
    Some((name.to_owned(), effort))
}

fn variant(row: &Value) -> Option<(String, &'static str, String)> {
    let id = row["id"].as_str()?;
    let (name, effort) = label_parts(row["label"].as_str().unwrap_or_default())?;
    let effort = effort?;
    let base = id.strip_suffix(&format!("-{effort}"))?;
    if base.is_empty() {
        return None;
    }
    Some((base.to_owned(), effort, name))
}

pub fn group(rows: &[Value]) -> Vec<Value> {
    let variants: Vec<_> = rows.iter().map(variant).collect();
    let mut out: Vec<Value> = Vec::new();
    let mut groups: Vec<(String, usize)> = Vec::new();
    for (row, variant) in rows.iter().zip(&variants) {
        if !row["id"].is_string() {
            continue;
        }
        let grouped = variant.as_ref().filter(|v| {
            variants
                .iter()
                .flatten()
                .filter(|other| other.0 == v.0)
                .count()
                >= 2
        });
        let Some((base, effort, name)) = grouped else {
            out.push(row.clone());
            continue;
        };
        let index = if let Some((_, index)) = groups.iter().find(|(key, _)| key == base) {
            *index
        } else {
            let index = out.len();
            groups.push((base.clone(), index));
            out.push(json!({"id":base,"label":name,"efforts":[]}));
            index
        };
        let efforts = out[index]["efforts"].as_array_mut().unwrap();
        if !efforts.iter().any(|v| v == effort) {
            efforts.push(json!(effort));
        }
        if row["isDefault"] == true {
            out[index]["isDefault"] = json!(true);
        }
    }
    for (_, index) in groups {
        out[index]["efforts"]
            .as_array_mut()
            .unwrap()
            .sort_by_key(|v| ORDER.iter().position(|id| v == id));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_grouping_matches_labels_singletons_and_effort_mismatches() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/agent_model_antigravity.json"
        ))
        .unwrap();
        for case in fixture["labels"].as_array().unwrap() {
            let actual = label_parts(case["input"].as_str().unwrap())
                .map(|(name, effort)| json!({"name":name,"effort":effort}));
            assert_eq!(
                actual.unwrap_or(Value::Null),
                case["expected"],
                "{}",
                case["input"]
            );
        }
        for case in fixture["groups"].as_array().unwrap() {
            assert_eq!(
                group(case["input"].as_array().unwrap()),
                *case["expected"].as_array().unwrap()
            );
        }
    }
}
