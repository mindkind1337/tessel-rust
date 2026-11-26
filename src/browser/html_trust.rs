use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Folder {
    Local(PathBuf),
    Ssh { host: String, root: String },
}

impl Folder {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Local(path) => path.display().to_string(),
            Self::Ssh { host, root } => format!("{host}:{root}"),
        }
    }
}

#[derive(Default)]
pub(crate) struct Trust {
    decisions: Vec<(Folder, bool)>,
    file: Option<PathBuf>,
}

impl Trust {
    pub(crate) fn load(directory: Option<&Path>) -> Self {
        let file = directory.map(|p| p.join("html-preview-trust.json"));
        let decisions = file
            .as_ref()
            .and_then(|p| {
                let mut reader = std::fs::File::open(p).ok()?;
                if reader.metadata().ok()?.len() > 4 * 1024 * 1024 {
                    return None;
                }
                serde_json::from_reader::<_, Vec<(Folder, bool)>>(&mut reader).ok()
            })
            .filter(|v| v.len() <= 300)
            .unwrap_or_default();
        Self { decisions, file }
    }
    pub(crate) fn decision(&self, folder: &Folder) -> Option<bool> {
        self.decisions
            .iter()
            .rev()
            .find(|(key, _)| key == folder)
            .map(|(_, value)| *value)
    }
    pub(crate) fn decide(&mut self, folder: Folder, trusted: bool) -> Result<(), String> {
        self.decisions.retain(|(key, _)| key != &folder);
        if self.decisions.len() >= 300 {
            self.decisions.remove(0);
        }
        self.decisions.push((folder, trusted));
        if let Some(path) = &self.file {
            crate::workspace::atomic_write(
                path,
                &serde_json::to_vec(&self.decisions).map_err(|e| e.to_string())?,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decisions_remain_local_to_the_folder_and_ssh_host_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let open = || Trust::load(Some(dir.path()));
        let folder = |host: &str, root: &str| Folder::Ssh {
            host: host.into(),
            root: root.into(),
        };
        let mut trust = open();
        trust.decide(folder("dev", "/Site"), true).unwrap();
        trust.decide(folder("dev", "/site"), false).unwrap();
        let mut trust = open();
        assert_eq!(trust.decision(&folder("dev", "/Site")), Some(true));
        assert_eq!(trust.decision(&folder("dev", "/site")), Some(false));
        assert_eq!(trust.decision(&folder("live", "/Site")), None);
        assert_eq!(trust.decision(&Folder::Local("/Site".into())), None);
        trust.decide(folder("dev", "/Site"), false).unwrap();
        assert_eq!(open().decision(&folder("dev", "/Site")), Some(false));
    }
}
