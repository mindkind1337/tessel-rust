// Adapted from Tessel shared/agentModelProbe.js / Orca.
// Copyright (c) 2026 Lovecast Inc. MIT; see THIRD_PARTY_NOTICES.md.
//! Pure parsers for model discovery. No provider invocation or credentials.
use crate::agent_env::{js_space, trim};
use serde_json::{Value, json};
use std::collections::HashSet;

pub const MAX_OUTPUT: usize = 4 * 1024 * 1024;
fn len(s: &str) -> usize {
    s.encode_utf16().count()
}
fn cut(s: &str, n: usize) -> String {
    String::from_utf16_lossy(&s.encode_utf16().take(n).collect::<Vec<_>>())
}
fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v[key].as_str()
}
fn rows(v: &Value) -> &[Value] {
    v["models"].as_array().map_or(&[], Vec::as_slice)
}
fn levels(v: &Value) -> Vec<Value> {
    v.as_array().map_or_else(Vec::new, |a| {
        a.iter().filter(|e| e.is_string()).cloned().collect()
    })
}
fn safe(s: &str) -> bool {
    crate::agent_arg_options::safe_session_value(s)
}
fn lines(s: &str) -> impl Iterator<Item = &str> {
    s.split(['\r', '\n'])
}

pub fn label_from_id(id: &str) -> String {
    id.split(['/', '-'])
        .filter(|p| !p.is_empty())
        .map(|p| {
            if p.eq_ignore_ascii_case("gpt") {
                return "GPT".into();
            }
            if len(p) <= 3 && p.as_bytes()[0].is_ascii_digit() {
                return p.to_uppercase();
            }
            let mut chars = p.chars();
            chars.next().map_or_else(String::new, |first| {
                format!("{}{}", first.to_uppercase(), chars.as_str())
            })
        })
        .collect::<Vec<String>>()
        .join(" ")
}
pub fn opencode_label(id: &str) -> String {
    match id.split_once('/') {
        Some((provider, model)) if provider != "opencode" => {
            format!("{} ({provider})", label_from_id(model))
        }
        Some((_, model)) => label_from_id(model),
        None => label_from_id(id),
    }
}

pub fn parse(agent: &str, output: &str) -> Vec<Value> {
    match agent {
        "claude" => claude(output),
        "codex" => codex(output),
        "muse" => muse(output),
        "grok" => grok(output),
        "opencode" | "pi" | "cursor" | "antigravity" => line_models(agent, output),
        _ => Vec::new(),
    }
}
fn claude(output: &str) -> Vec<Value> {
    for raw in output.split('\n') {
        let line = trim(raw);
        if !line.starts_with('{') || !line.contains("control_response") || len(line) > 512 * 1024 {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value["type"] != "control_response" || value["response"]["subtype"] != "success" {
            continue;
        }
        let Some(models) = value["response"]["response"]["models"].as_array() else {
            continue;
        };
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for entry in models.iter().take(200) {
            let id = trim(text(entry, "value").unwrap_or(""));
            if id.is_empty() || id == "default" || entry["disabled"] == true || !seen.insert(id) {
                continue;
            }
            let label = text(entry, "displayName")
                .filter(|s| !trim(s).is_empty())
                .unwrap_or(id);
            let effort = if entry["supportsEffort"] == true {
                levels(&entry["supportedEffortLevels"])
            } else {
                Vec::new()
            };
            let mut row = json!({"id":id,"label":label,"effortLevels":effort,"supportsFastMode":entry["supportsFastMode"] == true});
            if let Some(d) = text(entry, "description").filter(|s| !trim(s).is_empty()) {
                row["description"] = d.into();
            }
            out.push(row);
        }
        if !out.is_empty() {
            return out;
        }
    }
    Vec::new()
}
fn codex(output: &str) -> Vec<Value> {
    if len(output) > MAX_OUTPUT {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    rows(&value)
        .iter()
        .take(300)
        .filter_map(|model| {
            let id = text(model, "slug").filter(|s| !s.is_empty())?;
            let label = text(model, "display_name").filter(|s| !s.is_empty())?;
            if model["visibility"] == "hide" || !seen.insert(id) {
                return None;
            }
            let efforts: Vec<_> = model["supported_reasoning_levels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| text(e, "effort").filter(|s| !s.is_empty()))
                .collect();
            let mut row = json!({"id":id,"label":label,"effortLevels":efforts});
            if let Some(d) = text(model, "description")
                .map(trim)
                .filter(|s| !s.is_empty())
            {
                row["description"] = d.into();
            }
            if let Some(d) = text(model, "default_reasoning_level") {
                row["defaultEffort"] = d.into();
            }
            Some(row)
        })
        .collect()
}
fn muse(output: &str) -> Vec<Value> {
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return Vec::new();
    };
    let allowed = [
        "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
    ];
    let mut seen = HashSet::new();
    rows(&value)
        .iter()
        .take(300)
        .filter_map(|m| {
            let id = trim(text(m, "modelId").unwrap_or(""));
            if !safe(id) || !seen.insert(id) {
                return None;
            }
            let label = text(m, "displayLabel")
                .map(trim)
                .filter(|s| !s.is_empty())
                .map(|s| cut(s, 120))
                .unwrap_or_else(|| label_from_id(id));
            let efforts: Vec<Value> = if let Some(variants) = m["variants"].as_array() {
                variants.clone()
            } else {
                m["reasoningEffortVariants"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|v| v["tier"].clone())
                    .collect()
            };
            let efforts: Vec<_> = efforts
                .into_iter()
                .filter(|e| e.as_str().is_some_and(|s| allowed.contains(&s)))
                .collect();
            let mut row = json!({"id":id,"label":label});
            if !efforts.is_empty() {
                row["effortLevels"] = efforts.into();
            }
            if let Some(d) = text(m, "description").map(trim).filter(|s| !s.is_empty()) {
                row["description"] = cut(d, 300).into();
            }
            if text(m, "defaultReasoningEffort").is_some_and(|s| allowed.contains(&s)) {
                row["defaultEffort"] = m["defaultReasoningEffort"].clone();
            }
            if m["isDefault"] == true {
                row["isDefault"] = true.into();
            }
            Some(row)
        })
        .collect()
}
fn grok(output: &str) -> Vec<Value> {
    let mut lines = output.split_inclusive('\n').map(|raw| {
        raw.strip_suffix("\r\n")
            .or_else(|| raw.strip_suffix('\n'))
            .unwrap_or(raw)
    });
    if !lines.any(|s| trim(s) == "Available models:") {
        return Vec::new();
    }
    let mut out: Vec<Value> = Vec::new();
    for raw in lines {
        let line = raw.trim_start_matches(js_space);
        if trim(line).is_empty() && !out.is_empty() {
            break;
        }
        let Some(rest) = line
            .strip_prefix(['*', '-'])
            .filter(|s| s.starts_with(js_space))
        else {
            continue;
        };
        let rest = rest.trim_start_matches(js_space);
        if rest.contains(['\r', '\u{2028}', '\u{2029}']) {
            continue;
        }
        let end = rest.find(|c| js_space(c) || c == '(').unwrap_or(rest.len());
        let id = &rest[..end];
        if id.is_empty() {
            continue;
        }
        let default = rest[end..].contains("(default)");
        if let Some(row) = out.iter_mut().find(|r| r["id"] == id) {
            if default {
                row["isDefault"] = true.into();
            }
        } else {
            let mut row = json!({"id":id,"label":label_from_id(id)});
            if default {
                row["isDefault"] = true.into();
            }
            out.push(row);
        }
    }
    out
}
fn cursor_line(line: &str) -> Option<Value> {
    let end = line.find(js_space)?;
    let id = &line[..end];
    let rest = line[end..].trim_start_matches(js_space).strip_prefix('-')?;
    if !rest.starts_with(js_space) {
        return None;
    }
    let mut label = rest.trim_start_matches(js_space).to_owned();
    if label.contains(['\u{2028}', '\u{2029}']) {
        return None;
    }
    let mut default = false;
    // Source strips the first parenthesized current/default marker only.
    for (start, _) in label.match_indices('(') {
        let Some(end) = label[start..].find(')') else {
            continue;
        };
        let mark = &label[start + 1..start + end];
        let parts: Vec<_> = mark.split(',').collect();
        if parts.iter().enumerate().all(|(i, p)| {
            matches!(
                if i == 0 {
                    *p
                } else {
                    p.trim_start_matches(js_space)
                },
                "current" | "default"
            )
        }) {
            default = mark.contains("default");
            let before = label[..start].trim_end_matches(js_space);
            label = format!("{}{}", before, &label[start + end + 1..]);
            break;
        }
    }
    let label = trim(&label);
    if label.is_empty() {
        return None;
    }
    let mut row = json!({"id":id,"label":label});
    if default {
        row["isDefault"] = true.into();
    }
    Some(row)
}
fn line_models(agent: &str, output: &str) -> Vec<Value> {
    if len(output) > MAX_OUTPUT {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in lines(output) {
        if agent != "opencode" && len(raw) > 4096 {
            continue;
        }
        let line = trim(raw);
        let row = match agent {
            "opencode" => {
                let Some((provider, model)) = line.split_once('/') else {
                    continue;
                };
                if !safe(line)
                    || provider.is_empty()
                    || !provider
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.@+-".contains(&b))
                    || model.is_empty()
                    || model.len() > 150
                {
                    continue;
                }
                Some(json!({"id":line,"label":opencode_label(line)}))
            }
            "pi" => {
                let fields: Vec<_> = line
                    .split(js_space)
                    .filter(|s| !s.is_empty())
                    .take(6)
                    .collect();
                if fields.len() != 6
                    || fields[0].eq_ignore_ascii_case("provider")
                    || !(fields[4].eq_ignore_ascii_case("yes")
                        || fields[4].eq_ignore_ascii_case("no"))
                {
                    continue;
                }
                let id = format!("{}/{}", fields[0], fields[1]);
                Some(
                    json!({"label":label_from_id(&id),"id":id,"effortLevels":if fields[4].eq_ignore_ascii_case("yes") { vec!["off","low","medium","high","xhigh"] } else { vec![] }}),
                )
            }
            "cursor" => cursor_line(line),
            "antigravity" => {
                let fields: Vec<_> = line.split('\t').collect();
                if fields.len() != 2
                    || fields[0].eq_ignore_ascii_case("id")
                    || fields[0].eq_ignore_ascii_case("model")
                    || trim(fields[1]).is_empty()
                {
                    continue;
                }
                Some(json!({"id":fields[0],"label":trim(fields[1])}))
            }
            _ => None,
        };
        let Some(mut row) = row else {
            continue;
        };
        let id = row["id"].as_str().unwrap_or("");
        if !safe(id) || !seen.insert(id.to_owned()) {
            continue;
        }
        if agent != "opencode" {
            row["label"] = cut(row["label"].as_str().unwrap_or(""), 120).into();
        }
        out.push(row);
        if out.len() == 300 {
            break;
        }
    }
    out
}

/// Sanitize persisted/IPC rows before they reach a picker. Source keeps only
/// supported metadata, at most 300 rows; launch additionally rejects flag IDs.
pub fn valid_listed(rows: &Value) -> Vec<Value> {
    let Some(rows) = rows.as_array() else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let effort = |v: &Value| {
        v.as_str().is_some_and(|s| {
            (1..=20).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase())
        })
    };
    let keep_text = |v: &Value, limit| {
        v.as_str()
            .filter(|s| !trim(s).is_empty())
            .map(|s| cut(s, limit))
    };
    rows.iter().take(300).filter_map(|row| {
        let id = text(row,"id")?;
        if id.is_empty() || id.len()>160 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:/@[]=+-".contains(&b)) || !seen.insert(id) { return None; }
        let mut out = json!({"id":id,"label":keep_text(&row["label"],120).unwrap_or_else(|| id.to_owned())});
        if let Some(d) = keep_text(&row["description"],300) { out["description"] = d.into(); }
        if let Some(levels) = row["effortLevels"].as_array() { out["effortLevels"] = levels.iter().filter(|v| effort(v)).take(12).cloned().collect::<Vec<_>>().into(); }
        if effort(&row["defaultEffort"]) { out["defaultEffort"] = row["defaultEffort"].clone(); }
        for flag in ["supportsFastMode", "isDefault"] { if row[flag] == true { out[flag] = true.into(); } }
        Some(out)
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_eight_formats_match_vue_fixtures() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/agent_model_probe_parse.json"
        ))
        .unwrap();
        for case in fixture["sanitization"].as_array().unwrap() {
            assert_eq!(json!(valid_listed(&case["input"])), case["expected"]);
        }
        for case in fixture["cases"].as_array().unwrap() {
            let input = case["input"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    case["repeat"]
                        .as_str()
                        .unwrap()
                        .repeat(case["repeatCount"].as_u64().unwrap() as usize)
                });
            assert_eq!(
                json!(parse(case["agent"].as_str().unwrap(), &input)),
                case["expected"],
                "{}",
                case["name"]
            );
            assert_eq!(
                json!(valid_listed(&case["expected"])),
                case["sanitizedExpected"],
                "sanitized {}",
                case["name"]
            );
        }
    }
}
