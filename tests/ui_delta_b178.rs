//! Integration tests for Phase C (`Lot b178`):
//! 1. `Settings > Agents` singular/plural model count & persistent invalid-`env` callout (`8863fd0a`, `7bf60732`)
//! 2. `Settings > Accounts` duplicate `"System default"` suppression & JWT/profile email probing (`5382e434`)
//! 3. Terminal `.exit-overlay` pill copy & grouped narrow-pane wrapping (`8982166c`, `a587e1b9`)
//! 4. Remote HTTP MCP `401`/`403` OAuth probe, per-agent `Sign in` / `Sign in all`, & `google-workspace` retirement (`c89ba908`..`ebfff271`)
//! 5. `Settings > Tessel Account` strict boolean `admin` gating & tooltip (`c21f56c3`)
//! 6. Toolbar live Git build label (`bXXXX (7chars)`), `↑ bXXXX` pill, & safe fast-forward updater (`51738282`..`00f76c7e`)

#[allow(dead_code)]
#[path = "../src/agent_env.rs"]
mod agent_env;

#[allow(dead_code)]
#[path = "../src/agent_arg_options.rs"]
mod agent_arg_options;

#[allow(dead_code)]
#[path = "../src/agent_model_catalog.rs"]
mod agent_model_catalog;

#[allow(dead_code)]
#[path = "../src/agent_catalog.rs"]
mod agent_catalog;

#[allow(dead_code)]
#[path = "../src/vault.rs"]
mod vault;

#[allow(dead_code)]
mod workspace {
    use serde::{Deserialize, Serialize};
    use std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
    };

    pub type Result<T> = std::result::Result<T, String>;

    #[derive(Clone, Debug, Default, Serialize, Deserialize)]
    pub struct Workspace {
        #[serde(default)]
        pub agent_model_defaults: BTreeMap<String, crate::agent_model_defaults::Defaults>,
    }

    pub struct Store {
        file: PathBuf,
    }

    impl Store {
        pub fn open(dir: &Path) -> Result<(Self, Workspace)> {
            let file = dir.join("workspace.json");
            let state = match fs::read(&file) {
                Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
                Err(_) => Workspace::default(),
            };
            Ok((Self { file }, state))
        }

        pub fn save(&self, state: &Workspace) -> Result<()> {
            let bytes = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
            fs::write(&self.file, bytes).map_err(|e| e.to_string())
        }
    }
}

#[allow(dead_code)]
#[path = "../src/agent_model_defaults.rs"]
mod agent_model_defaults;

#[allow(dead_code)]
#[path = "../src/agent_model_launch.rs"]
mod agent_model_launch;

#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

#[allow(dead_code)]
#[path = "../src/theme.rs"]
mod theme;

#[allow(dead_code)]
#[path = "../src/settings_surface.rs"]
mod settings_surface;

#[allow(dead_code)]
#[path = "../src/agent_model_defaults_ui.rs"]
mod agent_model_defaults_ui;

#[allow(dead_code)]
#[path = "../src/settings_extra_model.rs"]
mod settings_extra_model;

#[allow(dead_code)]
#[path = "../src/account.rs"]
mod account;

#[path = "../src/dev_update.rs"]
mod dev_update;

#[path = "../src/terminal_exit.rs"]
mod terminal_exit;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;

#[test]
fn lot_b178_models_listed_singular_and_invalid_env_callout() {
    assert_eq!(
        agent_model_defaults_ui::format_models_listed(1, "Claude Code", "2026-11-07", false),
        "1 model listed by Claude Code on 2026-11-07."
    );
    assert_eq!(
        agent_model_defaults_ui::format_models_listed(2, "Claude Code", "2026-11-07", false),
        "2 models listed by Claude Code on 2026-11-07."
    );
    assert_eq!(
        agent_model_defaults_ui::format_models_listed(1, "Claude Code", "07/11/2026", true),
        "1 modèle listé par Claude Code le 07/11/2026."
    );
    assert_eq!(
        agent_model_defaults_ui::format_models_listed(3, "Claude Code", "07/11/2026", true),
        "3 modèles listés par Claude Code le 07/11/2026."
    );

    let err = agent_env::parse("BROKEN_LINE").expect_err("invalid env line must fail");
    let en = agent_env::format_invalid_env_callout(&err, false);
    let fr = agent_env::format_invalid_env_callout(&err, true);
    assert!(en.ends_with("Not used until fixed."));
    assert!(fr.ends_with("Non utilisées tant que ce n'est pas corrigé."));
}

#[test]
fn lot_b178_system_account_covered_and_profile_email_probing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let data = tmp.path().join("data");
    fs::create_dir_all(home.join(".codex")).expect("mkdir .codex");
    fs::write(
        home.join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"Alice@Example.com"}}"#,
    )
    .expect("write .claude.json");

    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
    let payload = URL_SAFE_NO_PAD.encode(r#"{"email":"codex.user@example.com"}"#);
    let jwt = format!("{header}.{payload}.sig");
    fs::write(
        home.join(".codex").join("auth.json"),
        serde_json::json!({ "tokens": { "id_token": jwt } }).to_string(),
    )
    .expect("write codex auth.json");

    assert_eq!(
        settings_extra_model::probe_system_provider_email(&home, "claude").as_deref(),
        Some("Alice@Example.com")
    );
    assert_eq!(
        settings_extra_model::probe_system_provider_email(&home, "codex").as_deref(),
        Some("codex.user@example.com")
    );

    let acct = settings_extra_model::add_managed_account(&data, "claude", "Work")
        .expect("add managed claude account");
    let profile_dir =
        settings_extra_model::managed_account_profile_dir(&data, "claude", &acct.id).unwrap();
    fs::create_dir_all(&profile_dir).expect("mkdir profile");
    fs::write(
        profile_dir.join(".claude.json"),
        r#"{"email":"alice@example.com"}"#,
    )
    .expect("write profile .claude.json");

    let mut roster = settings_extra_model::load_managed_accounts(&data, "claude");
    assert_eq!(roster.accounts[0].email, "alice@example.com");

    // Unselected -> System default is NOT covered
    roster.selected_id = None;
    assert!(!settings_extra_model::system_account_covered(
        Some("Alice@Example.com"),
        &roster
    ));

    // Selected with matching email (case-insensitive) -> System default IS covered
    roster.selected_id = Some(acct.id.clone());
    assert!(settings_extra_model::system_account_covered(
        Some("Alice@Example.com"),
        &roster
    ));

    // Blank system email -> NOT covered
    assert!(!settings_extra_model::system_account_covered(
        Some("   "),
        &roster
    ));

    // Switched to different email -> NOT covered
    assert!(!settings_extra_model::system_account_covered(
        Some("bob@example.com"),
        &roster
    ));

    // Deselected -> NOT covered
    roster.selected_id = None;
    assert!(!settings_extra_model::system_account_covered(
        Some("Alice@Example.com"),
        &roster
    ));
}

#[test]
fn lot_b178_terminal_exit_banner_copy_and_narrow_wrap() {
    let (msg_en, act_en, close_en) =
        terminal_exit::terminal_exit_banner_copy(false, Some(130), false);
    assert_eq!(msg_en, "Process exited with code 130.");
    assert_eq!(act_en, "Restart");
    assert_eq!(close_en, "Close pane");

    let (msg_fr, act_fr, close_fr) = terminal_exit::terminal_exit_banner_copy(false, None, true);
    assert_eq!(msg_fr, "Le processus s'est terminé.");
    assert_eq!(act_fr, "Redémarrer");
    assert_eq!(close_fr, "Fermer le panneau");

    let (err_en, retry_en, _) = terminal_exit::terminal_exit_banner_copy(true, None, false);
    assert_eq!(err_en, "This terminal couldn't start.");
    assert_eq!(retry_en, "Retry");

    let (err_fr, retry_fr, close_err_fr) =
        terminal_exit::terminal_exit_banner_copy(true, None, true);
    assert_eq!(err_fr, "Ce terminal n'a pas pu démarrer.");
    assert_eq!(retry_fr, "Réessayer");
    assert_eq!(close_err_fr, "Fermer le panneau");

    assert!(terminal_exit::exit_overlay_actions_wrap_together(
        280.0, 170.0, 170.0, 10.0
    ));
    assert!(!terminal_exit::exit_overlay_actions_wrap_together(
        420.0, 170.0, 170.0, 10.0
    ));
}

fn spawn_one_shot_status_server(status_line: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
    let addr = listener.local_addr().expect("local_addr");
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let reply =
                format!("HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}/mcp")
}

#[test]
fn lot_b178_mcp_http_oauth_probe_signin_planner_and_retire_old_google() {
    let url_401 = spawn_one_shot_status_server("401 Unauthorized");
    let url_403 = spawn_one_shot_status_server("403 Forbidden");
    assert_eq!(
        settings_extra_model::probe_mcp_http("linear", &url_401).expect("401 should succeed"),
        settings_extra_model::McpProbeOutcome::SignsInFromAgent
    );
    assert_eq!(
        settings_extra_model::probe_mcp_http("notion", &url_403).expect("403 should succeed"),
        settings_extra_model::McpProbeOutcome::SignsInFromAgent
    );
    assert_eq!(
        settings_extra_model::McpProbeOutcome::signs_in_badge(false),
        "Signs in from the agent"
    );
    assert_eq!(
        settings_extra_model::McpProbeOutcome::signs_in_badge(true),
        "Connexion par l'agent"
    );

    // Sign-in planner
    let agents = vec![
        ("codex".to_owned(), "Codex".to_owned()),
        ("opencode".to_owned(), "OpenCode".to_owned()),
        ("claude".to_owned(), "Claude Code".to_owned()),
        ("gemini".to_owned(), "Gemini CLI".to_owned()),
    ];
    let plan = settings_extra_model::plan_mcp_sign_in(&agents, "linear_prod", false)
        .expect("valid server name");
    assert_eq!(
        plan.commands,
        vec![
            "codex mcp login linear_prod".to_owned(),
            "opencode mcp auth linear_prod".to_owned(),
        ]
    );
    let card_msg = plan
        .card_message
        .as_deref()
        .expect("in-session instructions");
    assert!(card_msg.contains("/mcp"));
    assert!(
        settings_extra_model::plan_mcp_sign_in(&agents, "bad;rm -rf", false).is_err(),
        "shell metacharacters must be rejected"
    );

    // RetireOldGoogleTracker
    let tmp = tempfile::tempdir().expect("tempdir");
    let data_dir = tmp.path();
    let claude_cfg = data_dir.join("claude.json");
    let cursor_cfg = data_dir.join("cursor.json");
    let installed = vec![
        settings_extra_model::InstalledMcpServer {
            agent_id: "claude".into(),
            agent_label: "Claude".into(),
            config_path: claude_cfg.clone(),
            name: "tessel_apps".into(),
            transport: settings_extra_model::McpTransport::Http,
            endpoint_summary: "http://127.0.0.1/mcp".into(),
            env_keys: vec![],
        },
        settings_extra_model::InstalledMcpServer {
            agent_id: "claude".into(),
            agent_label: "Claude".into(),
            config_path: claude_cfg.clone(),
            name: "google-workspace".into(),
            transport: settings_extra_model::McpTransport::Stdio,
            endpoint_summary: "npx google-workspace".into(),
            env_keys: vec![],
        },
        // Cursor does NOT hold `tessel_apps`, so it must keep `google-workspace`!
        settings_extra_model::InstalledMcpServer {
            agent_id: "cursor".into(),
            agent_label: "Cursor".into(),
            config_path: cursor_cfg,
            name: "google-workspace".into(),
            transport: settings_extra_model::McpTransport::Stdio,
            endpoint_summary: "npx google-workspace".into(),
            env_keys: vec![],
        },
    ];

    let mut tracker = settings_extra_model::RetireOldGoogleTracker::default();
    let mut calls: Vec<String> = Vec::new();
    // 1) Not connected -> no-op
    assert!(
        !tracker.sync_with_remover(data_dir, false, &installed, |_, a, _| {
            calls.push(a.to_owned());
            Ok(())
        })
    );
    assert!(calls.is_empty());

    // 2) Connected, but removal fails -> goes to `refused` (not persisted to `done`)
    assert!(
        !tracker.sync_with_remover(data_dir, true, &installed, |_, a, _| {
            calls.push(format!("fail:{a}"));
            Err("locked".into())
        })
    );
    assert_eq!(calls, vec!["fail:claude".to_owned()]);
    assert!(!settings_extra_model::RetireOldGoogleTracker::state_file_path(data_dir).exists());

    // 3) Reconnect transition (`false` -> `true`) clears `refused` and succeeds; Cursor untouched!
    let _ = tracker.sync_with_remover(data_dir, false, &installed, |_, _, _| Ok(()));
    calls.clear();
    assert!(
        tracker.sync_with_remover(data_dir, true, &installed, |_, a, s| {
            calls.push(format!("{a}:{s}"));
            Ok(())
        })
    );
    assert_eq!(calls, vec!["claude:google-workspace".to_owned()]);
    assert!(settings_extra_model::RetireOldGoogleTracker::state_file_path(data_dir).exists());

    // 4) Fresh tracker on startup reads `old-google-retired.json` and skips already-retired `claude`
    let mut fresh_tracker = settings_extra_model::RetireOldGoogleTracker::default();
    calls.clear();
    assert!(
        !fresh_tracker.sync_with_remover(data_dir, true, &installed, |_, a, _| {
            calls.push(a.to_owned());
            Ok(())
        })
    );
    assert!(calls.is_empty());
}

#[test]
fn lot_b178_account_user_strict_admin_boolean_and_tooltip() {
    assert_eq!(account::ADMIN_PORTAL_URL, "https://admin.tesselcode.com");
    assert!(account::admin_button_tooltip(false).contains("admin.tesselcode.com"));
    assert!(account::admin_button_tooltip(true).contains("admin.tesselcode.com"));

    let u_true: account::User =
        serde_json::from_str(r#"{"id":"u_1234567890abcdef","email":"a@b.com","admin":true}"#)
            .expect("admin: true");
    assert!(u_true.admin);

    for raw in [
        r#"{"id":"u_1234567890abcdef","email":"a@b.com","admin":false}"#,
        r#"{"id":"u_1234567890abcdef","email":"a@b.com","admin":"true"}"#,
        r#"{"id":"u_1234567890abcdef","email":"a@b.com","admin":1}"#,
        r#"{"id":"u_1234567890abcdef","email":"a@b.com","admin":null}"#,
        r#"{"id":"u_1234567890abcdef","email":"a@b.com"}"#,
    ] {
        let parsed: account::User = serde_json::from_str(raw).expect("User must still deserialize");
        assert!(!parsed.admin, "expected admin == false for payload: {raw}");
    }
}

fn run_git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn lot_b178_dev_update_porcelain_overlap_and_git_fast_forward() {
    // Trimmed leading space + rename handling (`5421b63e`)
    let paths = dev_update::porcelain_paths(
        "M src/first.rs\n M src/second.rs\nR  src/old.rs -> src/new.rs\n",
    );
    assert!(paths.contains(&"src/first.rs".to_owned()));
    assert!(paths.contains(&"src/second.rs".to_owned()));
    assert!(paths.contains(&"src/old.rs".to_owned()));
    assert!(paths.contains(&"src/new.rs".to_owned()));

    let overlap_fr = dev_update::format_overlap_message(&["src/second.rs".to_owned()], true);
    assert!(overlap_fr.contains("«src/second.rs»"));

    // Real temporary upstream + downstream Git checkouts
    let tmp = tempfile::tempdir().expect("tempdir");
    let bare = tmp.path().join("origin.git");
    let seed = tmp.path().join("seed");
    let local = tmp.path().join("local");

    fs::create_dir_all(&bare).unwrap();
    run_git(&bare, &["init", "--bare", "-b", "main"]);

    run_git(
        tmp.path(),
        &["clone", bare.to_str().unwrap(), seed.to_str().unwrap()],
    );
    run_git(&seed, &["config", "user.email", "dev@example.com"]);
    run_git(&seed, &["config", "user.name", "Dev"]);
    fs::write(seed.join("a.txt"), "v1\n").unwrap();
    fs::write(seed.join("b.txt"), "local-safe\n").unwrap();
    run_git(&seed, &["add", "a.txt", "b.txt"]);
    run_git(&seed, &["commit", "-m", "initial"]);
    run_git(&seed, &["push", "-u", "origin", "main"]);

    run_git(
        tmp.path(),
        &["clone", bare.to_str().unwrap(), local.to_str().unwrap()],
    );
    run_git(&local, &["config", "user.email", "dev@example.com"]);
    run_git(&local, &["config", "user.name", "Dev"]);

    // Advance upstream on `a.txt`
    fs::write(seed.join("a.txt"), "v2\n").unwrap();
    run_git(&seed, &["add", "a.txt"]);
    run_git(&seed, &["commit", "-m", "update a"]);
    run_git(&seed, &["push"]);

    // Modify `b.txt` (non-overlapping!) in `local` and fetch
    fs::write(local.join("b.txt"), "my-work-in-progress\n").unwrap();
    let st = dev_update::fetch_and_look(&local).expect("look after fetch");
    assert_eq!(st.behind, 1);
    assert_eq!(st.remote_build, 2);
    assert_eq!(st.pill_label().as_deref(), Some("↑ b2"));
    assert!(
        st.blocked.is_none(),
        "non-overlapping edit on b.txt must NOT block"
    );

    // Now dirty `a.txt` (overlapping!) -> must block with Dirty { overlap: ["a.txt"] }
    fs::write(local.join("a.txt"), "conflicting-local\n").unwrap();
    let blocked_res = dev_update::apply(&local);
    assert_eq!(
        blocked_res,
        dev_update::ApplyDevUpdateResult::Dirty {
            overlap: vec!["a.txt".to_owned()]
        }
    );

    // Restore `a.txt` while keeping `b.txt` modified -> `apply` fast-forwards cleanly!
    run_git(&local, &["checkout", "--", "a.txt"]);
    match dev_update::apply(&local) {
        dev_update::ApplyDevUpdateResult::Updated(updated) => {
            assert_eq!(updated.local_build, 2);
            assert_eq!(updated.behind, 0);
        }
        other => panic!("expected Updated, got {other:?}"),
    }
    assert_eq!(
        fs::read_to_string(local.join("a.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "v2\n"
    );
    assert_eq!(
        fs::read_to_string(local.join("b.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "my-work-in-progress\n"
    );
}
