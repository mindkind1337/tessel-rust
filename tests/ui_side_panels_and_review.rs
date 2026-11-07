#[allow(dead_code)]
mod git {
    use std::io::Write;
    use std::path::Path;
    use std::process::{Command, Stdio};

    pub fn run(root: &Path, args: &[&str], input: Option<&[u8]>) -> Result<String, String> {
        let mut cmd = Command::new("git");
        cmd.current_dir(root)
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
            stdin.write_all(bytes).map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
            if !out.stderr.is_empty() {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            Err(combined)
        }
    }
}
#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/side_panels_model.rs"]
mod side_panels_model;
#[allow(dead_code)]
#[path = "../src/side_panels_ui.rs"]
mod side_panels_ui;
#[allow(dead_code)]
#[path = "../src/theme.rs"]
mod theme;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use side_panels_model::{
    ActivityMessage, ActivityStore, CommitNode, ConversationMediaKind, ConversationWindow,
    DashboardGroupBy, DashboardItem, DashboardState, InlineReviewComment, NotesModel,
    SessionHistoryEntry, SessionHistoryScope, TaskHistoryFilter, TaskHistoryRecord,
    TaskHistorySort, TaskHistoryStore, build_conversation_threads, build_dashboard,
    build_git_history_lanes, filter_and_sort_task_history, filter_sessions,
    format_review_request_changes, history_csv, load_branch_review, load_git_commit_history,
    mask_transcript_secrets, merge_review_branch,
};
use side_panels_ui::{
    SidePanelsUiState, show_conversations_panel, show_dashboard_panel, show_notes_modal,
    show_review_modal, show_scm_history_section, show_session_history_panel, show_sessions_modal,
    show_task_history_panel,
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tessel-sidepanels-{prefix}-{}-{}",
        std::process::id(),
        side_panels_model::now_unix_ms()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn dashboard_groups_filters_and_identifies_sleepable_idle_panes() {
    let items = vec![
        DashboardItem {
            pane_id: 1,
            project_id: Some(10),
            project_name: "Alpha".into(),
            title: "Ada".into(),
            agent_kind: "claude".into(),
            branch: Some("main".into()),
            task_line: Some("Fixing auth".into()),
            state: DashboardState::Waiting,
            is_chat: true,
            is_focused: false,
        },
        DashboardItem {
            pane_id: 2,
            project_id: Some(10),
            project_name: "Alpha".into(),
            title: "Turing".into(),
            agent_kind: "codex".into(),
            branch: Some("feat/ui".into()),
            task_line: None,
            state: DashboardState::Idle,
            is_chat: false,
            is_focused: false,
        },
        DashboardItem {
            pane_id: 3,
            project_id: Some(20),
            project_name: "Beta".into(),
            title: "Hopper".into(),
            agent_kind: "gemini".into(),
            branch: None,
            task_line: None,
            state: DashboardState::Idle,
            is_chat: false,
            is_focused: true,
        },
    ];

    let vm = build_dashboard(&items, DashboardGroupBy::Status, None, "", false);
    assert_eq!(vm.total, 3);
    assert_eq!(vm.sleepable_pane_ids, vec![2]);
    assert_eq!(vm.groups.len(), 2);
    assert_eq!(vm.groups[0].heading, "Waiting");
    assert_eq!(vm.groups[1].heading, "Idle");

    let by_proj = build_dashboard(&items, DashboardGroupBy::Project, None, "alpha", true);
    assert_eq!(by_proj.groups.len(), 1);
    assert_eq!(by_proj.groups[0].heading, "Alpha");
    assert_eq!(by_proj.groups[0].items.len(), 2);
}

#[test]
fn task_history_store_filters_sorts_and_exports_formula_safe_csv() {
    let dir = unique_temp_dir("task-hist");
    let mut store = TaskHistoryStore::default();
    let now = 1_800_000_000_000_u64;
    store.record_completed(TaskHistoryRecord {
        id: 0,
        task_id: 11,
        title: "=CMD|' /C calc'!A0".into(),
        project_id: 1,
        project_name: "+DangerProj".into(),
        agent_name: "Ada".into(),
        agent_kind: "claude".into(),
        created_at_ms: now - 120_000,
        done_at_ms: now - 60_000,
        duration_ms: 0,
        tokens: 4_200,
        cost_usd: 0.125,
    });
    store.record_completed(TaskHistoryRecord {
        id: 0,
        task_id: 12,
        title: "Normal task, with comma".into(),
        project_id: 2,
        project_name: "Safe".into(),
        agent_name: "Bohr".into(),
        agent_kind: "codex".into(),
        created_at_ms: now - 20_000,
        done_at_ms: now - 5_000,
        duration_ms: 15_000,
        tokens: 1_000,
        cost_usd: 0.025,
    });
    store.save(&dir).expect("save history");

    let reloaded = TaskHistoryStore::load(&dir);
    assert_eq!(reloaded.records.len(), 2);
    assert_eq!(reloaded.records[0].duration_ms, 60_000);

    let (sorted_cost, totals) = filter_and_sort_task_history(
        &reloaded.records,
        None,
        TaskHistoryFilter::Today,
        TaskHistorySort::Cost,
        "",
        now,
    );
    assert_eq!(sorted_cost.len(), 2);
    assert_eq!(sorted_cost[0].task_id, 11);
    assert_eq!(totals.tokens, 5_200);

    let csv = history_csv(&sorted_cost);
    assert!(csv.starts_with("\u{feff}Title,Project,Agent"));
    assert!(csv.contains("'=CMD|' /C calc'!A0,'+DangerProj"));
    assert!(csv.contains("\"Normal task, with comma\""));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn session_history_opt_in_transcript_search_masks_secrets() {
    let dir = unique_temp_dir("sessions");
    let transcript = dir.join("sess-1.jsonl");
    fs::write(
        &transcript,
        "{\"cwd\":\"C:/repo\",\"summary\":\"Deploy worker\"}\n{\"message\":{\"content\":\"Leak sk-1234567890abcdefghijklmnop needle_token\"}}\n",
    )
    .expect("write jsonl");

    let entries = vec![SessionHistoryEntry {
        id: "sess-1".into(),
        agent_id: "claude".into(),
        title: "Deploy worker".into(),
        folder: PathBuf::from("C:/repo"),
        updated_at_ms: 100,
        file_path: Some(transcript),
        open_pane_id: None,
        transcript_snippet: None,
    }];

    let meta_only = filter_sessions(
        &entries,
        SessionHistoryScope::All,
        None,
        &[],
        None,
        "needle_token",
        false,
    );
    assert!(meta_only.is_empty());

    let with_transcripts = filter_sessions(
        &entries,
        SessionHistoryScope::All,
        None,
        &[],
        None,
        "needle_token",
        true,
    );
    assert_eq!(with_transcripts.len(), 1);
    let snippet = with_transcripts[0]
        .transcript_snippet
        .as_deref()
        .unwrap_or("");
    assert!(snippet.contains("needle_token"));
    assert!(!snippet.contains("sk-1234567890abcdef"));
    assert!(mask_transcript_secrets("key sk-1234567890abcdefghij").contains("••••••••"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn notes_model_detects_external_disk_conflict_and_resolves_both_ways() {
    let dir = unique_temp_dir("notes");
    let mut notes = NotesModel::default();
    notes.load_project(Some(&dir)).expect("initial load");
    notes.text = "# Sprint plan\n- [x] Ship b169\n".into();
    assert!(notes.save(false).expect("first save"));

    // Local edit while external writer updates `.tessel/notes.md`
    notes.text.push_str("- [ ] Local item\n");
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(
        NotesModel::notes_path(&dir),
        "# Sprint plan\n- [x] External change\n",
    )
    .expect("external write");
    // Ensure mtime differs even on coarse filesystems
    notes.loaded_mtime_ms = Some(1);

    let saved = notes.save(false).expect("conflict check");
    assert!(!saved);
    assert!(notes.conflict.is_some());

    // Keep mine overwrites cleanly
    assert!(notes.save(true).expect("force save"));
    assert!(notes.conflict.is_none());
    let disk = fs::read_to_string(NotesModel::notes_path(&dir)).expect("read notes");
    assert!(disk.contains("Local item"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn git_swimlanes_and_branch_review_merge_guard_work_on_real_repo() {
    let repo = unique_temp_dir("git-review");
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(&repo)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    };

    run(&["init", "-b", "main"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    fs::write(repo.join("README.md"), "hello\n").unwrap();
    run(&["add", "README.md"]);
    run(&["commit", "-m", "initial"]);

    run(&["checkout", "-b", "agent/task-1"]);
    fs::write(repo.join("feature.rs"), "pub fn answer() -> u32 { 42 }\n").unwrap();
    run(&["add", "feature.rs"]);
    run(&["commit", "-m", "feat: add answer"]);
    run(&["checkout", "main"]);

    let snap = load_branch_review(&repo, "main", "agent/task-1", None).expect("load review");
    assert_eq!(snap.ahead, 1);
    assert_eq!(snap.behind, 0);
    assert_eq!(snap.files.len(), 1);
    assert_eq!(snap.files[0].path, "feature.rs");
    assert!(snap.can_clean_merge());

    // Stale expectHead must refuse to merge
    let stale_err =
        merge_review_branch(&repo, "main", "agent/task-1", "0000000000000000").unwrap_err();
    assert!(stale_err.contains("moved"));

    // Matching expectHead merges cleanly with --no-ff
    merge_review_branch(&repo, "main", "agent/task-1", &snap.head_oid).expect("clean merge");

    let commits = load_git_commit_history(&repo, 20).expect("load history");
    assert!(commits.len() >= 3);
    assert!(commits[0].is_merge);

    let synthetic = vec![
        CommitNode {
            hash: "c3".into(),
            short_hash: "c3".into(),
            parents: vec!["c1".into(), "c2".into()],
            author: "Ada".into(),
            date_short: "2026-11-07".into(),
            refs: vec!["HEAD -> main".into()],
            subject: "Merge branch".into(),
        },
        CommitNode {
            hash: "c2".into(),
            short_hash: "c2".into(),
            parents: vec!["c1".into()],
            author: "Ada".into(),
            date_short: "2026-11-07".into(),
            refs: vec![],
            subject: "Branch commit".into(),
        },
        CommitNode {
            hash: "c1".into(),
            short_hash: "c1".into(),
            parents: vec![],
            author: "Ada".into(),
            date_short: "2026-11-07".into(),
            refs: vec![],
            subject: "Root".into(),
        },
    ];
    let lanes = build_git_history_lanes(&synthetic);
    assert_eq!(lanes.len(), 3);
    assert!(lanes[0].active_lanes >= 2);

    let feedback = format_review_request_changes(
        "agent/task-1",
        "Please add unit tests.",
        &[InlineReviewComment {
            file: "feature.rs".into(),
            line: 1,
            body: "Document return value".into(),
        }],
    );
    assert!(feedback.contains("`feature.rs:1`"));

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn side_panels_and_modals_render_headless_in_en_and_fr() {
    let ctx = eframe::egui::Context::default();
    theme::apply(&ctx, true, Some(theme_palettes::ThemeId::default()));

    let mut ui_state = SidePanelsUiState::new();
    ui_state.notes_modal_open = true;
    ui_state.notes.text = "# Heading\n- [x] Done\n```rust\nlet x = 1;\n```\n".into();
    ui_state.notes.preview_mode = true;
    ui_state.sessions_modal_open = true;
    ui_state.review_modal_open = true;

    let mut act_store = ActivityStore::default();
    act_store.push(ActivityMessage {
        id: 0,
        thread_key: "team:core".into(),
        thread_title: "Core Team".into(),
        pane_id: Some(1),
        sender: "Ada".into(),
        recipient: "Team".into(),
        body: "See https://example.com and ```rust\nfn main() {}\n``` in src/main.rs".into(),
        timestamp_ms: side_panels_model::now_unix_ms(),
    });
    let threads = build_conversation_threads(
        &act_store.messages,
        ConversationWindow::Days7,
        Some(ConversationMediaKind::Code),
        "",
        side_panels_model::now_unix_ms(),
    );
    assert_eq!(threads.len(), 1);

    for french in [false, true] {
        let _ = ctx.run(eframe::egui::RawInput::default(), |ctx| {
            eframe::egui::CentralPanel::default().show(ctx, |ui| {
                let _ = show_dashboard_panel(ui, &mut ui_state, &[], french);
                let _ = show_task_history_panel(ui, &mut ui_state, &[], Some(1), french);
                let _ = show_session_history_panel(ui, &mut ui_state, &[], None, &[], french);
                let _ = show_conversations_panel(ui, &mut ui_state, &act_store.messages, french);
                let _ = show_scm_history_section(ui, &mut ui_state, &[], french);
            });
            let _ = show_notes_modal(ctx, &mut ui_state, "Demo", french);
            let _ = show_sessions_modal(ctx, &mut ui_state, &[], None, &[], french);
            let _ = show_review_modal(ctx, &mut ui_state, &[], french);
        });
    }
}
