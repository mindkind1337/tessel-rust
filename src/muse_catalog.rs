// Adapted from Tessel main/museCatalog.js and main/museSessions.js / Orca.
// Copyright (c) 2026 Lovecast Inc. MIT; see THIRD_PARTY_NOTICES.md.
//! Read-only Muse model metadata. Never reads sign-in files or session logs.
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_FILES: usize = 20;
const MAX_DIRECTORY_ENTRIES: usize = 100_000;

pub fn model_id(value: &Value) -> Option<&str> {
    let id = crate::agent_env::trim(value.as_str()?);
    (!id.is_empty()
        && id.len() <= 120
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:/@[]+-".contains(&b)))
    .then_some(id)
}

/// Source model/list response shape. Inputs are in filename order; first visible
/// occurrence wins across files, while a hidden row does not reserve its id.
pub fn answer(texts: &[String]) -> Option<Value> {
    let mut models = Vec::new();
    let mut seen = HashSet::new();
    for text in texts {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        let Some(rows) = value["rows"].as_array() else {
            continue;
        };
        for row in rows.iter().take(300) {
            let Some(id) = model_id(&row["model_id"]) else {
                continue;
            };
            if row["visibility"] == "hidden" || !seen.insert(id.to_owned()) {
                continue;
            }
            let tiers: Vec<_> = row["reasoning_effort_variants"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v["tier"].as_str().filter(|s| !s.is_empty()))
                .collect();
            let mut model = json!({"modelId":id,
                "displayLabel":row["display_label"].as_str().unwrap_or(id),
                "description":row["description"].as_str(),
                "isDefault":row["is_default"] == true,
                "variants":tiers});
            if let Some(effort) = row["default_reasoning_effort"].as_str() {
                model["defaultReasoningEffort"] = effort.into();
            }
            models.push(model);
        }
    }
    (!models.is_empty()).then(|| json!({"models":models}))
}

pub fn data_dir(home: &Path, real_home: &Path, xdg: Option<&str>) -> PathBuf {
    let xdg = xdg
        .map(crate::agent_env::trim)
        .filter(|s| !s.is_empty())
        .map(Path::new);
    xdg.filter(|p| home == real_home && p.is_absolute())
        .map(Path::to_owned)
        .unwrap_or_else(|| home.join(".local").join("share"))
        .join("muse")
}

/// Worker-only bounded read. Keep the lexically first twenty JSON regular files,
/// without collecting an unbounded directory. A directory over budget yields no
/// answer so the protocol fallback can run instead of claiming a partial list.
pub fn read_catalog(dir: &Path) -> Option<Value> {
    let mut names: Vec<PathBuf> = Vec::new();
    for (count, entry) in fs::read_dir(dir).ok()?.enumerate() {
        if count >= MAX_DIRECTORY_ENTRIES {
            return None;
        }
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let file = entry.path();
        if !file
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            continue;
        }
        names.push(file);
        names.sort_by_cached_key(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .encode_utf16()
                .collect::<Vec<_>>()
        });
        names.truncate(MAX_FILES);
    }
    let mut texts = Vec::new();
    for path in names {
        let Ok(file) = fs::File::open(path) else {
            continue;
        };
        if !file
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() <= MAX_BYTES)
        {
            continue;
        }
        let mut bytes = Vec::new();
        if file.take(MAX_BYTES + 1).read_to_end(&mut bytes).is_err()
            || bytes.len() as u64 > MAX_BYTES
        {
            continue;
        }
        texts.push(String::from_utf8_lossy(&bytes).into_owned());
    }
    answer(&texts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visible_rows_and_field_shapes_match_source() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/muse_catalog.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let texts: Vec<_> = case["texts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_owned())
                .collect();
            assert_eq!(
                answer(&texts).unwrap_or(Value::Null),
                case["expected"],
                "{}",
                case["name"]
            );
        }
    }
    #[test]
    fn bounded_files_are_sorted_hidden_duplicates_do_not_mask_visible_models() {
        let dir = tempfile::tempdir().unwrap();
        for i in (0..25).rev() {
            fs::write(
                dir.path().join(format!("{i:02}.json")),
                json!({"rows":[{"model_id":format!("m{i}"),"display_label":format!("Model {i}")}]})
                    .to_string(),
            )
            .unwrap();
        }
        let list = read_catalog(dir.path()).unwrap();
        assert_eq!(list["models"].as_array().unwrap().len(), 20);
        assert_eq!(list["models"][0]["modelId"], "m0");
        assert_eq!(list["models"][19]["modelId"], "m19");
        fs::write(
            dir.path().join("00.json"),
            vec![b' '; MAX_BYTES as usize + 1],
        )
        .unwrap();
        assert_eq!(
            read_catalog(dir.path()).unwrap()["models"]
                .as_array()
                .unwrap()
                .len(),
            19
        );
        assert!(read_catalog(&dir.path().join("missing")).is_none());
    }
    #[test]
    fn xdg_override_belongs_only_to_real_home_and_must_be_absolute() {
        let home = std::env::current_dir().unwrap();
        let xdg = home.join("fixture-data");
        assert_eq!(data_dir(&home, &home, xdg.to_str()), xdg.join("muse"));
        assert_eq!(
            data_dir(&home, &home, Some("relative")),
            home.join(".local/share/muse")
        );
        assert_eq!(
            data_dir(&home, &home.join("other"), xdg.to_str()),
            home.join(".local/share/muse")
        );
    }
}
