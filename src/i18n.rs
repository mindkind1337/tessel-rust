use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "fr")]
    French,
}
static FRENCH: AtomicBool = AtomicBool::new(false);
static CATALOG: OnceLock<HashMap<String, String>> = OnceLock::new();
fn catalog() -> &'static HashMap<String, String> {
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("i18n/fr.json"))
            .expect("validated embedded French catalogue")
    })
}
pub fn set_language(language: Language) {
    FRENCH.store(language == Language::French, Ordering::Relaxed);
}
pub fn is_french() -> bool {
    FRENCH.load(Ordering::Relaxed)
}
pub fn translate(language: Language, text: &str) -> &str {
    if language == Language::French {
        catalog().get(text).map(String::as_str).unwrap_or(text)
    } else {
        text
    }
}
pub fn tr(text: &str) -> &str {
    translate(
        if FRENCH.load(Ordering::Relaxed) {
            Language::French
        } else {
            Language::English
        },
        text,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translations_preserve_unknown_content_and_language_is_validated() {
        assert_eq!(translate(Language::French, "Settings"), "Paramètres");
        assert_eq!(translate(Language::English, "Settings"), "Settings");
        assert_eq!(
            translate(Language::French, "a user filename.pdf"),
            "a user filename.pdf"
        );
        assert!(serde_json::from_str::<Language>("\"unknown\"").is_err());
        assert!(catalog().len() > 100);
        assert!(catalog().values().all(|v| !v.trim().is_empty()));
    }
}
