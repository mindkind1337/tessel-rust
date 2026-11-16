use super::*;
const PROBE: &str = "TESSEL_SYNTHETIC_GIT_ENV_PROBE";
fn empty_repo(path: &Path) {
    std::fs::create_dir_all(path.join(".git/objects")).unwrap();
    std::fs::create_dir_all(path.join(".git/refs/heads")).unwrap();
    std::fs::write(path.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
    std::fs::write(
        path.join(".git/config"),
        b"[core]\n repositoryformatversion = 0\n bare = false\n",
    )
    .unwrap();
}
#[test]
fn guest_environment_probe() {
    let Some(root) = std::env::var_os(PROBE) else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let output = run_guest_bytes(
        &root,
        &["rev-parse", "--show-toplevel"],
        None,
        Duration::from_secs(3),
    )
    .unwrap();
    let actual = std::path::PathBuf::from(std::str::from_utf8(&output.bytes).unwrap().trim());
    assert_eq!(actual.canonicalize().unwrap(), root.canonicalize().unwrap());
}
#[test]
fn guest_runner_ignores_inherited_repository_and_config_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("expected");
    let other = dir.path().join("wrong");
    empty_repo(&root);
    empty_repo(&other);
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join(".gitconfig"), b"").unwrap();
    let test_name = format!(
        "{}::guest_environment_probe",
        module_path!().split_once("::").unwrap().1
    );
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", &test_name, "--nocapture"])
        .env(PROBE, &root)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("GIT_DIR", other.join(".git"))
        .env("GIT_WORK_TREE", &other)
        .env("GIT_COMMON_DIR", other.join(".git"))
        .env("GIT_INDEX_FILE", dir.path().join("wrong-index"))
        .env("GIT_OBJECT_DIRECTORY", dir.path().join("missing-objects"))
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.bare")
        .env("GIT_CONFIG_VALUE_0", "true");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        child.creation_flags(0x08000000);
    }
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "probe did not run"
    );
}

#[test]
fn parent_session_git_probe() {
    let Some(root) = std::env::var_os("TESSEL_SYNTHETIC_CLEAN_GIT_PROBE") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let check = r#"alias.tessel-env-check=!test -z "${TESSEL_TEAM_MCP_KEY+x}" && test -z "${TESSEL_TEAM_MCP_PANE+x}" && test -z "${TESSEL_TEAM_MCP_GENERATION+x}" && test -z "${CLAUDE_CODE_SESSION_ID+x}" && test "$TESSEL_SYNTHETIC_KEEP" = keep"#;
    run_bytes(&root, &["-c", check, "tessel-env-check"], None).unwrap();
}

#[test]
fn git_children_do_not_inherit_parent_session_capabilities() {
    let dir = tempfile::tempdir().unwrap();
    empty_repo(dir.path());
    let test_name = format!(
        "{}::parent_session_git_probe",
        module_path!().split_once("::").unwrap().1
    );
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", &test_name, "--nocapture"])
        .env("TESSEL_SYNTHETIC_CLEAN_GIT_PROBE", dir.path())
        .env("TESSEL_TEAM_MCP_KEY", "synthetic-secret")
        .env("TESSEL_TEAM_MCP_PANE", "synthetic-pane")
        .env("TESSEL_TEAM_MCP_GENERATION", "synthetic-generation")
        .env("CLAUDE_CODE_SESSION_ID", "synthetic-session")
        .env("TESSEL_SYNTHETIC_KEEP", "keep");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        child.creation_flags(0x08000000);
    }
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "Synthetic inherited-environment probe failed"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "probe did not run"
    );
}
