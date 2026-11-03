//! Installer locations from Tessel's toolDirs.js (see Orca attribution in README).
use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

pub(crate) struct SearchPath {
    pub directories: Vec<PathBuf>,
    pub value: OsString,
}

impl SearchPath {
    pub fn current() -> Result<Self, String> {
        // Reuse the terminal backend's environment snapshot. On Windows it merges
        // current machine/user registry PATH values and expands REG_EXPAND_SZ.
        // Constructing a builder does not start a process.
        let environment = portable_pty::CommandBuilder::new("tessel-path-snapshot");
        let base = environment
            .get_env("PATH")
            .unwrap_or_else(|| OsStr::new(""))
            .to_owned();
        let extras = directories::BaseDirs::new()
            .map(|dirs| {
                extra_dirs(
                    dirs.home_dir(),
                    std::env::var_os("LOCALAPPDATA").as_deref(),
                    std::env::var_os("APPDATA").as_deref(),
                    cfg!(windows),
                    &[],
                )
            })
            .unwrap_or_default();
        Self::with_extras(&base, &extras)
    }

    pub fn with_extras(base: &OsStr, extra: &[PathBuf]) -> Result<Self, String> {
        let mut directories: Vec<_> = std::env::split_paths(base)
            .filter(|p| !p.as_os_str().is_empty())
            .collect();
        let key = |path: &Path| {
            path.to_string_lossy()
                .trim_end_matches(['/', '\\'])
                .to_lowercase()
        };
        let mut seen: HashSet<_> = directories.iter().map(|p| key(p)).collect();
        for path in extra {
            if seen.insert(key(path)) {
                directories.push(path.clone());
            }
        }
        let value = std::env::join_paths(&directories)
            .map_err(|_| "Cannot compose the agent search path.".to_owned())?;
        Ok(Self { directories, value })
    }
}

fn subdirectories(dir: &Path, matches: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut found: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && matches(&entry.file_name().to_string_lossy())
        })
        .map(|entry| entry.path())
        .collect();
    found.sort();
    found
}

fn python_version(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower
        .strip_prefix("python")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()))
}

fn extra_dirs(
    home: &Path,
    local: Option<&OsStr>,
    roaming: Option<&OsStr>,
    windows: bool,
    python: &[PathBuf],
) -> Vec<PathBuf> {
    if !windows {
        return vec![home.join(".local/bin")];
    }
    let local = local
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData/Local"));
    let roaming = roaming
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData/Roaming"));
    let mut dirs = vec![
        home.join(".kimi-code/bin"),
        home.join(".opencode/bin"),
        local.join("cursor-agent"),
        local.join("Programs/Ollama"),
        home.join(".local/bin"),
        roaming.join("npm"),
        home.join(".bun/bin"),
    ];
    for package in subdirectories(&local.join("Packages"), |n| {
        n.to_ascii_lowercase()
            .starts_with("pythonsoftwarefoundation.python.")
    }) {
        for version in subdirectories(&package.join("LocalCache/local-packages"), python_version) {
            dirs.push(version.join("Scripts"));
        }
    }
    for version in subdirectories(&roaming.join("Python"), python_version) {
        dirs.push(version.join("Scripts"));
    }
    dirs.retain(|path| path.is_dir());
    // Source retains Python-reported drive-absolute directories before pip creates them.
    dirs.extend(
        python
            .iter()
            .filter(|path| {
                let text = path.to_string_lossy();
                let bytes = text.as_bytes();
                bytes.len() >= 3
                    && bytes[0].is_ascii_alphabetic()
                    && bytes[1] == b':'
                    && bytes[2] == b'\\'
            })
            .cloned(),
    );
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_installers_and_python_directories_keep_source_priority() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let local = home.join("Local");
        let roaming = home.join("Roaming");
        let expected = vec![home.join(".kimi-code/bin"), home.join(".opencode/bin"), local.join("cursor-agent"), local.join("Programs/Ollama"), home.join(".local/bin"), roaming.join("npm"), home.join(".bun/bin"),
            local.join("Packages/PythonSoftwareFoundation.Python.3.13_fixture/LocalCache/local-packages/Python313/Scripts"), roaming.join("Python/Python312/Scripts")];
        for dir in &expected {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::create_dir_all(roaming.join("Python/PythonNotAVersion/Scripts")).unwrap();
        assert_eq!(
            extra_dirs(
                home,
                Some(local.as_os_str()),
                Some(roaming.as_os_str()),
                true,
                &[]
            ),
            expected
        );
    }
    #[test]
    fn path_precedence_case_deduplication_and_missing_directories_match_source() {
        let base =
            std::env::join_paths([PathBuf::from("Existing"), PathBuf::from("existing")]).unwrap();
        let search = SearchPath::with_extras(
            &base,
            &["EXISTING/".into(), "Added".into(), "added\\".into()],
        )
        .unwrap();
        assert_eq!(
            search.directories,
            vec![PathBuf::from("Existing"), "existing".into(), "Added".into()]
        );
        assert_eq!(
            std::env::split_paths(&search.value).collect::<Vec<_>>(),
            search.directories
        );
        let temp = tempfile::tempdir().unwrap();
        assert!(extra_dirs(temp.path(), None, None, true, &[]).is_empty());
        assert_eq!(
            extra_dirs(temp.path(), None, None, false, &[]),
            vec![temp.path().join(".local/bin")]
        );
        assert_eq!(
            extra_dirs(
                temp.path(),
                None,
                None,
                true,
                &["C:\\future\\Scripts".into(), "relative".into()]
            ),
            vec![PathBuf::from("C:\\future\\Scripts")]
        );
    }
}
