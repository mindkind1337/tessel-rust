//! Integration tests for Lot b180:
//! 1. SSH persistent `tmux` sessions + static `tmux` bootstrap (`ssh_tmux`)
//! 2. Opt-in per-host `tessel-host` daemon + `Files`/`Changes` caches (`host_remote_cache`)
//! 3. SSH remote agent tools + hardened Claude folder trust (`remote_agent_tools`)

#[path = "../src/ssh_tmux.rs"]
mod ssh_tmux;

#[path = "../src/host_remote_cache.rs"]
mod host_remote_cache;

#[path = "../src/remote_agent_tools.rs"]
mod remote_agent_tools;

use host_remote_cache::*;
use remote_agent_tools::*;
use ssh_tmux::*;
use std::collections::BTreeMap;

#[test]
fn lot_b180_tmux_session_names_versions_and_probe_precedence() {
    assert_eq!(tmux_session_name("pane-12"), "tessel-pane-12");
    assert_eq!(tmux_session_name("a b;c"), "tessel-a-b-c");
    assert_eq!(tmux_session_name(""), "tessel-pane");
    assert!(tmux_session_name(&"x".repeat(100)).len() <= "tessel-".len() + 40);

    assert_eq!(parse_tmux_version("tmux 3.4"), Some(TmuxVersion::new(3, 4)));
    assert_eq!(
        parse_tmux_version("tmux 3.2a"),
        Some(TmuxVersion::new(3, 2))
    );
    assert_eq!(
        parse_tmux_version("tmux next-3.5"),
        Some(TmuxVersion::new(3, 5))
    );
    assert_eq!(parse_tmux_version("bash"), None);
    assert!(TmuxVersion::new(3, 2).supports_prefix_none());
    assert!(!TmuxVersion::new(3, 1).supports_prefix_none());

    let (both, os) = parse_tmux_probe_output("SYS:tmux 3.0a\nOWN:tmux 3.5a\nUNAME:Linux x86_64\n");
    let both = both.unwrap();
    assert_eq!(both.source, TmuxBinarySource::System);
    assert_eq!(both.binary_expr, "tmux");
    assert_eq!(os, Some(("Linux".into(), "x86_64".into())));
    let (own, _) = parse_tmux_probe_output("OWN:tmux 3.5a\n");
    assert_eq!(own.unwrap().source, TmuxBinarySource::Bundled);
    assert!(parse_tmux_probe_output("UNAME:Linux aarch64").0.is_none());
    assert_eq!(
        static_tmux_asset_for_arch("Linux", "arm64").unwrap().arch,
        "aarch64"
    );
    assert!(static_tmux_asset_for_arch("Darwin", "x86_64").is_none());
}

#[test]
fn lot_b180_tmux_options_and_fresh_vs_reattach_commands() {
    let old = tmux_server_options(Some(TmuxVersion::new(3, 1)));
    assert!(old.iter().any(|o| o == "set -g status off"));
    assert!(old.iter().any(|o| o == "set -g history-limit 50000"));
    assert!(!old.iter().any(|o| o.contains("prefix None")));
    let new = tmux_server_options(Some(TmuxVersion::new(3, 4)));
    assert!(new.iter().any(|o| o == "set -g prefix None"));
    assert!(new.iter().any(|o| o == "unbind C-b"));

    let fresh = build_tmux_wrap_command("tmux", "p1", "/srv/app", None, false).unwrap();
    assert!(fresh.contains("kill-session -t '=tessel-p1'"));
    assert!(fresh.contains("new-session -s 'tessel-p1'"));
    assert!(fresh.contains("-L tessel"));
    let again = build_tmux_wrap_command(OWN_TMUX, "p1", "", None, true).unwrap();
    assert!(again.contains("new-session -A -D -s 'tessel-p1'"));
    assert!(!again.contains("kill-session"));
    assert!(build_tmux_wrap_command("rm -rf /", "p1", "", None, true).is_err());
    assert!(build_tmux_wrap_command("tmux", "p1", "/srv/../etc", None, true).is_err());
    assert!(
        build_tmux_kill_session_command("tmux", "p1")
            .unwrap()
            .contains("'=tessel-p1'")
    );

    assert!(can_wrap_ssh_with_tmux(true, None, None));
    assert!(!can_wrap_ssh_with_tmux(false, None, None));
    assert!(!can_wrap_ssh_with_tmux(true, Some("jump"), None));
    assert!(!can_wrap_ssh_with_tmux(true, None, Some("nc %h %p")));

    assert_eq!(
        build_ssh_launch_remote_script("dev", "", false, None, true).unwrap(),
        None
    );
    let plain = build_ssh_launch_remote_script("dev", "/srv", false, None, true)
        .unwrap()
        .unwrap();
    assert!(!plain.contains("tmux"));
    let wrapped = build_ssh_launch_remote_script("dev", "/srv", true, Some("p2"), true)
        .unwrap()
        .unwrap();
    assert!(wrapped.contains("command -v tmux"));
    assert!(wrapped.contains("new-session -A -D -s 'tessel-p2'"));
    assert!(wrapped.ends_with("exec \"${SHELL:-/bin/sh}\" -l"));
}

#[test]
fn lot_b180_static_tmux_install_script_and_notices() {
    let asset = STATIC_TMUX_ASSETS[0];
    let script = build_static_tmux_install_script(asset.url, asset.sha256_hex, "abc123").unwrap();
    assert!(script.contains("umask 077"));
    assert!(script.contains("! -L"));
    assert!(script.contains("exit 83"));
    assert!(script.contains("exit 84"));
    assert!(build_static_tmux_install_script("http://x", asset.sha256_hex, "n").is_err());
    assert!(build_static_tmux_install_script(asset.url, "ABC", "n").is_err());
    assert!(build_static_tmux_install_script(asset.url, asset.sha256_hex, "a;b").is_err());
    assert_eq!(
        classify_tmux_install_exit(0),
        StaticTmuxInstallOutcome::Installed
    );
    assert_eq!(
        classify_tmux_install_exit(83),
        StaticTmuxInstallOutcome::HashMismatch
    );
    assert_eq!(
        classify_tmux_install_exit(84),
        StaticTmuxInstallOutcome::NoExec
    );
    assert_eq!(
        classify_tmux_install_exit(7),
        StaticTmuxInstallOutcome::Failed(7)
    );
    assert!(TmuxBannerKind::NoTmux.message(false).contains("tmux"));
    assert!(
        TmuxBannerKind::ReconnectedSame
            .message(true)
            .starts_with("Reconnecté")
    );
}

#[test]
fn lot_b180_host_daemon_gate_and_direct_exec_command() {
    assert!(validate_direct_exec_connect_command(
        "\"$HOME/.tessel-server/host/bin/tessel-host-0.1.0-x86_64\" connect --stdio"
    ));
    assert!(validate_direct_exec_connect_command(
        "\"$HOME/.tessel-server/host/bin/tessel-host-1.2_b-aarch64\" connect --stdio"
    ));
    for bad in [
        "\"$HOME/.tessel-server/host/bin/tessel-host-0.1.0-x86_64\" connect --stdio; rm -rf ~",
        "\"$HOME/.tessel-server/host/bin/tessel-host-0.1.0-armv7\" connect --stdio",
        "\"$HOME/.tessel-server/host/bin/tessel-host--x86_64\" connect --stdio",
        "\"$HOME/.tessel-server/host/bin/tessel-host-A-x86_64\" connect --stdio",
        "tessel-host connect --stdio",
    ] {
        assert!(!validate_direct_exec_connect_command(bad), "{bad}");
    }
    assert!(!should_use_host_daemon(false, false));
    assert!(should_use_host_daemon(true, false));
    assert!(should_use_host_daemon(false, true));
}

fn entry(name: &str) -> RemoteTreeEntry {
    RemoteTreeEntry {
        name: name.into(),
        path: format!("/w/{name}"),
        is_dir: false,
        size: 1,
    }
}

#[test]
fn lot_b180_remote_tree_cache_reset_gate_post_write_and_refusal() {
    let mut cache = RemoteTreeCache::new();
    let mut dirs = BTreeMap::new();
    dirs.insert("/w".to_owned(), vec![entry("a")]);
    cache.apply_update(RemoteTreeUpdate {
        revision: 1,
        reset: false,
        dirs: dirs.clone(),
        removed_dirs: vec![],
    });
    assert!(!cache.is_ready());
    assert_eq!(
        cache.read_dir_source(true, true, "/w", 0),
        RemoteTreeReadSource::SftpFallback
    );
    cache.apply_update(RemoteTreeUpdate {
        revision: 2,
        reset: true,
        dirs,
        removed_dirs: vec![],
    });
    assert!(cache.is_ready());
    assert_eq!(
        cache.read_dir_source(true, true, "/w", 0),
        RemoteTreeReadSource::HostCache
    );
    assert_eq!(
        cache.read_dir_source(false, true, "/w", 0),
        RemoteTreeReadSource::SftpFallback
    );
    assert_eq!(
        cache.read_dir_source(true, false, "/w", 0),
        RemoteTreeReadSource::SftpFallback
    );

    cache.record_write(1_000);
    assert_eq!(
        cache.read_dir_source(true, true, "/w", 2_000),
        RemoteTreeReadSource::SftpFallback
    );
    assert_eq!(
        cache.read_dir_source(true, true, "/w", 1_000 + POST_WRITE_WAIT_MS),
        RemoteTreeReadSource::HostCache
    );
    cache.record_write(10_000);
    let mut next = BTreeMap::new();
    next.insert("/w".to_owned(), vec![entry("a"), entry("b")]);
    cache.apply_update(RemoteTreeUpdate {
        revision: 3,
        reset: false,
        dirs: next,
        removed_dirs: vec![],
    });
    assert_eq!(
        cache.read_dir_source(true, true, "/w", 10_001),
        RemoteTreeReadSource::HostCache
    );
    assert_eq!(cache.cached_entries("/w").unwrap().len(), 2);

    cache.record_refusal(50_000);
    assert!(!cache.is_ready());
    assert!(!cache.can_open_watch(true, true, 50_000 + REFUSAL_COOLDOWN_MS - 1));
    assert!(cache.can_open_watch(true, true, 50_000 + REFUSAL_COOLDOWN_MS));
    assert!(!cache.can_open_watch(false, true, 1_000_000));
}

fn snap(head: &str) -> RemoteGitSnapshot {
    RemoteGitSnapshot {
        oid: "abc".into(),
        head: head.into(),
        upstream: Some("origin/main".into()),
        ahead: 1,
        behind: 2,
        files: vec![
            RemoteGitFileEntry {
                xy: ['.', 'M'],
                path: "src/a.rs".into(),
                orig_path: None,
            },
            RemoteGitFileEntry {
                xy: ['?', '?'],
                path: "new.txt".into(),
                orig_path: None,
            },
            RemoteGitFileEntry {
                xy: ['R', '.'],
                path: "b.rs".into(),
                orig_path: Some("old_b.rs".into()),
            },
        ],
    }
}

#[test]
fn lot_b180_remote_git_lru_and_porcelain_v2() {
    let text = reconstruct_porcelain_v2(&snap("main"));
    assert!(text.contains("# branch.oid abc\n"));
    assert!(text.contains("# branch.head main\n"));
    assert!(text.contains("# branch.ab +1 -2\n"));
    assert!(text.contains("1 .M N... 100644 100644 100644 0000000 0000000 src/a.rs\n"));
    assert!(text.contains("? new.txt\n"));
    assert!(text.contains("R100 b.rs\told_b.rs\n"));

    let mut lru = RemoteGitLruCache::new();
    for i in 0..MAX_GITS {
        assert_eq!(lru.put(format!("/r{i}"), snap("main")), None);
    }
    assert!(lru.get("/r0").is_some());
    assert_eq!(lru.put("/r9", snap("dev")), Some("/r1".to_owned()));
    assert_eq!(lru.len(), MAX_GITS);
    assert!(lru.contains_root("/r0"));
    lru.invalidate("/r0");
    assert!(!lru.contains_root("/r0"));
}

#[test]
fn lot_b180_remote_tools_relink_proof_of_life_and_pty_block() {
    assert!(supports_remote_agent_tools("Claude"));
    assert!(supports_remote_agent_tools("antigravity"));
    assert!(!supports_remote_agent_tools("aider"));
    assert_eq!(relink_delay_ms(0), 5_000);
    assert_eq!(relink_delay_ms(119), 5_000);
    assert_eq!(relink_delay_ms(120), 60_000);

    let mut st = RemotePaneToolState::register_early(7, "claude");
    assert!(st.registered_early);
    assert_eq!(st.record_relink_failure("bridge down"), 5_000);
    assert!(st.remote_tools_note.is_some());
    assert!(should_block_team_pty_reminder(true, st.remote_tools_ready));
    st.record_proof_of_life();
    assert!(st.remote_tools_ready);
    assert!(st.remote_tools_note.is_none());
    assert!(!should_block_team_pty_reminder(true, st.remote_tools_ready));
    assert!(!should_block_team_pty_reminder(false, false));
    assert!(uninitialized_agent_host_note("Codex", true).contains("Codex"));
}

#[test]
fn lot_b180_antigravity_mcp_merge_and_filesystem_guards() {
    let existing = r#"{"mcpServers":{"tessel-team":{"command":"old"},"tessel_apps":{"command":"x","disabled":true},"other":{"command":"keep"}},"theme":"dark"}"#;
    let merged: serde_json::Value =
        serde_json::from_str(&merge_antigravity_json(Some(existing), "/shim").unwrap()).unwrap();
    let servers = &merged["mcpServers"];
    assert!(servers.get("tessel-team").is_none());
    assert_eq!(servers["other"]["command"], "keep");
    assert_eq!(servers["tessel_team"]["args"], serde_json::json!(["mcp"]));
    assert_eq!(
        servers["tessel_apps"]["args"],
        serde_json::json!(["mcp", "--apps"])
    );
    assert_eq!(servers["tessel_apps"]["disabled"], true);
    assert!(servers["tessel_team"].get("type").is_none());
    assert!(servers["tessel_team"].get("env").is_none());
    assert_eq!(merged["theme"], "dark");
    assert!(merge_antigravity_json(Some("not json"), "/shim").is_err());
    assert!(merge_antigravity_json(None, "").is_err());

    let home = tempfile::tempdir().unwrap();
    assert_eq!(
        install_antigravity_mcp_on_fs(home.path(), "/shim").unwrap(),
        AntigravityInstallOutcome::SkippedUninitialized
    );
    assert!(!home.path().join(".gemini").exists());
    std::fs::create_dir_all(home.path().join(".gemini/config")).unwrap();
    std::fs::write(home.path().join(".gemini/config/mcp_config.json"), existing).unwrap();
    assert_eq!(
        install_antigravity_mcp_on_fs(home.path(), "/shim").unwrap(),
        AntigravityInstallOutcome::Updated
    );
    assert!(
        home.path()
            .join(".gemini/config/mcp_config.json.tessel-bak")
            .is_file()
    );
}

#[test]
fn lot_b180_claude_folder_trust_rejections_and_merge() {
    let home = "/home/u";
    assert!(validate_trust_folder_path("/", home).is_err());
    assert!(validate_trust_folder_path("/home/u", home).is_err());
    assert!(validate_trust_folder_path("/home/u/", home).is_err());
    assert!(validate_trust_folder_path("/home", home).is_err());
    assert!(validate_trust_folder_path("/home/other", home).is_ok());
    assert!(validate_trust_folder_path("/home/user", home).is_ok());
    for alias in ["/home/u/.", "/home//u", "/home/./u/./", "/./", "//"] {
        assert!(validate_trust_folder_path(alias, home).is_err(), "{alias}");
    }
    for alias in ["C:/Users/test/.", "C:/Users//test/", "C:/.", "C://"] {
        assert!(
            validate_trust_folder_path(alias, "C:/Users/test").is_err(),
            "{alias}"
        );
    }
    assert!(validate_trust_folder_path("/home/u/.ssh/x", home).is_err());
    assert!(validate_trust_folder_path("relative/dir", home).is_err());
    assert!(validate_trust_folder_path("/srv/../etc", home).is_err());
    assert_eq!(
        validate_trust_folder_path("/home/u/proj/", home).unwrap(),
        "/home/u/proj"
    );

    let merged: serde_json::Value = serde_json::from_str(
        &merge_claude_trust_json(
            Some(r#"{"projects":{"/a":{"x":1}},"numStartups":3}"#),
            "/home/u/proj",
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(merged["projects"]["/a"]["x"], 1);
    assert_eq!(
        merged["projects"]["/home/u/proj"]["hasTrustDialogAccepted"],
        true
    );
    assert_eq!(merged["numStartups"], 3);

    assert_eq!(
        build_remote_claude_trust_script("/srv/app", false, false).unwrap(),
        None
    );
    assert_eq!(
        build_remote_claude_trust_script("/srv/app", true, true).unwrap(),
        None
    );
    assert!(build_remote_claude_trust_script("/", true, false).is_err());
    assert!(build_remote_claude_trust_script("~/.claude", true, false).is_err());
    let script = build_remote_claude_trust_script("/srv/app", true, false)
        .unwrap()
        .unwrap();
    assert!(script.contains("hasTrustDialogAccepted"));
    assert!(script.contains("! -L"));

    let tmp_home = tempfile::tempdir().unwrap();
    let proj = tmp_home.path().join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    assert!(!apply_local_claude_folder_trust(tmp_home.path(), &proj, false, false).unwrap());
    assert!(!apply_local_claude_folder_trust(tmp_home.path(), &proj, true, true).unwrap());
    assert!(!apply_local_claude_folder_trust(tmp_home.path(), &proj, true, false).unwrap());
    std::fs::write(tmp_home.path().join(".claude.json"), r#"{"numStartups":7}"#).unwrap();
    assert!(apply_local_claude_folder_trust(tmp_home.path(), &proj, true, false).unwrap());
    let raw = std::fs::read_to_string(tmp_home.path().join(".claude.json")).unwrap();
    assert!(raw.contains("hasTrustDialogAccepted"));
    assert!(
        apply_local_claude_folder_trust(tmp_home.path(), tmp_home.path(), true, false).is_err()
    );
    assert!(
        apply_local_claude_folder_trust(tmp_home.path(), &tmp_home.path().join("."), true, false)
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(tmp_home.path().join(".claude.json")).unwrap(),
        raw
    );
}

#[test]
fn claude_trust_serializes_updates_and_preserves_invalid_or_locked_config() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join(".claude.json");
    let a = home.path().join("a");
    let b = home.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    std::fs::write(&config, r#"{"keep":"value"}"#).unwrap();
    std::thread::scope(|scope| {
        let first = scope.spawn(|| apply_local_claude_folder_trust(home.path(), &a, true, false));
        let second = scope.spawn(|| apply_local_claude_folder_trust(home.path(), &b, true, false));
        assert!(first.join().unwrap().unwrap());
        assert!(second.join().unwrap().unwrap());
    });
    let content: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert_eq!(content["keep"], "value");
    assert_eq!(content["projects"].as_object().unwrap().len(), 2);
    assert!(!home.path().join(".claude.json.lock").exists());
    std::fs::write(&config, "invalid original").unwrap();
    assert!(apply_local_claude_folder_trust(home.path(), &a, true, false).is_err());
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        "invalid original"
    );
    assert!(!home.path().join(".claude.json.lock").exists());
    std::fs::create_dir(home.path().join(".claude.json.lock")).unwrap();
    assert!(!apply_local_claude_folder_trust(home.path(), &a, true, false).unwrap());
    assert!(home.path().join(".claude.json.lock").is_dir());
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        "invalid original"
    );
}

#[test]
fn claude_trust_uses_only_the_selected_account_configuration() {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join("project");
    let account = home.path().join("account");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&account).unwrap();
    let original = r#"{"keep":"default account"}"#;
    std::fs::write(home.path().join(".claude.json"), original).unwrap();
    let selected = account.join(".claude.json");
    assert_eq!(
        claude_config_path(home.path(), None, false),
        home.path().join(".claude.json")
    );
    assert_eq!(
        claude_config_path(home.path(), Some(account.as_os_str()), false),
        selected
    );
    assert_eq!(
        claude_config_path(home.path(), Some(account.as_os_str()), true),
        account.join(".claude-custom-oauth.json")
    );
    std::fs::write(account.join(".config.json"), "{}").unwrap();
    assert_eq!(
        claude_config_path(home.path(), Some(account.as_os_str()), true),
        account.join(".config.json")
    );
    std::fs::write(&selected, "{}").unwrap();
    assert!(
        apply_local_claude_folder_trust_at(home.path(), &selected, &project, true, false).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(home.path().join(".claude.json")).unwrap(),
        original
    );
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(selected).unwrap()).unwrap();
    assert_eq!(value["projects"].as_object().unwrap().len(), 1);
}
