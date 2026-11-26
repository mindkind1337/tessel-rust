#[allow(dead_code)]
#[path = "../src/agent_catalog.rs"]
mod agent_catalog;
#[allow(dead_code)]
#[path = "../src/agent_env.rs"]
mod agent_env;
#[allow(dead_code)]
#[path = "../src/automations.rs"]
mod automations;
#[allow(dead_code)]
#[path = "../src/i18n.rs"]
mod i18n;
#[allow(dead_code)]
#[path = "../src/settings_extra_model.rs"]
mod settings_extra_model;
#[allow(dead_code)]
#[path = "../src/settings_extra_panel.rs"]
mod settings_extra_panel;
#[allow(dead_code)]
#[path = "../src/settings_model.rs"]
mod settings_model;
#[allow(dead_code)]
#[path = "../src/theme.rs"]
mod theme;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

#[cfg(windows)]
#[allow(dead_code)]
mod vault {
    pub fn protect(data: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
        // Deterministic XOR envelope for fast isolated UI/model unit tests;
        // real Windows DPAPI `vault::protect` is tested in `src/vault.rs`.
        let key = b"tessel-test-seal";
        let mut out = Vec::with_capacity(data.len() + 4);
        if encrypt {
            out.extend_from_slice(b"DP01");
            for (i, &b) in data.iter().enumerate() {
                out.push(b ^ key[i % key.len()]);
            }
            Ok(out)
        } else {
            let body = data.strip_prefix(b"DP01").ok_or("invalid test cipher")?;
            for (i, &b) in body.iter().enumerate() {
                out.push(b ^ key[i % key.len()]);
            }
            Ok(out)
        }
    }
}

use automations::{
    Automation, AutomationAgent, AutomationIsolation, AutomationSchedule, AutomationsFile,
    validate_cron_min_15m, write_automation_prompt_file,
};
use eframe::egui;
use settings_extra_model::{
    CUSTOM_AGENT_SWATCHES, CliSignInProbe, ContextModelFamily, ContextPreferences,
    ContextScanReport, CustomAgent, DEVTOOLS_CATALOG, DevToolStatus, ManagedAccountsRoster,
    McpTransport, NotificationPreferences, OrchestrationPreferences, ProviderAccountsPreferences,
    VoicePreferences, active_managed_env_line, add_managed_account, install_skill_from_folder,
    load_provider_secret_flags, parse_skill_frontmatter, remove_json_mcp_server, remove_skill_dir,
    scan_installed_mcp_servers, scan_project_context, scan_skills, set_provider_secret,
    upsert_json_mcp_server, validate_custom_agents,
};
use settings_extra_panel::{AccountsViewInput, SettingsExtraState};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

fn run_headless(mut f: impl FnMut(&mut egui::Ui)) {
    let ctx = egui::Context::default();
    theme::apply(&ctx, true, None);
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1080.0, 900.0),
        )),
        ..Default::default()
    };
    let _ = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            f(ui);
        });
    });
}

#[test]
fn renders_all_ten_remaining_settings_pages_and_custom_agents_in_en_and_fr() {
    let mut state = SettingsExtraState::new();
    let mut accounts_prefs = ProviderAccountsPreferences::default();
    let claude_roster = ManagedAccountsRoster::default();
    let codex_roster = ManagedAccountsRoster::default();
    let secret_flags = BTreeMap::new();
    let cli_sign_ins = CliSignInProbe {
        grok_signed_in: true,
        muse_signed_in: false,
        cursor_signed_in: true,
    };
    let ssh_aliases = vec!["prod-eu".to_owned()];
    let mut context_prefs = ContextPreferences::default();
    let context_report = ContextScanReport::default();
    let mut orch_prefs = OrchestrationPreferences::default();
    let mut notif_prefs = NotificationPreferences::default();
    let mut voice_prefs = VoicePreferences::default();
    let devtools: Vec<DevToolStatus> = DEVTOOLS_CATALOG
        .iter()
        .enumerate()
        .map(|(idx, spec)| DevToolStatus {
            id: spec.id.to_owned(),
            installed: idx % 2 == 0,
            version: if idx % 2 == 0 {
                "1.0.0".to_owned()
            } else {
                String::new()
            },
            extra_note: String::new(),
        })
        .collect();
    let auto_store = AutomationsFile::default();
    let projects = vec![(1_u64, "tessel-rust".to_owned(), PathBuf::from("C:/repo"))];
    let custom_agents = vec![CustomAgent {
        id: "custom-aider".into(),
        name: "Aider".into(),
        command: "aider --model sonnet".into(),
        accent: CUSTOM_AGENT_SWATCHES[3].into(),
        enabled: true,
    }];
    let targets = vec!["Claude Code (~/.claude.json)".to_owned()];

    for french in [false, true] {
        run_headless(|ui| {
            let input = AccountsViewInput {
                claude_roster: &claude_roster,
                codex_roster: &codex_roster,
                claude_system_email: None,
                codex_system_email: None,
                secret_saved_flags: &secret_flags,
                cli_sign_ins: &cli_sign_ins,
                ssh_host_aliases: &ssh_aliases,
                french,
            };
            let _ = state.show_accounts(ui, &mut accounts_prefs, &input);
            let _ = state.show_skills(ui, &[], true, french);
            let _ = state.show_mcp(ui, &[], &targets, french);
            let _ = state.show_connections(ui, true, &[], french);
            let _ = state.show_context(ui, &mut context_prefs, &context_report, french);
            let _ = state.show_orchestration(ui, &mut orch_prefs, french);
            let _ = state.show_alerts(ui, &mut notif_prefs, french);
            let _ = state.show_voice(ui, &mut voice_prefs, french);
            let _ = state.show_devtools(ui, &devtools, false, french);
            let _ = state.show_automations(ui, &auto_store, &projects, french);
            let _ = state.show_custom_agents_section(ui, &custom_agents, french);
        });
    }
}

#[test]
fn custom_agents_validate_uniqueness_and_builtin_collisions() {
    let valid = vec![CustomAgent {
        id: "custom-101".into(),
        name: "My Custom Bot".into(),
        command: "my-bot --chat".into(),
        accent: "#5b8def".into(),
        enabled: true,
    }];
    assert!(validate_custom_agents(&valid).is_ok());

    let collide_builtin = vec![CustomAgent {
        id: "custom-102".into(),
        name: "Claude Code".into(),
        command: "claude".into(),
        accent: "#5b8def".into(),
        enabled: true,
    }];
    assert!(validate_custom_agents(&collide_builtin).is_err());

    let duplicate_names = vec![
        CustomAgent {
            id: "custom-1".into(),
            name: "Alpha".into(),
            command: "alpha".into(),
            accent: "#5b8def".into(),
            enabled: true,
        },
        CustomAgent {
            id: "custom-2".into(),
            name: "alpha".into(),
            command: "alpha2".into(),
            accent: "#56b870".into(),
            enabled: true,
        },
    ];
    assert!(validate_custom_agents(&duplicate_names).is_err());
}

#[test]
fn managed_accounts_and_provider_secrets_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let added = add_managed_account(dir.path(), "claude", "Work Team").expect("add claude account");
    let mut roster = settings_extra_model::load_managed_accounts(dir.path(), "claude");
    assert_eq!(roster.accounts.len(), 1);
    roster.selected_id = Some(added.id.clone());
    settings_extra_model::save_managed_accounts(dir.path(), "claude", &roster)
        .expect("save selected");
    let env_line = active_managed_env_line(dir.path(), "claude").expect("env line");
    assert!(env_line.starts_with("CLAUDE_CONFIG_DIR="));

    #[cfg(windows)]
    {
        set_provider_secret(dir.path(), "minimaxApiKey", "sk-secret-value").expect("save secret");
        let flags = load_provider_secret_flags(dir.path());
        assert_eq!(flags.get("minimaxApiKey").copied(), Some(true));
        set_provider_secret(dir.path(), "minimaxApiKey", "").expect("clear secret");
        let flags_after = load_provider_secret_flags(dir.path());
        assert_eq!(flags_after.get("minimaxApiKey").copied(), Some(false));
    }
}

#[test]
fn skills_frontmatter_install_scan_and_remove_work_cleanly() {
    let md = "---\nname: rust-audit\ndescription: Audits unsafe blocks\n---\n# Body\n";
    let (name, desc) = parse_skill_frontmatter(md);
    assert_eq!(name.as_deref(), Some("rust-audit"));
    assert_eq!(desc.as_deref(), Some("Audits unsafe blocks"));

    let home = tempfile::tempdir().expect("home");
    let proj = tempfile::tempdir().expect("proj");
    let src = tempfile::tempdir().expect("src");
    let named_src = src.path().join("rust-audit");
    fs::create_dir_all(&named_src).unwrap();
    fs::write(named_src.join("SKILL.md"), md).unwrap();

    let dest_root = proj.path().join(".agents").join("skills");
    let installed = install_skill_from_folder(&named_src, &dest_root).expect("install skill");
    assert!(installed.join("SKILL.md").is_file());

    let found = scan_skills(home.path(), Some(proj.path()));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "rust-audit");

    remove_skill_dir(&installed).expect("remove skill");
    assert!(!installed.exists());
}

#[test]
fn mcp_json_upsert_creates_backup_and_scans_back() {
    let home = tempfile::tempdir().expect("home");
    let cfg = home.path().join(".claude.json");
    fs::write(&cfg, r#"{"mcpServers":{}}"#).unwrap();

    upsert_json_mcp_server(
        &cfg,
        "claude",
        "playwright",
        McpTransport::Stdio,
        "npx",
        &["-y".into(), "@playwright/mcp@latest".into()],
        &[],
    )
    .expect("upsert mcp");

    assert!(cfg.with_extension("tessel-bak").is_file());
    let scanned = scan_installed_mcp_servers(home.path(), None);
    assert_eq!(scanned.len(), 1);
    assert_eq!(scanned[0].name, "playwright");

    remove_json_mcp_server(&cfg, "claude", "playwright").expect("remove mcp");
    let after = scan_installed_mcp_servers(home.path(), None);
    assert!(after.is_empty());
}

#[test]
fn context_scanner_estimates_tokens_and_ignores_dotenv() {
    let home = tempfile::tempdir().expect("home");
    let proj = tempfile::tempdir().expect("proj");
    fs::write(proj.path().join("CLAUDE.md"), "Always run cargo clippy.\n").unwrap();
    fs::write(proj.path().join(".env"), "SECRET_KEY=do_not_read\n").unwrap();

    let report = scan_project_context(
        home.path(),
        Some(proj.path()),
        &[],
        &[],
        ContextModelFamily::default(),
    );
    assert_eq!(report.rules_files.len(), 1);
    assert!(report.total_tokens > 0);
}

#[test]
fn automations_enforce_15m_cron_floor_and_signature_confirmation() {
    assert!(validate_cron_min_15m("* * * * *").is_err());
    assert!(validate_cron_min_15m("*/5 * * * *").is_err());
    assert!(validate_cron_min_15m("*/15 * * * *").is_ok());
    assert!(validate_cron_min_15m("0 9 * * 1-5").is_ok());

    let mut auto = Automation {
        id: "auto-1".into(),
        name: "Morning check".into(),
        project_id: 1,
        project_path: PathBuf::from("C:/repo"),
        agent: AutomationAgent::Claude,
        isolation: AutomationIsolation::Worktree,
        schedule: AutomationSchedule::Daily { hour: 9, minute: 0 },
        prompt: "Run cargo test and summarize failures.".into(),
        enabled: false,
        confirmed_sig: None,
        last_minute_slot: None,
        runs: Vec::new(),
    };
    assert!(!auto.is_confirmed());
    auto.mark_confirmed();
    assert!(auto.is_confirmed());

    // Changing the prompt invalidates the confirmation signature!
    auto.prompt.push_str(" Also push to main.");
    assert!(!auto.is_confirmed());

    let dir = tempfile::tempdir().expect("data");
    let prompt_path = write_automation_prompt_file(dir.path(), "run-123", &auto.prompt)
        .expect("write prompt file");
    assert!(prompt_path.is_file());
}
