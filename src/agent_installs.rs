//! Immutable installation metadata produced by the source checkInstall rules.
//! Reading this catalogue never downloads or executes anything.
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Install {
    pub id: String,
    pub install: Option<Vec<String>>,
    pub install_shell: Option<String>,
    pub install_source: Option<String>,
    pub install_confirm: bool,
    pub docs_url: Option<String>,
}

#[derive(Deserialize)]
struct Catalogue {
    agents: Vec<Install>,
}

pub fn all() -> &'static [Install] {
    static DATA: OnceLock<Catalogue> = OnceLock::new();
    &DATA
        .get_or_init(|| {
            serde_json::from_str(include_str!("agent_installs.json"))
                .expect("embedded checked installation catalogue")
        })
        .agents
}

pub fn find(id: &str) -> Option<&'static Install> {
    let canonical = crate::agent_catalog::find(id)?.id;
    all().iter().find(|entry| entry.id == canonical)
}

impl Install {
    /// Copy instructions verbatim. Multi-step installers retain their line boundaries.
    pub fn copy_text(&self) -> Option<String> {
        self.install.as_ref().map(|steps| steps.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_source_preset_has_one_install_record_and_aliases_share_it() {
        assert_eq!(all().len(), crate::agent_catalog::ALL.len());
        let mut ids = std::collections::HashSet::new();
        for entry in all() {
            assert!(ids.insert(&entry.id));
            assert!(crate::agent_catalog::find(&entry.id).is_some());
            for url in [&entry.install_source, &entry.docs_url]
                .into_iter()
                .flatten()
            {
                assert_eq!(url::Url::parse(url).unwrap().scheme(), "https");
            }
            if entry.install_confirm {
                assert!(entry.install.is_some());
                assert!(entry.install_source.is_some());
            }
        }
        assert!(std::ptr::eq(
            find("cursor").unwrap(),
            find("cursor-agent").unwrap()
        ));
        assert!(find("unknown").is_none());
    }

    #[test]
    fn source_confirmation_and_docs_only_choices_are_preserved() {
        let codex = find("codex").unwrap();
        assert_eq!(
            codex.copy_text().as_deref(),
            Some("npm install -g @openai/codex")
        );
        assert!(!codex.install_confirm);
        let kimi = find("kimi").unwrap();
        assert!(kimi.install_confirm);
        assert_eq!(kimi.install_shell.as_deref(), Some("powershell"));
        let aider = find("aider").unwrap();
        assert!(aider.install_confirm);
        assert_eq!(
            aider.copy_text().as_deref(),
            Some("python -m pip install aider-install\naider-install")
        );
        for id in ["goose", "auggie", "kiro", "rovo", "opencode2", "dsh"] {
            let install = find(id).unwrap();
            assert!(install.install.is_none());
            assert!(install.copy_text().is_none());
            assert!(install.docs_url.is_some());
        }
    }
}
