#[allow(dead_code)]
mod git {
    use std::path::Path;
    pub fn run(_root: &Path, _args: &[&str], _input: Option<&[u8]>) -> Result<String, String> {
        Ok(String::new())
    }
}
#[allow(dead_code)]
#[path = "../src/dialogs_toolbar_model.rs"]
mod dialogs_toolbar_model;
#[allow(dead_code)]
#[path = "../src/dialogs_toolbar_ui.rs"]
mod dialogs_toolbar_ui;
#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/project_panel.rs"]
mod project_panel;
#[allow(dead_code)]
#[path = "../src/shortcuts.rs"]
mod shortcuts;
#[allow(dead_code)]
#[path = "../src/side_panels_model.rs"]
mod side_panels_model;
#[allow(dead_code)]
#[path = "../src/sidebar_model.rs"]
mod sidebar_model;
#[allow(dead_code)]
#[path = "../src/theme.rs"]
mod theme;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::process::Command;

use dialogs_toolbar_model::{
    GhCheckRollup, NotificationCenter, NotificationKind, UsageAlertLevel, clone_git_repository,
    compute_usage_summary, default_repo_folder_name, discover_nested_git_repos,
    format_fix_failing_checks_prompt, init_new_git_project, parse_gh_issues_json,
    parse_gh_prs_json, validate_safe_clone_url,
};
use dialogs_toolbar_ui::{
    AddProjectTab, DialogsToolbarUiState, GitHubTab, paint_agent_request_pill,
    paint_notifications_toolbar_button, paint_usage_toolbar_button, show_add_project_modal,
    show_github_modal, show_notifications_menu_contents, show_project_group_modal,
    show_usage_menu_contents,
};
use eframe::egui;
use project_panel::{
    ProjectEntry, ProjectPanel, ProjectPanelInput, SessionEntry, SessionKind, SidebarOptions,
};
use side_panels_model::TaskHistoryRecord;
use sidebar_model::AgentDotState;

#[test]
fn nested_git_discovery_and_safe_clone_init_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();

    let repo_a = init_new_git_project(ws, "alpha-service").unwrap();
    assert!(repo_a.join(".git").exists());

    let nested_parent = ws.join("services");
    fs::create_dir_all(&nested_parent).unwrap();
    let repo_b = init_new_git_project(&nested_parent, "beta-worker").unwrap();
    assert!(repo_b.join(".git").exists());

    // Ignored heavy directory should never be traversed
    let ignored = ws.join("node_modules").join("fake-repo").join(".git");
    fs::create_dir_all(&ignored).unwrap();

    let discovered = discover_nested_git_repos(ws, 3, 20);
    assert_eq!(discovered, vec![repo_a.clone(), repo_b]);

    // Option injection & unsafe transport helpers rejected
    assert!(validate_safe_clone_url("-u./payload").is_err());
    assert!(validate_safe_clone_url("--upload-pack=evil").is_err());
    assert!(validate_safe_clone_url("ext::sh -c whoami").is_err());
    assert!(validate_safe_clone_url("fd::1").is_err());
    assert!(validate_safe_clone_url("https://github.com/org/tessel-rust.git").is_ok());
    assert_eq!(
        default_repo_folder_name("git@github.com:org/tessel-rust.git"),
        "tessel-rust"
    );

    // Commit a file in repo_a and clone it safely
    fs::write(repo_a.join("README.md"), "# Alpha\n").unwrap();
    assert!(
        Command::new("git")
            .current_dir(&repo_a)
            .args(["config", "user.email", "test@tessel.dev"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .current_dir(&repo_a)
            .args(["config", "user.name", "Tessel Test"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .current_dir(&repo_a)
            .args(["add", "README.md"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .current_dir(&repo_a)
            .args(["commit", "-m", "Initial commit"])
            .status()
            .unwrap()
            .success()
    );

    let clones_dir = ws.join("clones");
    fs::create_dir_all(&clones_dir).unwrap();
    let cloned =
        clone_git_repository(&repo_a.to_string_lossy(), &clones_dir, "alpha-clone").unwrap();
    assert!(cloned.join("README.md").is_file());
}

#[test]
fn github_json_parsers_and_fix_checks_prompt() {
    let issues_json = r#"[
        {"number": 42, "title": "Add nested repo picker", "state": "OPEN", "author": {"login": "alice"}, "url": "https://github.com/org/repo/issues/42"}
    ]"#;
    let issues = parse_gh_issues_json(issues_json);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].number, 42);
    assert_eq!(issues[0].author, "alice");

    let prs_json = r#"[
        {
            "number": 108,
            "title": "Feat: toolbar usage gauge",
            "headRefName": "feat/usage",
            "baseRefName": "main",
            "state": "OPEN",
            "isDraft": false,
            "url": "https://github.com/org/repo/pull/108",
            "statusCheckRollup": [
                {"name": "clippy", "conclusion": "FAILURE", "detailsUrl": "https://ci.example/1"},
                {"name": "unit-tests", "conclusion": "SUCCESS", "detailsUrl": "https://ci.example/2"}
            ]
        },
        {
            "number": 109,
            "title": "Chore: docs",
            "headRefName": "chore/docs",
            "baseRefName": "main",
            "state": "OPEN",
            "isDraft": false,
            "url": "https://github.com/org/repo/pull/109",
            "statusCheckRollup": [
                {"name": "build", "state": "IN_PROGRESS"}
            ]
        }
    ]"#;
    let prs = parse_gh_prs_json(prs_json);
    assert_eq!(prs.len(), 2);
    assert_eq!(prs[0].rollup, GhCheckRollup::Failing);
    assert_eq!(prs[0].failing_checks.len(), 1);
    assert_eq!(prs[1].rollup, GhCheckRollup::Pending);

    let prompt = format_fix_failing_checks_prompt(&prs[0]);
    assert!(prompt.contains("PR #108"));
    assert!(prompt.contains("clippy (FAILURE)"));
    assert!(prompt.contains("https://ci.example/1"));
}

#[test]
fn usage_thresholds_and_coalesced_notifications() {
    let now = 2_000_000_000_000_u64;
    let records = vec![
        TaskHistoryRecord {
            id: 1,
            project_id: 1,
            title: "Build UI".into(),
            agent_kind: "claude".into(),
            duration_ms: 90_000,
            tokens: 45_000,
            cost_usd: 6.50,
            done_at_ms: now - 1_000,
            ..TaskHistoryRecord::default()
        },
        TaskHistoryRecord {
            id: 2,
            project_id: 1,
            title: "Tests".into(),
            agent_kind: "codex".into(),
            duration_ms: 60_000,
            tokens: 20_000,
            cost_usd: 2.00,
            done_at_ms: now - 500,
            ..TaskHistoryRecord::default()
        },
    ];

    let warn_summary = compute_usage_summary(&records[..1], 10.0, now);
    assert_eq!(warn_summary.level, UsageAlertLevel::Warn);

    let danger_summary = compute_usage_summary(&records, 10.0, now);
    assert_eq!(danger_summary.level, UsageAlertLevel::Danger);
    assert_eq!(danger_summary.providers.len(), 2);

    let mut center = NotificationCenter::default();
    center.push_coalesced(
        NotificationKind::ApprovalNeeded,
        "Agent waiting for response",
        "Pane 3",
        Some(3),
    );
    center.push_coalesced(
        NotificationKind::ApprovalNeeded,
        "Agent waiting for response",
        "Pane 3 updated",
        Some(3),
    );
    assert_eq!(center.items.len(), 1);
    assert_eq!(center.items[0].count, 2);
    assert_eq!(center.unread_count(), 1);

    center.mark_all_read();
    assert_eq!(center.unread_count(), 0);
    center.clear();
    assert!(center.items.is_empty());
}

#[test]
fn dialogs_toolbar_and_sidebar_groups_render_headless_en_fr() {
    let ctx = egui::Context::default();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::default()));

    let mut state = DialogsToolbarUiState::new();
    state.add_project_open = true;
    state.github_open = true;
    state.group_modal_project = Some(1);
    state.group_modal_draft = "Core".into();
    state.github_prs = parse_gh_prs_json(
        r#"[{"number":12,"title":"Sync","headRefName":"feat/sync","baseRefName":"main","state":"OPEN","statusCheckRollup":[{"name":"ci","conclusion":"SUCCESS"}]}]"#,
    );

    let summary = compute_usage_summary(&[], 10.0, 1_000_000);
    let mut notif = NotificationCenter::default();
    notif.push_coalesced(
        NotificationKind::TaskCompleted,
        "Task done",
        "b170",
        Some(1),
    );

    let mut panel = ProjectPanel::new();
    let mut groups = HashMap::new();
    groups.insert(1_u64, "Core".to_owned());
    panel.set_project_groups(groups, HashSet::new());
    let mut dot_states = HashMap::new();
    dot_states.insert(10_u64, AgentDotState::Working);
    panel.set_session_states(dot_states);
    panel.set_options(SidebarOptions {
        sort_by_activity: false,
        compact_rows: false,
        hide_sleeping: false,
        hide_agent_task_line: true,
    });
    let projects = vec![ProjectEntry {
        id: 1,
        name: "Tessel".into(),
        path: "C:/Tessel-rust".into(),
        sessions: vec![SessionEntry {
            id: 10,
            title: "Claude".into(),
            kind: SessionKind::Agent,
            sleeping: false,
        }],
    }];

    for french in [false, true] {
        for add_tab in [
            AddProjectTab::LocalFolder,
            AddProjectTab::CloneRepo,
            AddProjectTab::NewProject,
            AddProjectTab::SshRemote,
        ] {
            state.add_tab = add_tab;
            state.github_tab = if french {
                GitHubTab::Issues
            } else {
                GitHubTab::PullRequests
            };
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::Vec2::new(1280.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    let pal = &theme::active(ctx).spec().ui;
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let _ = paint_agent_request_pill(ui, pal, 2, french);
                        let _ = paint_usage_toolbar_button(ui, pal, summary.level, french, false);
                        let _ = show_usage_menu_contents(ui, pal, &summary, french);
                        let _ = paint_notifications_toolbar_button(
                            ui,
                            pal,
                            notif.unread_count(),
                            french,
                            false,
                        );
                        let _ = show_notifications_menu_contents(ui, pal, &notif, french);
                        let _ = panel.show(
                            ui,
                            ProjectPanelInput {
                                projects: &projects,
                                global_sessions: &[],
                                active_project: Some(1),
                                active_session: Some(10),
                                french,
                            },
                        );
                    });
                    let aliases = vec!["prod-box".to_owned()];
                    let existing = vec!["Core".to_owned(), "Services".to_owned()];
                    let _ = show_add_project_modal(ctx, &mut state, &aliases, french);
                    let _ = show_github_modal(ctx, &mut state, "Tessel", french);
                    let _ = show_project_group_modal(ctx, &mut state, &existing, french);
                },
            );
        }
    }
}
