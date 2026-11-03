// Adapted from Tessel agentSessionOptions.js / cursorModels.js and Orca.
// Copyright (c) 2026 Lovecast Inc. MIT; see THIRD_PARTY_NOTICES.md.
//! Explicit model choices to shell-safe launch flags. Untouched values add nothing.
use crate::{agent_arg_options as args, agent_model_catalog as catalog};
use serde_json::{Map, Value};

fn model_flag(agent: &str) -> (&'static str, &'static [&'static str]) {
    match agent {
        "amp" => ("--mode", &["--mode"]),
        "codex" | "gemini" | "grok" => ("-m", &["-m", "--model"]),
        "cursor" | "kimi" | "opencode" => ("--model", &["-m", "--model"]),
        _ => ("--model", &["--model"]),
    }
}
fn string(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}
fn safe_arg(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/@[]=+-".contains(&b))
}
fn effort_rank(value: &Value) -> i32 {
    ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
        .iter()
        .position(|e| value == *e)
        .map_or(-1, |i| i as i32)
}

fn cursor_variant(model: &Value, values: &Map<String, Value>) -> String {
    let Some(variants) = model["variants"].as_array().filter(|v| !v.is_empty()) else {
        return string(&model["id"]);
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
    // Doubled source cost avoids floating-point ties, preserving listed order.
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
    string(&best.unwrap().0["id"])
}

fn compose(
    agent: &str,
    model_id: &str,
    values: &Map<String, Value>,
    model: Option<&Value>,
) -> String {
    let effort = values.get("effort").and_then(Value::as_str);
    if agent == "antigravity" && effort.is_some_and(|e| !e.is_empty()) {
        for level in ["low", "medium", "high", "xhigh", "max"] {
            if let Some(base) = model_id.strip_suffix(&format!("-{level}")) {
                return base.into();
            }
        }
    }
    if agent != "cursor" || model_id == "auto" {
        return model_id.into();
    }
    if let Some(model) = model.filter(|m| m["variants"].is_array()) {
        return cursor_variant(model, values);
    }
    let mut result = model_id.to_owned();
    if model_id.starts_with("claude-") && values.get("thinking") == Some(&Value::Bool(true)) {
        result.push_str("-thinking");
    }
    if let Some(effort) = effort {
        result.push('-');
        result.push_str(effort);
    }
    if !model_id.starts_with("claude-") && values.get("fastMode") == Some(&Value::Bool(true)) {
        result.push_str("-fast");
    }
    result
}

/// Equivalent to sessionOptionLaunchText: only explicit values, user flags win.
/// `models` is the list shown by the picker, including CLI-discovered options.
pub fn text(agent: &str, values: &Value, user_args: &str, models: Option<&[Value]>) -> String {
    let Some(catalog) = catalog::find(agent) else {
        return String::new();
    };
    let Some(input) = values.as_object() else {
        return String::new();
    };
    let Some(model_id) = input
        .get("model")
        .and_then(Value::as_str)
        .filter(|id| args::safe_session_value(id))
    else {
        return String::new();
    };
    if input.iter().any(|(id, v)| {
        id != "model"
            && !v.is_null()
            && !v.is_boolean()
            && !v.as_str().is_some_and(args::safe_session_value)
    }) {
        return String::new();
    }
    let tokens = args::tokenize(user_args);
    let (flag, aliases) = model_flag(agent);
    if args::has_flag(&tokens, aliases) {
        return String::new();
    }
    let model = models
        .and_then(|rows| rows.iter().find(|m| m["id"] == model_id))
        .or_else(|| catalog.models.iter().find(|m| m["id"] == model_id));
    let options = model.map_or(catalog.unknown_model_options.as_slice(), |m| {
        m["options"].as_array().map_or(&[], Vec::as_slice)
    });
    let mut picked = Map::new();
    for option in options {
        let Some(id) = option["id"].as_str() else {
            continue;
        };
        if let Some(value) = input.get(id) {
            if model.is_none()
                && option["kind"]["type"] == "select"
                && !option["kind"]["choices"]
                    .as_array()
                    .is_some_and(|choices| choices.iter().any(|choice| choice["value"] == *value))
            {
                continue;
            }
            picked.insert(id.into(), value.clone());
        }
    }
    let mut result = vec![flag.to_owned(), compose(agent, model_id, &picked, model)];
    for option in options {
        let Some(id) = option["id"].as_str() else {
            continue;
        };
        let Some(value) = picked.get(id) else {
            continue;
        };
        if option["apply"]["composedIntoModel"] == true || id != "effort" {
            continue;
        }
        let (flag, aliases): (&str, &[&str]) = match agent {
            "claude" | "antigravity" | "amp" => ("--effort", &["--effort"]),
            "grok" | "muse" | "copilot" => {
                ("--reasoning-effort", &["--reasoning-effort", "--effort"])
            }
            "pi" => ("--thinking", &["--thinking"]),
            "codex" => ("-c", &[]),
            _ => continue,
        };
        if args::has_flag(&tokens, aliases) || (agent == "codex" && args::has_codex_effort(&tokens))
        {
            continue;
        }
        result.push(flag.into());
        result.push(if agent == "codex" {
            format!("model_reasoning_effort={}", string(value))
        } else {
            string(value)
        });
    }
    if result.iter().all(|arg| safe_arg(arg)) {
        result.join(" ")
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launch_text_matches_source_for_all_catalogues() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/agent_model_launch.json"))
                .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            assert_eq!(
                text(
                    case["agent"].as_str().unwrap(),
                    &case["values"],
                    case["userArgs"].as_str().unwrap(),
                    case["models"].as_array().map(Vec::as_slice)
                ),
                case["expected"].as_str().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
}
