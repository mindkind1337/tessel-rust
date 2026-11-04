// Adapted from Tessel shared/cursorModels.js / Orca.
// Copyright (c) 2026 Lovecast Inc. MIT; see THIRD_PARTY_NOTICES.md.
//! Group Cursor CLI variant model IDs into one picker row per base model.

use crate::agent_env::js_space;
use serde_json::{Map, Value, json};
use std::cmp::Ordering;

/// Canonical picker and tie-break order for Cursor effort levels (`CURSOR_EFFORT_ORDER`).
pub const EFFORT_ORDER: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

// Longest first: `"extra-high"` must match before `"high"`.
const EFFORT_SUFFIXES: [(&str, &str); 8] = [
    ("extra-high", "xhigh"),
    ("minimal", "minimal"),
    ("medium", "medium"),
    ("xhigh", "xhigh"),
    ("none", "none"),
    ("high", "high"),
    ("low", "low"),
    ("max", "max"),
];

const CLAUDE_TIERS: [&str; 4] = ["fable", "opus", "sonnet", "haiku"];

/// Parsed components of a listed Cursor CLI model ID (`parseCursorModelId`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedId {
    pub base: String,
    pub effort: &'static str,
    pub fast: bool,
    pub thinking: bool,
}

/// Parses a listed Cursor model ID from its spelling alone (`cursorModels.js:parseCursorModelId`).
#[must_use]
pub fn parse_id(id: &str) -> ParsedId {
    let mut rest = id;
    let mut fast = false;
    let mut thinking = false;
    let mut effort = "";

    if let Some(stripped) = rest.strip_suffix("-fast") {
        fast = true;
        rest = stripped;
    }
    if let Some(stripped) = rest.strip_suffix("-thinking") {
        thinking = true;
        rest = stripped;
    }
    for (suffix, value) in EFFORT_SUFFIXES {
        if let Some(stripped) = rest.strip_suffix(suffix)
            && let Some(before_dash) = stripped.strip_suffix('-')
            && !before_dash.is_empty()
        {
            effort = value;
            rest = before_dash;
            break;
        }
    }
    if !thinking && let Some(stripped) = rest.strip_suffix("-thinking") {
        thinking = true;
        rest = stripped;
    }
    let base = if rest.is_empty() { id } else { rest };
    ParsedId {
        base: base.to_owned(),
        effort,
        fast,
        thinking,
    }
}

/// Groups sanitized `cursor-agent --list-models` rows into one picker row per base model
/// (`cursorModels.js:groupCursorModels`).
#[must_use]
pub fn group(rows: &[Value]) -> Vec<Value> {
    struct Entry<'a> {
        row: &'a Value,
        id: &'a str,
        parsed: ParsedId,
    }
    struct Group<'a> {
        base: String,
        rows: Vec<Entry<'a>>,
    }

    let mut groups: Vec<Group<'_>> = Vec::new();
    for row in rows {
        let Some(id) = row
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let parsed = parse_id(id);
        if let Some(existing) = groups.iter_mut().find(|g| g.base == parsed.base) {
            existing.rows.push(Entry { row, id, parsed });
        } else {
            groups.push(Group {
                base: parsed.base.clone(),
                rows: vec![Entry { row, id, parsed }],
            });
        }
    }

    let mut out = Vec::new();
    for mut g in groups {
        // An ID without an effort suffix represents Medium unless an explicit `-medium`
        // variant is also listed, in which case unsuffixed rows stay standalone rows.
        let explicit_medium = g.rows.iter().any(|r| r.parsed.effort == "medium");
        let has_plain = g.rows.iter().any(|r| r.parsed.effort.is_empty());
        if has_plain && explicit_medium {
            for r in g.rows.iter().filter(|r| r.parsed.effort.is_empty()) {
                out.push(plain_row(r.row, r.id, &r.parsed));
            }
            g.rows.retain(|r| !r.parsed.effort.is_empty());
        }
        if g.rows.is_empty() {
            continue;
        }
        if g.rows.len() == 1 {
            let only = &g.rows[0];
            out.push(plain_row(only.row, only.id, &only.parsed));
            continue;
        }

        let has_efforts = g.rows.iter().any(|r| !r.parsed.effort.is_empty());
        let mut variants = Vec::with_capacity(g.rows.len());
        for r in &g.rows {
            let effort = if !r.parsed.effort.is_empty() {
                r.parsed.effort
            } else if has_efforts {
                "medium"
            } else {
                ""
            };
            variants.push((r.id, effort, r.parsed.fast, r.parsed.thinking));
        }

        let efforts: Vec<&'static str> = EFFORT_ORDER
            .iter()
            .copied()
            .filter(|e| variants.iter().any(|v| v.1 == *e))
            .collect();
        let has_fast = variants.iter().any(|v| v.2) && variants.iter().any(|v| !v.2);
        let has_thinking = variants.iter().any(|v| v.3) && variants.iter().any(|v| !v.3);

        let plain_variants: Vec<&Entry<'_>> = g
            .rows
            .iter()
            .filter(|r| !r.parsed.fast && (!has_thinking || !r.parsed.thinking))
            .collect();
        let own = plain_variants
            .iter()
            .copied()
            .find(|r| r.parsed.effort.is_empty())
            .or_else(|| {
                plain_variants.iter().copied().find(|r| {
                    let raw_label = r.row["label"].as_str().unwrap_or("");
                    !label_has_effort(raw_label, r.parsed.effort)
                })
            });
        let own_effort = match own {
            Some(o) if !o.parsed.effort.is_empty() => o.parsed.effort,
            Some(_) if has_efforts => "medium",
            _ => "",
        };
        let default_effort = if efforts.contains(&own_effort) {
            own_effort
        } else if !efforts.is_empty() {
            efforts[(efforts.len() - 1) / 2]
        } else {
            ""
        };
        let default_thinking = if has_thinking {
            false
        } else {
            variants.iter().all(|v| v.3)
        };

        // Longest cleaned label in UTF-16 code units (preserving `"1M"` dropped on some Fast rows).
        let mut best_label = String::new();
        let mut best_utf16_len = 0usize;
        for r in &g.rows {
            let cleaned = base_label(r.row["label"].as_str().unwrap_or(""), &r.parsed);
            if !cleaned.is_empty() {
                let u16_len = cleaned.encode_utf16().count();
                if u16_len > best_utf16_len {
                    best_utf16_len = u16_len;
                    best_label = cleaned;
                }
            }
        }
        if best_label.is_empty() {
            best_label.clone_from(&g.base);
        }

        let mut options = Vec::new();
        if efforts.len() > 1 {
            options.push(effort_option(&efforts, default_effort));
        }
        if has_fast {
            options.push(listed_fast_option());
        }
        if has_thinking {
            options.push(listed_thinking_option());
        }

        let variants_json: Vec<Value> = variants
            .iter()
            .map(|(id, effort, fast, thinking)| {
                json!({
                    "id": id,
                    "effort": effort,
                    "fast": fast,
                    "thinking": thinking,
                })
            })
            .collect();

        let mut obj = Map::new();
        obj.insert("id".into(), json!(g.base));
        obj.insert("label".into(), json!(best_label));
        if g.rows.iter().any(|r| is_truthy(&r.row["isDefault"])) {
            obj.insert("isDefault".into(), json!(true));
        }
        obj.insert("variants".into(), Value::Array(variants_json));
        obj.insert("efforts".into(), json!(efforts));
        obj.insert("defaultEffort".into(), json!(default_effort));
        obj.insert("hasFast".into(), json!(has_fast));
        obj.insert("hasThinking".into(), json!(has_thinking));
        obj.insert("defaultThinking".into(), json!(default_thinking));
        obj.insert("options".into(), Value::Array(options));
        out.push(Value::Object(obj));
    }

    sort(&out)
}

/// Sorts grouped Cursor rows (`Auto`, then Claude by tier newest first, then GPT/Codex, Gemini,
/// Grok, Composer, Kimi, and others newest first; `cursorModels.js:sortCursorModels`).
#[must_use]
pub fn sort(models: &[Value]) -> Vec<Value> {
    let mut sorted = models.to_vec();
    sorted.sort_by(compare_models);
    sorted
}

fn compare_models(a: &Value, b: &Value) -> Ordering {
    let ia = a["id"].as_str().unwrap_or("").to_lowercase();
    let ib = b["id"].as_str().unwrap_or("").to_lowercase();
    if ia == "auto" || ib == "auto" {
        return (ib == "auto").cmp(&(ia == "auto"));
    }

    let fa = family_rank(&ia);
    let fb = family_rank(&ib);
    if fa != fb {
        return fa.cmp(&fb);
    }
    if fa == 0 {
        let ta = tier_rank(&ia);
        let tb = tier_rank(&ib);
        if ta != tb {
            return ta.cmp(&tb);
        }
    }

    let va = version_of(a);
    let vb = version_of(b);
    let version_cmp = vb.total_cmp(&va);
    if version_cmp != Ordering::Equal {
        return version_cmp;
    }

    let la = a["label"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(ia.as_str());
    let lb = b["label"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(ib.as_str());
    locale_like_cmp(la, lb)
}

fn family_rank(id_lower: &str) -> usize {
    if ["claude", "fable", "opus", "sonnet", "haiku"]
        .iter()
        .any(|w| id_lower.contains(w))
    {
        0
    } else if id_lower.contains("gpt") || id_lower.contains("codex") {
        1
    } else if id_lower.contains("gemini") {
        2
    } else if id_lower.contains("grok") {
        3
    } else if id_lower.contains("composer") {
        4
    } else if id_lower.contains("kimi") {
        5
    } else {
        6
    }
}

fn tier_rank(id_lower: &str) -> usize {
    CLAUDE_TIERS
        .iter()
        .position(|tier| id_lower.contains(tier))
        .unwrap_or(CLAUDE_TIERS.len())
}

// Matches `/(?:^|[\s-]K?)(\d+(?:\.\d+)?)(?![\dM])/i` with exact JS backtracking semantics.
fn version_of(model: &Value) -> f64 {
    let text = model["label"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| model["id"].as_str())
        .unwrap_or("");
    version_of_str(text)
}

fn version_of_str(s: &str) -> f64 {
    let bytes = s.as_bytes();
    for (idx, ch) in s.char_indices() {
        if idx == 0
            && let Some(v) = try_parse_version_at(s, bytes, 0)
        {
            return v;
        }
        if js_space(ch) || ch == '-' {
            let after_sep = idx + ch.len_utf8();
            let digit_start = if after_sep < bytes.len() && matches!(bytes[after_sep], b'k' | b'K')
            {
                after_sep + 1
            } else {
                after_sep
            };
            if let Some(v) = try_parse_version_at(s, bytes, digit_start) {
                return v;
            }
        }
    }
    0.0
}

fn try_parse_version_at(s: &str, bytes: &[u8], start: usize) -> Option<f64> {
    if start >= bytes.len() || !bytes[start].is_ascii_digit() {
        return None;
    }
    let mut int_end = start + 1;
    while int_end < bytes.len() && bytes[int_end].is_ascii_digit() {
        int_end += 1;
    }
    if int_end + 1 < bytes.len() && bytes[int_end] == b'.' && bytes[int_end + 1].is_ascii_digit() {
        let mut frac_end = int_end + 2;
        while frac_end < bytes.len() && bytes[frac_end].is_ascii_digit() {
            frac_end += 1;
        }
        if frac_end >= bytes.len() || !matches!(bytes[frac_end], b'm' | b'M') {
            return s[start..frac_end].parse::<f64>().ok();
        }
        // Backtracks `(?:\.\d+)?` to `int_end`, where the next byte is `'.'` (not `[\dM]`).
        return s[start..int_end].parse::<f64>().ok();
    }
    if int_end < bytes.len() && matches!(bytes[int_end], b'm' | b'M') {
        return None;
    }
    s[start..int_end].parse::<f64>().ok()
}

// ICU data preserves Unicode normalization, accent and punctuation ordering.
// The reference fixture records the source runtime's default collation.
fn locale_like_cmp(a: &str, b: &str) -> Ordering {
    static COLLATOR: std::sync::OnceLock<icu_collator::CollatorBorrowed<'static>> =
        std::sync::OnceLock::new();
    COLLATOR
        .get_or_init(|| {
            icu_collator::Collator::try_new(Default::default(), Default::default())
                .expect("compiled root collation data")
        })
        .compare(a, b)
}
fn effort_word(effort: &str) -> &'static str {
    match effort {
        "none" => "None",
        "minimal" => "Minimal",
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra High",
        "max" => "Max",
        _ => "",
    }
}

fn effort_label(effort: &str) -> &'static str {
    match effort {
        "none" => "None",
        "minimal" => "Minimal",
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra high",
        "max" => "Max",
        _ => "",
    }
}

fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}')
}

fn tidy(label: &str) -> String {
    let stripped: String = label.chars().filter(|c| !is_zero_width(*c)).collect();
    stripped
        .split(js_space)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn without_word(label: &str, word: &str) -> String {
    if word.is_empty() {
        return label.to_owned();
    }
    let words: Vec<&str> = label.split(' ').collect();
    let drop: Vec<&str> = word.split(' ').collect();
    if words.len() < drop.len() {
        return label.to_owned();
    }
    for idx in (0..=words.len() - drop.len()).rev() {
        if words[idx..idx + drop.len()] == drop[..] {
            let mut kept = Vec::with_capacity(words.len() - drop.len());
            kept.extend_from_slice(&words[..idx]);
            kept.extend_from_slice(&words[idx + drop.len()..]);
            return kept.join(" ");
        }
    }
    label.to_owned()
}

fn base_label(label: &str, parsed: &ParsedId) -> String {
    let mut out = tidy(label);
    if parsed.fast {
        out = without_word(&out, "Fast");
    }
    if parsed.thinking {
        out = without_word(&out, "Thinking");
    }
    if !parsed.effort.is_empty() {
        out = without_word(&out, effort_word(parsed.effort));
    }
    tidy(&out)
}

fn label_has_effort(label: &str, effort: &str) -> bool {
    if effort.is_empty() {
        return false;
    }
    let word = effort_word(effort);
    if word.is_empty() {
        return false;
    }
    let padded = format!(" {} ", tidy(label));
    let needle = format!(" {word} ");
    padded.contains(&needle)
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0 && !x.is_nan()),
        _ => true,
    }
}

fn plain_row(row: &Value, id: &str, parsed: &ParsedId) -> Value {
    let tidied = tidy(row["label"].as_str().unwrap_or(""));
    let label = if tidied.is_empty() {
        id
    } else {
        tidied.as_str()
    };
    let mut obj = Map::new();
    obj.insert("id".into(), json!(id));
    obj.insert("label".into(), json!(label));
    if is_truthy(&row["isDefault"]) {
        obj.insert("isDefault".into(), json!(true));
    }
    obj.insert(
        "variants".into(),
        json!([{
            "id": id,
            "effort": "",
            "fast": parsed.fast,
            "thinking": parsed.thinking,
        }]),
    );
    obj.insert("efforts".into(), json!([]));
    obj.insert("defaultEffort".into(), json!(""));
    obj.insert("hasFast".into(), json!(false));
    obj.insert("hasThinking".into(), json!(false));
    obj.insert("defaultThinking".into(), json!(parsed.thinking));
    obj.insert("options".into(), json!([]));
    Value::Object(obj)
}

fn effort_option(efforts: &[&str], default_value: &str) -> Value {
    let choices: Vec<Value> = efforts
        .iter()
        .map(|e| json!({ "value": e, "label": effort_label(e) }))
        .collect();
    json!({
        "id": "effort",
        "label": "Effort",
        "category": "thought_level",
        "kind": {
            "type": "select",
            "choices": choices,
            "defaultValue": default_value,
        },
        "apply": { "composedIntoModel": true },
    })
}

fn listed_fast_option() -> Value {
    json!({
        "id": "fastMode",
        "label": "Fast mode",
        "category": "mode",
        "kind": { "type": "boolean", "defaultValue": false },
        "apply": { "composedIntoModel": true },
    })
}

fn listed_thinking_option() -> Value {
    json!({
        "id": "thinking",
        "label": "Thinking",
        "category": "model_config",
        "kind": { "type": "boolean", "defaultValue": false },
        "apply": { "composedIntoModel": true },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_id_parsing_grouping_and_sorting_match_source_fixtures() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/agent_model_cursor.json"))
                .unwrap();

        for case in fixture["parsedIds"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            let parsed = parse_id(input);
            assert_eq!(
                json!({
                    "base": parsed.base,
                    "effort": parsed.effort,
                    "fast": parsed.fast,
                    "thinking": parsed.thinking,
                }),
                case["expected"],
                "parse_id({input:?})"
            );
        }

        for suite in fixture["groups"].as_array().unwrap() {
            let name = suite["name"].as_str().unwrap();
            let rows = suite["rows"].as_array().unwrap();
            assert_eq!(
                group(rows),
                *suite["expected"].as_array().unwrap(),
                "group suite {name}"
            );
        }

        let real_rows = fixture["realList"]["parsedRows"].as_array().unwrap();
        let real_expected = fixture["realList"]["grouped"].as_array().unwrap();
        assert_eq!(group(real_rows), *real_expected, "real 246-row Cursor list");
    }
}
