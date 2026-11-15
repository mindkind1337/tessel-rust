//! Resolve the real configured hook folder without the execution-time override.
use super::guest_git_host::{GitRun, HostGit};
use super::*;
pub(super) fn discover(root: &Path, roots: &[PathBuf]) -> Result<Vec<String>> {
    let mut repository = false;
    for parent in root.ancestors() {
        match std::fs::symlink_metadata(parent.join(".git")) {
            Ok(_) => {
                repository = true;
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot verify Git hooks scope.".into()),
        }
    }
    if !repository {
        return Ok(Vec::new());
    }
    discover_with(&mut HostGit::new(root), root, roots)
}
fn discover_with(git: &mut dyn GitRun, root: &Path, roots: &[PathBuf]) -> Result<Vec<String>> {
    // `config --list` does not execute hooks. Do not replace core.hooksPath here:
    // that would conceal the real value which determines guest write permissions.
    let output = git.run(
        &[
            "-c",
            "core.fsmonitor=false",
            "config",
            "--includes",
            "-z",
            "--show-origin",
            "--list",
        ],
        None,
    )?;
    if output.truncated {
        return Err("Incomplete Git hooks configuration.".into());
    }
    super::guest_git_config::validate_config_origins(&output.bytes, root, roots)?;
    let text = std::str::from_utf8(&output.bytes).map_err(|_| "Invalid Git hooks configuration")?;
    let fields: Vec<&str> = text
        .strip_suffix('\0')
        .unwrap_or(text)
        .split('\0')
        .collect();
    let mut configured = None;
    for pair in fields.as_chunks::<2>().0 {
        let (key, value) = pair[1].split_once('\n').unwrap_or((pair[1], ""));
        if key.eq_ignore_ascii_case("core.hookspath") {
            configured = Some(value.trim());
        }
    }
    let Some(value) = configured.filter(|v| !v.is_empty()) else {
        return Ok(Vec::new());
    };
    if value.len() > 4096 {
        return Err("Git hooks path is too long.".into());
    }
    let top = git.run(
        &["-c", "core.fsmonitor=false", "rev-parse", "--show-toplevel"],
        None,
    )?;
    if top.truncated {
        return Err("Incomplete Git repository scope.".into());
    }
    let top = std::str::from_utf8(&top.bytes)
        .map_err(|_| "Invalid Git repository scope")?
        .trim();
    if !Path::new(top).is_absolute() {
        return Err("Invalid Git repository scope.".into());
    }
    resolve_scope(root, Path::new(top), value)
}
fn resolve_scope(root: &Path, top: &Path, value: &str) -> Result<Vec<String>> {
    use super::guest_git_config::{canonical_existing_prefix, comparison_path, lexical_normalize};
    let path = Path::new(value);
    if !path.is_absolute()
        && path.components().any(|c| {
            matches!(
                c,
                std::path::Component::Prefix(_) | std::path::Component::RootDir
            )
        })
    {
        return Err("Ambiguous Git hooks path.".into());
    }
    let full = lexical_normalize(&if path.is_absolute() {
        path.to_path_buf()
    } else {
        top.join(path)
    });
    let real = canonical_existing_prefix(&full).ok_or("Cannot resolve Git hooks folder")?;
    let root = comparison_path(&dunce::canonicalize(root).map_err(|_| "Shared root unavailable")?);
    let real = comparison_path(&real);
    Ok(match real.strip_prefix(&root) {
        Ok(relative) => vec![relative.to_string_lossy().replace('\\', "/").to_lowercase()],
        Err(_) => Vec::new(),
    })
}
pub(super) fn protects(path: &Path, folders: &[String]) -> bool {
    super::protected_paths::is_protected(&path.to_string_lossy().replace('\\', "/"), folders)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::Capture;
    struct FixtureGit {
        config: Vec<u8>,
        top: Vec<u8>,
        truncated: bool,
        calls: usize,
    }
    impl GitRun for FixtureGit {
        fn run(&mut self, args: &[&str], _: Option<&[u8]>) -> Result<Capture> {
            self.calls += 1;
            assert!(!args.iter().any(|s| s.starts_with("core.hooksPath=")));
            Ok(Capture {
                bytes: if args.contains(&"config") {
                    self.config.clone()
                } else {
                    self.top.clone()
                },
                truncated: self.truncated,
            })
        }
    }
    #[test]
    fn configured_scope_uses_repo_top_and_last_value_not_project_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path();
        let root = top.join("sub");
        std::fs::create_dir(&root).unwrap();
        let origin = format!("file:{}", top.join(".git/config").display());
        let mut git = FixtureGit {
            config: format!(
                "{origin}\0core.hookspath\nold\0{origin}\0core.hookspath\nsub/missing/hooks\0"
            )
            .into_bytes(),
            top: top.to_string_lossy().as_bytes().to_vec(),
            truncated: false,
            calls: 0,
        };
        let scope = discover_with(&mut git, &root, std::slice::from_ref(&root)).unwrap();
        assert_eq!(scope, vec!["missing/hooks"]);
        assert!(protects(Path::new("missing/hooks/run"), &scope));
        assert!(!protects(Path::new("missing/hooks-other/run"), &scope));
        assert_eq!(resolve_scope(&root, top, "sub/.").unwrap(), vec![""]);
        assert!(protects(Path::new("post-commit"), &[String::new()]));
        assert!(!protects(Path::new("nested/post-commit"), &[String::new()]));
        assert!(!protects(Path::new("post-commit"), &[]));
        assert!(resolve_scope(&root, top, "outside").unwrap().is_empty());
        assert_eq!(
            resolve_scope(&root, top, root.join("absolute").to_str().unwrap()).unwrap(),
            vec!["absolute"]
        );
    }
    #[test]
    fn missing_hook_value_is_distinct_from_an_unreadable_configuration() {
        let root = tempfile::tempdir().unwrap();
        let mut git = FixtureGit {
            config: vec![],
            top: vec![],
            truncated: false,
            calls: 0,
        };
        assert!(
            discover_with(&mut git, root.path(), &[])
                .unwrap()
                .is_empty()
        );
        assert_eq!(git.calls, 1);
        git.truncated = true;
        assert!(discover_with(&mut git, root.path(), &[]).is_err());
        git.truncated = false;
        git.config = b"file:broken\0core.hookspath\nfolder".to_vec();
        assert!(discover_with(&mut git, root.path(), &[]).is_err());
    }
}
