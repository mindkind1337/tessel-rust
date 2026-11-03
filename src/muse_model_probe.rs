// Adapted from Tessel main/museServe.js, museCatalog.js and agentModelList.js.
//! Model-only Muse discovery: local metadata first, direct native server second.
use crate::agent_model_probe::Failure;
use serde_json::Value;
use std::{
    ffi::OsString,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

fn valid_version(version: &str) -> bool {
    let Some((base, revision)) = version.split_once("-R") else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    base.split('.').count() == 3
        && base.split('.').all(digits)
        && (1..=2).contains(&revision.split('.').count())
        && revision.split('.').all(digits)
}
fn binary_in(
    dir: &Path,
    read: &impl Fn(&Path) -> String,
    exists: &impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let version = read(&dir.join(".muse-version"));
    let version = crate::agent_env::trim(&version);
    if !valid_version(version) {
        return None;
    }
    let file = dir.join(format!("muse-bin-{version}.exe"));
    exists(&file).then_some(file)
}
fn find_in(
    dirs: &[PathBuf],
    windows: bool,
    read: &impl Fn(&Path) -> String,
    exists: &impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    for dir in dirs.iter().filter(|d| d.is_absolute()) {
        let direct = dir.join(if windows { "muse.exe" } else { "muse" });
        if exists(&direct) {
            return Some(direct);
        }
        if windows
            && exists(&dir.join("muse.cmd"))
            && let Some(file) = binary_in(dir, read, exists)
        {
            return Some(file);
        }
    }
    None
}
fn install_dirs(home: &Path, env: &[(OsString, OsString)]) -> Vec<PathBuf> {
    let value = |name: &str| {
        env.iter()
            .find(|(k, _)| {
                if cfg!(windows) {
                    k.eq_ignore_ascii_case(name)
                } else {
                    k == name
                }
            })
            .map(|(_, v)| v.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let absolute = |name| {
        let v = value(name);
        let p = PathBuf::from(crate::agent_env::trim(&v));
        p.is_absolute().then_some(p)
    };
    let path = env
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
        .map(|(_, v)| v.to_string_lossy())
        .unwrap_or_default();
    let mut dirs: Vec<_> = path
        .split(if cfg!(windows) { ';' } else { ':' })
        .map(|s| {
            let s = crate::agent_env::trim(s);
            PathBuf::from(
                s.strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .unwrap_or(s),
            )
        })
        .collect();
    dirs.push(absolute("MUSE_INSTALL_DIR").unwrap_or_else(|| {
        if cfg!(windows) {
            absolute("LOCALAPPDATA")
                .unwrap_or_else(|| home.join("AppData/Local"))
                .join("Programs/muse")
        } else {
            home.join(".local/bin")
        }
    }));
    dirs
}
fn read_version(path: &Path) -> String {
    let Ok(file) = fs::File::open(path) else {
        return String::new();
    };
    let mut bytes = Vec::new();
    if file.take(4097).read_to_end(&mut bytes).is_err() || bytes.len() > 4096 {
        return String::new();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
fn from_catalog_or_query(
    dir: &Path,
    query: impl FnOnce() -> Result<Value, Failure>,
) -> Result<Vec<Value>, Failure> {
    let answer = match crate::muse_catalog::read_catalog(dir) {
        Some(value) => value,
        None => query()?,
    };
    let models = crate::agent_model_probe_parse::parse("muse", &answer.to_string());
    if models.is_empty() {
        Err(Failure::Empty)
    } else {
        Ok(models)
    }
}
pub fn list() -> Result<Vec<Value>, Failure> {
    let home = directories::BaseDirs::new().ok_or(Failure::NotFound)?;
    let home = home.home_dir();
    let xdg = std::env::var("XDG_DATA_HOME").ok();
    let dir = crate::muse_catalog::data_dir(home, home, xdg.as_deref()).join("model-catalog");
    from_catalog_or_query(&dir, || {
        let dirs = install_dirs(home, &std::env::vars_os().collect::<Vec<_>>());
        let file = find_in(&dirs, cfg!(windows), &read_version, &|p| p.is_file())
            .ok_or(Failure::NotFound)?;
        let mut command = tokio::process::Command::new(file);
        command.arg("serve").current_dir(home);
        crate::muse_protocol::model_list(command, Duration::from_secs(60))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn catalog_prevents_starting_server_and_missing_catalog_uses_protocol() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("meta.json"), json!({"rows":[{"model_id":"local-model", "display_label":"Local", "reasoning_effort_variants":[{"tier":"high"}]}]}).to_string()).unwrap();
        let models = from_catalog_or_query(dir.path(), || {
            panic!("Local catalogue must prevent process start")
        })
        .unwrap();
        assert_eq!(models[0]["id"], "local-model");
        assert_eq!(models[0]["effortLevels"], json!(["high"]));
        let models = from_catalog_or_query(&dir.path().join("missing"), || {
            Ok(json!({"models":[{"modelId":"remote-model"}]}))
        })
        .unwrap();
        assert_eq!(models[0]["id"], "remote-model");
        assert_eq!(
            from_catalog_or_query(&dir.path().join("missing"), || Err(Failure::NotFound)),
            Err(Failure::NotFound)
        );
    }
    #[test]
    fn native_binary_precedence_and_version_validation_never_execute_launcher() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        let dirs = vec![PathBuf::from("relative"), first.clone(), second.clone()];
        let version = |_: &Path| "1.4.10-R2.1\n".to_owned();
        let target = first.join("muse-bin-1.4.10-R2.1.exe");
        assert_eq!(
            find_in(&dirs, true, &version, &|p| p == first.join("muse.cmd")
                || p == target
                || p == second.join("muse.exe")),
            Some(target)
        );
        assert_eq!(
            find_in(&dirs, true, &version, &|p| p == first.join("muse.exe")
                || p == second.join("muse.exe")),
            Some(first.join("muse.exe"))
        );
        for bad in [
            "",
            "1.2-R1",
            "1.2.3",
            "1.2.3-R../x",
            "1.2.3-R1.2.3",
            "1.2.3-R1\nother",
        ] {
            assert!(!valid_version(bad));
        }
        assert_eq!(
            find_in(&dirs, true, &|_| "../escape".into(), &|_| true),
            Some(first.join("muse.exe"))
        );
        assert!(binary_in(&first, &|_| "../escape".into(), &|_| true).is_none());
        assert_eq!(
            find_in(&dirs, false, &version, &|p| p == second.join("muse")),
            Some(second.join("muse"))
        );
    }
    #[test]
    fn install_directories_preserve_path_order_and_require_absolute_override() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("with space");
        let second = dir.path().join("other");
        let install = dir.path().join("custom-install");
        let path = format!(
            "\"{}\"{}{}",
            first.display(),
            if cfg!(windows) { ';' } else { ':' },
            second.display()
        );
        let env = vec![
            ("PATH".into(), path.into()),
            ("MUSE_INSTALL_DIR".into(), install.as_os_str().into()),
        ];
        assert_eq!(install_dirs(dir.path(), &env), vec![first, second, install]);
        let env = vec![("MUSE_INSTALL_DIR".into(), "relative".into())];
        let dirs = install_dirs(dir.path(), &env);
        assert_eq!(
            dirs.last().unwrap(),
            &dir.path().join(if cfg!(windows) {
                "AppData/Local/Programs/muse"
            } else {
                ".local/bin"
            })
        );
    }
}
