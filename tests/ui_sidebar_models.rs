#[path = "../src/finder_model.rs"]
mod finder_model;
#[path = "../src/sidebar_model.rs"]
mod sidebar_model;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use finder_model::{
    ALL_HISTORY_AGENTS, AgentCandidate, DEFAULT_MAX_ENTRIES_PER_DIR, DEFAULT_MAX_SINGLE_PATH_BYTES,
    DEFAULT_MAX_TOTAL_PATH_BYTES, DEFAULT_MAX_VISITED_ENTRIES, DEFAULT_MAX_WALK_DEPTH,
    DEFAULT_MAX_WALKED_DIRS, DEFAULT_WALK_TIMEOUT, DROP_RETRIES, DROP_RETRY_WINDOW_MS,
    FILE_FINDER_LIMIT, FileFinderAction, FileFinderState, FinderScanCoordinator, HostAgentProbe,
    LAUNCH_MENU_VIEWPORT_MARGIN, LaunchMenuAction, LaunchMenuItem, LaunchMenuState,
    LaunchPlacement, LauncherChoice, LauncherItem, LauncherLang, MAX_FINDER_QUERY_CHARS,
    MAX_LISTED_FILES, ProjectLauncherAction, ProjectLauncherState, ProjectOpenSetting,
    REMOTE_AGENT_IDS, RemoteAgentStatus, RemoteDropCheck, SKIP_DIRS, SSH_DROPPED_EXIT_CODE,
    SessionFilterOpts, SessionRecord, SessionScope, SessionSort, ShellCandidate,
    WIN_FILE_ATTRIBUTE_REPARSE_POINT, WalkLimits, WalkStopReason, agent_display_name,
    allow_drop_retry, available_placements, clamp_finder_query, clamp_launch_menu_pos, dir_of,
    dropped_remote_pane, emptied_workspace_choice, filter_sessions, folder_key, folder_label,
    fuzzy_filter, fuzzy_score, has_reparse_point_attr, in_folder, is_within_canonical_scope,
    launcher_agents, launcher_shells, name_of, project_open_choice, recent_project_sessions,
    remembered_value, remote_agent_found, valid_project_open, walk_project_files,
    walk_project_files_with_limits, where_text,
};
use sidebar_model::{
    APP_HISTORY_KEY, AgentDotState, AgentSummaryInput, AppResourceEntry, CardSortBy,
    CardVisibilityOpts, DiscoveredWorktree, HIDDEN_GROUP_LIMIT, HIDDEN_PREVIEW_LIMIT,
    HISTORY_CAPACITY, HISTORY_STALE_MS, KeepAwakeMode, ListeningPortEntry, PaneBadgeInput,
    PaneHeaderBadge, PaneRuntimeKind, ProjectFolder, ProjectOrderBy, RESOURCE_BACKGROUND_POLL_MS,
    RESOURCE_OPEN_POLL_MS, ResourceHistory, ResourceSnapshot, ResourceSortBy, SPARK_SLOTS,
    STATUS_BAR_COMPACT_BELOW, STATUS_BAR_ICON_ONLY_BELOW, SUMMARY_STATE_ORDER, SidebarBuildOpts,
    SidebarGroupBy, SidebarLang, SidebarPaneInput, SidebarProjectInput, SidebarRow,
    StatusBarDensity, TaskCopyInput, TerminalMeta, TerminalResourceSnapshot, WorktreeCardStatus,
    agent_state_label, attention_of, awake_copy, build_project_cards, build_sidebar_rows,
    card_status, card_target_pane, compare_cards, folder_name, format_cpu, format_memory,
    format_short_time_ago, group_worktrees_by_parent_path, hidden_worktrees_preview,
    is_card_visible, merge_project_order, move_to_index, neighbor_card, norm_sidebar_path,
    order_workspaces, pane_dot_state, pane_header_badge, pane_row, port_probes, ports_summary,
    project_drop_index, project_other_branches, project_worktree_visibility, resource_tree,
    same_path, select_summary_group_icon_agents, sparkline_lead, sparkline_points,
    status_bar_density, summarize_agents, summary_state_word, worktree_parent_path,
};

fn sample_pane(
    id: &str,
    kind: PaneRuntimeKind,
    state: Option<&str>,
    since: u64,
    sleeping: bool,
) -> SidebarPaneInput {
    SidebarPaneInput {
        id: id.to_string(),
        kind,
        agent_id: kind.is_agent_pane().then(|| "claude".to_string()),
        name: format!("Pane {id}"),
        title: format!("Title {id}"),
        sleeping,
        state: state.map(ToString::to_string),
        state_since: since,
        last_output_at: since,
        started_at: 1_000,
        worktree_path: None,
        worktree_branch: None,
    }
}

#[test]
fn agent_dot_states_summaries_and_time_ago_match_reference() {
    assert_eq!(SUMMARY_STATE_ORDER.len(), 8);
    assert_eq!(
        pane_dot_state(PaneRuntimeKind::Shell, false, Some("working")),
        AgentDotState::Idle
    );
    assert_eq!(
        pane_dot_state(PaneRuntimeKind::Editor, false, Some("working")),
        AgentDotState::Idle
    );
    assert_eq!(
        pane_dot_state(PaneRuntimeKind::Browser, false, Some("approval")),
        AgentDotState::Idle
    );
    assert_eq!(
        pane_dot_state(PaneRuntimeKind::Agent, true, Some("approval")),
        AgentDotState::Idle
    );
    assert_eq!(
        pane_dot_state(PaneRuntimeKind::Agent, false, Some("approval")),
        AgentDotState::Waiting
    );
    assert_eq!(
        pane_dot_state(PaneRuntimeKind::Chat, false, Some("limited")),
        AgentDotState::Blocked
    );
    assert_eq!(
        pane_dot_state(PaneRuntimeKind::Agent, false, Some("waiting")),
        AgentDotState::Done
    );
    assert_eq!(
        pane_dot_state(PaneRuntimeKind::Agent, false, Some("monitoring")),
        AgentDotState::Monitoring
    );
    assert_eq!(
        pane_dot_state(PaneRuntimeKind::Agent, false, Some("unknown")),
        AgentDotState::Unverifiable
    );
    assert_eq!(
        pane_dot_state(PaneRuntimeKind::Chat, false, Some("stopped")),
        AgentDotState::Interrupted
    );

    assert_eq!(
        agent_state_label(AgentDotState::Waiting, SidebarLang::En),
        "Waiting for your reply"
    );
    assert_eq!(
        agent_state_label(AgentDotState::Waiting, SidebarLang::Fr),
        "En attente de votre réponse"
    );
    assert_eq!(
        agent_state_label(AgentDotState::Monitoring, SidebarLang::En),
        "Monitoring background tasks"
    );
    assert_eq!(
        agent_state_label(AgentDotState::Blocked, SidebarLang::Fr),
        "Bloqué"
    );
    assert_eq!(
        agent_state_label(AgentDotState::Interrupted, SidebarLang::En),
        "Interrupted"
    );
    assert_eq!(
        agent_state_label(AgentDotState::Done, SidebarLang::Fr),
        "Terminé"
    );
    assert_eq!(
        agent_state_label(AgentDotState::Idle, SidebarLang::En),
        "Idle"
    );
    assert_eq!(
        agent_state_label(AgentDotState::Unverifiable, SidebarLang::Fr),
        "Aucune mise à jour récente"
    );
    assert_eq!(
        summary_state_word(AgentDotState::Unverifiable, SidebarLang::En),
        "not heard from"
    );

    let agents = vec![
        AgentSummaryInput {
            agent_id: Some("codex".to_string()),
            name: "Hopper".to_string(),
            state: AgentDotState::Working,
        },
        AgentSummaryInput {
            agent_id: Some("claude".to_string()),
            name: "Darwin".to_string(),
            state: AgentDotState::Waiting,
        },
        AgentSummaryInput {
            agent_id: Some("claude".to_string()),
            name: "Halley".to_string(),
            state: AgentDotState::Waiting,
        },
    ];
    let sum_en = summarize_agents(&agents, "Agents", SidebarLang::En).unwrap();
    assert_eq!(sum_en.lead_state, AgentDotState::Waiting);
    assert_eq!(sum_en.summary_label, "Agents: 2 waiting, 1 working");
    assert_eq!(
        sum_en.identities,
        "Hopper working, Darwin waiting, Halley waiting"
    );

    let sum_fr = summarize_agents(&agents[1..], "Agents", SidebarLang::Fr).unwrap();
    assert_eq!(sum_fr.summary_label, "Agents : tous en attente");

    let icons = select_summary_group_icon_agents(&agents);
    assert_eq!(icons.len(), 1);
    assert_eq!(icons[0].agent_id.as_deref(), Some("claude"));

    assert_eq!(format_short_time_ago(0, 100_000, SidebarLang::En), "");
    assert_eq!(
        format_short_time_ago(70_000, 100_000, SidebarLang::En),
        "< 1m"
    );
    assert_eq!(
        format_short_time_ago(70_000, 100_000, SidebarLang::Fr),
        "< 1 min"
    );
    assert_eq!(
        format_short_time_ago(1, 5 * 60_000 + 1, SidebarLang::En),
        "5m"
    );
    assert_eq!(
        format_short_time_ago(1, 3 * 3_600_000 + 1, SidebarLang::Fr),
        "3 h"
    );
    assert_eq!(
        format_short_time_ago(1, 2 * 86_400_000 + 1, SidebarLang::Fr),
        "2 j"
    );
}

#[test]
fn project_cards_worktree_visibility_and_sidebar_rows_match_reference() {
    assert!(same_path("C:\\Tessel-rust\\", "c:/tessel-rust"));
    assert_eq!(norm_sidebar_path("C:\\Tessel\\"), "c:/tessel");
    assert_eq!(folder_name("C:\\work\\feat-sidebar\\"), "feat-sidebar");
    assert_eq!(worktree_parent_path("C:\\repo"), "C:\\");
    assert_eq!(worktree_parent_path("/repo"), "/");
    assert_eq!(worktree_parent_path("C:\\work\\copy-a"), "C:\\work");

    let mut pane_copy = sample_pane("p2", PaneRuntimeKind::Chat, Some("approval"), 9_000, false);
    pane_copy.worktree_path = Some("C:\\work\\t2-ui".to_string());
    pane_copy.worktree_branch = Some("feat/ui".to_string());

    let mut unread = HashSet::new();
    unread.insert("ws1::".to_string());
    unread.insert("ws1::C:\\work\\t2-ui".to_string());

    let project = SidebarProjectInput {
        id: "ws1".to_string(),
        name: "Tessel Rust".to_string(),
        cwd: "C:\\Tessel-rust".to_string(),
        branch: "main".to_string(),
        default_branch: "main".to_string(),
        remote_host_id: None,
        remote_host_label: None,
        is_active_workspace: true,
        active_pane_id: Some("p1".to_string()),
        active_view_key: String::new(),
        created_at: 1_000,
        panes: vec![
            sample_pane("p1", PaneRuntimeKind::Agent, Some("working"), 5_000, false),
            sample_pane("ed1", PaneRuntimeKind::Editor, None, 8_000, false),
            pane_copy,
        ],
        task_copies: vec![TaskCopyInput {
            path: "C:\\work\\t2-empty".to_string(),
            branch: "feat/empty".to_string(),
            created_at: 2_000,
        }],
        unread_keys: unread,
        shown_worktree_paths: vec!["C:\\ext\\wt-shown".to_string()],
        worktrees_inbox_dismissed: false,
        discovered_worktrees: vec![
            DiscoveredWorktree {
                path: "C:\\Tessel-rust".to_string(),
                branch: "main".to_string(),
                head: "abc".to_string(),
                is_primary: true,
                locked: false,
                detached: false,
            },
            DiscoveredWorktree {
                path: "C:\\ext\\wt-shown".to_string(),
                branch: "ext/shown".to_string(),
                head: "def".to_string(),
                is_primary: false,
                locked: true,
                detached: false,
            },
            DiscoveredWorktree {
                path: "C:\\ext\\wt-hidden-1".to_string(),
                branch: "ext/h1".to_string(),
                head: "111".to_string(),
                is_primary: false,
                locked: false,
                detached: false,
            },
            DiscoveredWorktree {
                path: "C:\\ext\\wt-detached".to_string(),
                branch: String::new(),
                head: "222".to_string(),
                is_primary: false,
                locked: false,
                detached: true,
            },
        ],
        collapsed: false,
    };

    assert_eq!(project_other_branches(&project).len(), 3);
    let vis = project_worktree_visibility(&project);
    assert_eq!(vis.shown.len(), 1);
    assert_eq!(vis.hidden.len(), 2);
    assert_eq!(vis.inbox.len(), 1); // detached worktree excluded from inbox
    assert!(vis.show_banner);

    let preview = hidden_worktrees_preview(&vis.hidden, &HashSet::new());
    assert_eq!(HIDDEN_GROUP_LIMIT, 5);
    assert_eq!(HIDDEN_PREVIEW_LIMIT, 3);
    assert_eq!(preview.groups.len(), 1);
    assert_eq!(preview.groups[0].total, 2);
    assert_eq!(group_worktrees_by_parent_path(&vis.hidden).len(), 1);

    let cards = build_project_cards(&project, 10_000, SidebarLang::En);
    assert_eq!(cards.len(), 4); // main + task_copy + shown_wt + unlisted live pane copy
    assert!(cards[0].is_main && cards[0].is_active && !cards[0].is_unread);
    assert_eq!(cards[0].status, WorktreeCardStatus::Working);
    assert_eq!(cards[0].status.label(SidebarLang::Fr), "Au travail");
    // Empty task copy has `[].every(sleeping) == true` and status Inactive.
    assert!(cards[1].sleeping);
    assert_eq!(cards[1].status, WorktreeCardStatus::Inactive);
    assert_eq!(
        WorktreeCardStatus::Sleeping.label(SidebarLang::En),
        "Sleeping"
    );
    // Unlisted copy card with approval chat pane gets Permission + class 1 + unread.
    let ui_card = cards.iter().find(|c| c.name == "t2-ui").unwrap();
    assert_eq!(ui_card.status, WorktreeCardStatus::Permission);
    assert_eq!(ui_card.attention.cls, 1);
    assert!(ui_card.is_unread);
    assert_eq!(card_target_pane(&cards[0]), Some("p1"));

    // Verify `pane_row`, `card_status` & `attention_of` directly.
    let single_row = pane_row(
        &sample_pane("s1", PaneRuntimeKind::Shell, None, 2_000, false),
        0,
        None,
        SidebarLang::En,
    );
    assert_eq!(
        card_status(std::slice::from_ref(&single_row)),
        WorktreeCardStatus::Active
    );
    assert_eq!(attention_of(std::slice::from_ref(&single_row)).cls, 5);

    // Visibility: active card stays visible even when `hide_default_branch` & `hide_sleeping` are set.
    let strict_vis = CardVisibilityOpts {
        project_filter: None,
        search_query: String::new(),
        hide_sleeping: true,
        except_default_branch: false,
        hide_default_branch: true,
    };
    assert!(is_card_visible(&cards[0], &strict_vis));
    assert!(!is_card_visible(&cards[1], &strict_vis));

    // Build rows with folder + empty folder + sorting + neighbor wrap.
    let sleeping_project = SidebarProjectInput {
        id: "ws2".to_string(),
        name: "Alpha".to_string(),
        cwd: "C:\\Alpha".to_string(),
        branch: "main".to_string(),
        default_branch: "main".to_string(),
        is_active_workspace: false,
        ..SidebarProjectInput::default()
    };
    let build_opts = SidebarBuildOpts {
        group_by: SidebarGroupBy::Repo,
        sort_by: CardSortBy::Smart,
        project_order_by: ProjectOrderBy::Manual,
        project_order: vec!["ws1".to_string(), "ws2".to_string()],
        folders: vec![
            ProjectFolder {
                id: "f1".to_string(),
                name: "Core".to_string(),
                project_ids: vec!["ws1".to_string()],
                collapsed: false,
            },
            ProjectFolder {
                id: "f-empty".to_string(),
                name: "Scratch".to_string(),
                project_ids: Vec::new(),
                collapsed: false,
            },
        ],
        visibility: strict_vis,
    };
    let projects = vec![project.clone(), sleeping_project];
    let rows = build_sidebar_rows(&projects, &build_opts, 10_000, SidebarLang::En);
    assert!(
        matches!(rows.first(), Some(SidebarRow::Folder { folder_id, .. }) if folder_id == "f1")
    );
    assert!(rows.iter().any(|r| matches!(r, SidebarRow::HiddenSummary { project_id, count: 1, .. } if project_id == "ws2")));
    assert!(
        matches!(rows.last(), Some(SidebarRow::Folder { folder_id, count: 0, .. }) if folder_id == "f-empty")
    );

    let next = neighbor_card(&rows, "ws1::", 1).unwrap();
    assert_eq!(next.name, "t2-ui");
    let prev = neighbor_card(&rows, "ws1::", -1).unwrap();
    assert_eq!(prev.name, "t2-ui");

    // Flat group_by == None + remaining sort modes.
    for sort_by in [
        CardSortBy::Name,
        CardSortBy::Recent,
        CardSortBy::Repo,
        CardSortBy::Manual,
    ] {
        let mut sorted = cards.clone();
        sorted.sort_by(|a, b| compare_cards(a, b, sort_by));
        assert_eq!(sorted.len(), 4);
    }
    let flat_rows = build_sidebar_rows(
        &projects,
        &SidebarBuildOpts {
            group_by: SidebarGroupBy::None,
            ..SidebarBuildOpts::default()
        },
        10_000,
        SidebarLang::En,
    );
    assert!(
        flat_rows
            .iter()
            .all(|r| matches!(r, SidebarRow::Card { .. }))
    );

    // Drag-reorder helpers & port probes.
    let ids = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    assert_eq!(project_drop_index(&ids, "b", true), Some(2));
    assert_eq!(
        move_to_index(&ids, &"a".to_string(), 2),
        vec!["b".to_string(), "a".to_string(), "c".to_string()]
    );
    assert_eq!(
        merge_project_order(&["c".to_string(), "ghost".to_string()], &ids),
        vec!["c".to_string(), "a".to_string(), "b".to_string()]
    );
    assert_eq!(
        order_workspaces(&projects, ProjectOrderBy::Recent, &[])[0].id,
        "ws1"
    );
    assert_eq!(
        order_workspaces(
            &projects,
            ProjectOrderBy::Manual,
            &["ws2".to_string(), "ws1".to_string()]
        )[0]
        .id,
        "ws2"
    );

    let mut pids = HashMap::new();
    pids.insert("p1".to_string(), 4242);
    let probes = port_probes(&projects, &pids, 10_000);
    assert!(
        probes
            .iter()
            .any(|pr| pr.key == "ws1::" && pr.pids == vec![4242])
    );
}

#[test]
fn status_bar_resource_sparklines_and_header_badges_match_reference() {
    assert_eq!(STATUS_BAR_COMPACT_BELOW, 900.0);
    assert_eq!(STATUS_BAR_ICON_ONLY_BELOW, 500.0);
    assert_eq!(RESOURCE_OPEN_POLL_MS, 2_000);
    assert_eq!(RESOURCE_BACKGROUND_POLL_MS, 30_000);
    assert_eq!(status_bar_density(1200.0), StatusBarDensity::Full);
    assert_eq!(status_bar_density(750.0), StatusBarDensity::Compact);
    assert_eq!(status_bar_density(420.0), StatusBarDensity::IconOnly);

    assert_eq!(format_memory(None, SidebarLang::En), "—");
    assert_eq!(
        format_memory(Some(512.0 * 1024.0), SidebarLang::En),
        "512 KB"
    );
    assert_eq!(
        format_memory(Some(512.0 * 1024.0), SidebarLang::Fr),
        "512 Ko"
    );
    assert_eq!(
        format_memory(Some(128.5 * 1024.0 * 1024.0), SidebarLang::En),
        "128.5 MB"
    );
    assert_eq!(
        format_memory(Some(128.5 * 1024.0 * 1024.0), SidebarLang::Fr),
        "128,5 Mo"
    );
    assert_eq!(
        format_memory(Some(2.25 * 1024.0 * 1024.0 * 1024.0), SidebarLang::Fr),
        "2,25 Go"
    );
    assert_eq!(format_cpu(Some(41.7), SidebarLang::En), "41.7%");
    assert_eq!(format_cpu(Some(41.7), SidebarLang::Fr), "41,7 %");

    let awake = awake_copy(KeepAwakeMode::Agents, true, SidebarLang::Fr);
    assert_eq!(awake.status, "Agent · Actif");
    assert_eq!(
        awake_copy(KeepAwakeMode::On, true, SidebarLang::En).status,
        "On · Active"
    );
    assert_eq!(
        awake_copy(KeepAwakeMode::Off, false, SidebarLang::En).status,
        "Off · Inactive"
    );

    let mut term_map = HashMap::new();
    term_map.insert(
        "t1".to_string(),
        TerminalResourceSnapshot {
            id: "t1".to_string(),
            memory_bytes: 200.0 * 1024.0 * 1024.0,
            private_bytes: 180.0 * 1024.0 * 1024.0,
            cpu_percent: 12.5,
        },
    );
    let snap = ResourceSnapshot {
        timestamp_ms: 50_000,
        total_memory_bytes: 350.0 * 1024.0 * 1024.0,
        total_private_bytes: 300.0 * 1024.0 * 1024.0,
        total_cpu_percent: 15.0,
        app: vec![AppResourceEntry {
            kind: "main".to_string(),
            memory_bytes: 150.0 * 1024.0 * 1024.0,
            cpu_percent: 2.5,
        }],
        terminals: term_map,
    };
    let metas = vec![TerminalMeta {
        id: "t1".to_string(),
        workspace_id: "ws1".to_string(),
        workspace_name: "Tessel Rust".to_string(),
        name: "Codex".to_string(),
        is_agent: true,
    }];
    for sort in [
        ResourceSortBy::Memory,
        ResourceSortBy::Cpu,
        ResourceSortBy::Name,
    ] {
        let tree = resource_tree(Some(&snap), &metas, sort, SidebarLang::Fr).unwrap();
        assert_eq!(tree.terminal_count, 1);
        assert_eq!(tree.app_rows[0].label, "Principal");
    }

    let mut hist = ResourceHistory::new();
    assert_eq!(HISTORY_CAPACITY, 60);
    assert_eq!(HISTORY_STALE_MS, 600_000);
    assert_eq!(SPARK_SLOTS, 30);
    for i in 0..65u64 {
        let mut s = snap.clone();
        s.timestamp_ms = 1_000 + i * 2_000;
        hist.record_snapshot(&s, s.timestamp_ms);
    }
    assert_eq!(hist.series(APP_HISTORY_KEY).unwrap().memory.len(), 60);
    hist.prune(&HashSet::new(), 1_000 + 65 * 2_000 + HISTORY_STALE_MS + 1);
    assert!(hist.series("t1").is_none());
    assert!(hist.series(APP_HISTORY_KEY).is_some());

    let pts = sparkline_points(&[10.0, 20.0, 30.0], 48.0, 14.0);
    let (lx, ly) = sparkline_lead(&pts).unwrap();
    assert!((lx - 48.0).abs() < 0.2 && ly >= 1.5);

    let psum = ports_summary(
        &[
            ListeningPortEntry {
                port: 5173,
                workspace_key: Some("ws1::".to_string()),
                process_name: "node".to_string(),
                pid: 100,
            },
            ListeningPortEntry {
                port: 5432,
                workspace_key: None,
                process_name: "postgres".to_string(),
                pid: 200,
            },
        ],
        SidebarLang::Fr,
    );
    assert_eq!(psum.label, "1 port d'espace de travail · 1 externe");

    // Priority badge ladder from TerminalPane.vue:1680-1708
    let mut b = PaneBadgeInput {
        is_agent: true,
        asks_approval: true,
        asks_question_input: true,
        limit_reached: true,
        needs_you: true,
        ..PaneBadgeInput::default()
    };
    assert_eq!(pane_header_badge(&b), Some(PaneHeaderBadge::Question));
    assert_eq!(
        PaneHeaderBadge::Question.label(SidebarLang::Fr),
        "question ?"
    );
    b.asks_question_input = false;
    assert_eq!(pane_header_badge(&b), Some(PaneHeaderBadge::Approval));
    b.sleeping = true;
    assert_eq!(pane_header_badge(&b), Some(PaneHeaderBadge::Asleep));
    b.sleeping = false;
    b.exited = true;
    assert_eq!(pane_header_badge(&b), Some(PaneHeaderBadge::Exited));

    let guest = PaneBadgeInput {
        guest_runs: true,
        guest_state: Some("monitoring".to_string()),
        ..PaneBadgeInput::default()
    };
    assert_eq!(pane_header_badge(&guest), Some(PaneHeaderBadge::Monitoring));
}

#[test]
fn finder_scores_and_result_order_match_vue_for_unicode_and_path_boundaries() {
    // Outputs of src/shared/fuzzy.js in the Vue application.
    for (query, path, expected) in [
        ("app", "app.rs", 59.94),
        ("app", "src/App.rs", 50.90),
        ("app", "apps/App.rs", 53.89),
        (".env", ".env", 67.96),
        ("😀x", "src/😀X.txt", 41.89),
        ("i\u{307}", "İ.txt", 47.94),
        ("ος", "ΟΣ.txt", -1.),
        ("\u{feff} a", "a.txt", 38.95),
        ("a\u{85}b", "ab.rs", -1.),
        ("a\u{85}b", "a\u{85}b.rs", 59.94),
        ("ab", "a\u{feff}B.rs", 14.94),
        ("fb", "src/fastBuffer.js", 11.83),
        ("💾", "a💾.rs", 22.94),
    ] {
        assert!(
            (fuzzy_score(query, path) - expected).abs() < 1e-9,
            "{query:?}, {path:?}"
        );
    }
    let paths = [
        "src/App.rs",
        "app.rs",
        "apps/App.rs",
        "app-old.rs",
        "UI/app.rs",
    ]
    .map(String::from);
    assert_eq!(
        fuzzy_filter("app", &paths, 60),
        [
            "app.rs",
            "apps/App.rs",
            "UI/app.rs",
            "src/App.rs",
            "app-old.rs"
        ]
    );
    let tied = ["x/a.rs", "y/a.rs", "z/a.rs"].map(String::from);
    assert_eq!(fuzzy_filter("a", &tied, 2), tied[..2]);
}

#[test]
fn finder_orders_unicode_paths_and_preserves_ties_at_the_visible_cutoff() {
    let root = tempfile::tempdir().unwrap();
    let names = ["\u{e000}.rs", "\u{10000}.rs", "plain.rs"];
    for name in names {
        std::fs::write(root.path().join(name), "").unwrap();
    }
    assert_eq!(
        walk_project_files(root.path(), 10).unwrap().files,
        ["plain.rs", "\u{10000}.rs", "\u{e000}.rs"]
    );

    let mut paths: Vec<_> = (0..300).rev().map(|i| format!("{i:03}/a.rs")).collect();
    paths.push("a.rs".into());
    let mut expected = vec![String::from("a.rs")];
    expected.extend_from_slice(&paths[..FILE_FINDER_LIMIT - 1]);
    assert_eq!(fuzzy_filter("a", &paths, FILE_FINDER_LIMIT), expected);
}

#[test]
fn fuzzy_matcher_file_finder_and_bounded_walk_match_reference() {
    assert_eq!(FILE_FINDER_LIMIT, 60);
    assert_eq!(MAX_LISTED_FILES, 20_000);
    assert!(SKIP_DIRS.contains(&"target") && SKIP_DIRS.contains(&"node_modules"));

    assert_eq!(fuzzy_score("   ", "src/app.rs"), 0.0);
    assert!(fuzzy_score("zzz", "src/app.rs") < 0.0);
    // Exact stem & filename match beats scattered directory match.
    let exact = fuzzy_score("app", "src/app.rs");
    let scattered = fuzzy_score("app", "archive/panel_picker.rs");
    assert!(exact > scattered);

    let paths = vec![
        "archive/panel_picker.rs".to_string(),
        "src/app.rs".to_string(),
        "src/ui/AppHeader.rs".to_string(),
    ];
    let ranked = fuzzy_filter("app", &paths, 60);
    assert_eq!(ranked[0], "src/app.rs");
    assert_eq!(name_of("src/ui/AppHeader.rs"), "AppHeader.rs");
    assert_eq!(dir_of("src/ui/AppHeader.rs"), "src/ui");
    assert_eq!(dir_of("Cargo.toml"), "");

    let mut finder = FileFinderState::new(paths, false, true);
    assert!(!finder.truncated);
    assert_eq!(finder.files().len(), 3);
    assert_eq!(finder.results().len(), 3);
    assert_eq!(finder.recompute_count(), 1);
    assert_eq!(MAX_FINDER_QUERY_CHARS, 256);
    assert_eq!(clamp_finder_query(&"x".repeat(400)).chars().count(), 256);
    finder.set_query("app");
    assert_eq!(finder.query(), "app");
    assert_eq!(finder.recompute_count(), 2);
    // Setting identical query does NOT recompute cached results.
    finder.set_query("app");
    assert_eq!(finder.recompute_count(), 2);
    assert_eq!(
        finder.handle_key("Enter", false),
        FileFinderAction::OpenSelected("src/app.rs".to_string())
    );
    assert_eq!(
        finder.handle_key("Enter", true),
        FileFinderAction::InsertSelected("src/app.rs".to_string())
    );
    assert_eq!(
        finder.handle_key("ArrowDown", false),
        FileFinderAction::MoveSelection(1)
    );
    assert_eq!(finder.handle_key("Escape", false), FileFinderAction::Close);

    // Single-worker scan coordinator with cooperative cancellation (no unbounded spawns).
    let mut scan_coord = FinderScanCoordinator::new();
    assert!(!scan_coord.is_active());
    let (scan1, tok1) = scan_coord.request_scan().unwrap();
    assert!(scan_coord.is_active());
    assert!(!tok1.load(std::sync::atomic::Ordering::Relaxed));
    // Rapid second & third requests cancel `tok1` and coalesce into ONE follow-up scan.
    assert!(scan_coord.request_scan().is_none());
    assert!(scan_coord.request_scan().is_none());
    assert!(tok1.load(std::sync::atomic::Ordering::Relaxed));
    let (was_valid, next_scan) = scan_coord.finish_scan(scan1);
    assert!(!was_valid);
    let (scan2, _tok2) = next_scan.unwrap();
    scan_coord.cancel_active();
    let (was_valid2, none_after) = scan_coord.finish_scan(scan2);
    assert!(!was_valid2 && none_after.is_none());

    // Pure-Rust bounded directory walk in an isolated temp folder (zero subprocesses).
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("tessel_rust_finder_test_{nanos}"));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("target").join("debug")).unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
    fs::write(root.join("src").join("main.rs"), "fn main() {}\n").unwrap();
    fs::write(root.join("src").join("lib.rs"), "pub fn ok() {}\n").unwrap();
    fs::write(root.join("target").join("debug").join("ignored.bin"), "x").unwrap();
    fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();

    let full = walk_project_files(&root, 99_999).unwrap();
    assert!(!full.truncated);
    assert_eq!(full.stop_reason, None);
    assert!(full.total_path_bytes > 0);
    finder.set_files(full.clone());
    assert_eq!(finder.files().len(), 3);
    assert_eq!(
        full.files,
        vec![
            "Cargo.toml".to_string(),
            "src/lib.rs".to_string(),
            "src/main.rs".to_string(),
        ]
    );

    let capped = walk_project_files(&root, 2).unwrap();
    assert!(capped.truncated);
    assert_eq!(capped.stop_reason, Some(WalkStopReason::MaxFiles));
    assert_eq!(capped.files.len(), 2);

    // Verify default budgets & Windows junction/reparse + canonical scope helpers.
    assert_eq!(DEFAULT_MAX_VISITED_ENTRIES, 100_000);
    assert_eq!(DEFAULT_MAX_WALKED_DIRS, 10_000);
    assert_eq!(DEFAULT_MAX_ENTRIES_PER_DIR, 25_000);
    assert_eq!(DEFAULT_MAX_WALK_DEPTH, 32);
    assert_eq!(DEFAULT_MAX_TOTAL_PATH_BYTES, 8 * 1024 * 1024);
    assert_eq!(DEFAULT_MAX_SINGLE_PATH_BYTES, 1_024);
    assert_eq!(DEFAULT_WALK_TIMEOUT, Duration::from_secs(5));
    assert_eq!(WIN_FILE_ATTRIBUTE_REPARSE_POINT, 0x0400);
    assert!(has_reparse_point_attr(0x0410)); // FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT
    assert!(!has_reparse_point_attr(0x0010));

    let outside = std::env::temp_dir().join(format!("tessel_rust_outside_test_{nanos}"));
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.txt"), "top-secret\n").unwrap();
    let canon_root = fs::canonicalize(&root).unwrap();
    let canon_outside = fs::canonicalize(&outside).unwrap();
    assert!(is_within_canonical_scope(
        &canon_root,
        &canon_root.join("src")
    ));
    assert!(!is_within_canonical_scope(&canon_root, &canon_outside));

    #[cfg(windows)]
    {
        // If directory symlink privilege is available (Developer Mode), verify real FS escape is skipped.
        let link_path = root.join("linked_outside");
        if std::os::windows::fs::symlink_dir(&outside, &link_path).is_ok() {
            let walked = walk_project_files(&root, 50).unwrap();
            assert!(!walked.files.iter().any(|f| f.contains("secret.txt")));
            let _ = fs::remove_dir(&link_path);
        }
    }

    // Empty-directory flood budget (`max_dirs`), depth budget, entry budgets, timeout & cancellation.
    for idx in 0..8 {
        fs::create_dir_all(root.join(format!("empty_tree/d{idx}/nested"))).unwrap();
    }
    let by_dirs = walk_project_files_with_limits(
        &root,
        &WalkLimits {
            max_dirs: 3,
            ..WalkLimits::default()
        },
        None,
    )
    .unwrap();
    assert!(by_dirs.truncated);
    assert_eq!(by_dirs.stop_reason, Some(WalkStopReason::MaxDirs));

    let by_depth = walk_project_files_with_limits(
        &root,
        &WalkLimits {
            max_depth: 0,
            ..WalkLimits::default()
        },
        None,
    )
    .unwrap();
    assert!(by_depth.truncated);
    assert_eq!(by_depth.stop_reason, Some(WalkStopReason::MaxDepth));

    let by_visited = walk_project_files_with_limits(
        &root,
        &WalkLimits {
            max_visited_entries: 2,
            ..WalkLimits::default()
        },
        None,
    )
    .unwrap();
    assert!(by_visited.truncated);
    assert_eq!(
        by_visited.stop_reason,
        Some(WalkStopReason::MaxVisitedEntries)
    );

    let by_per_dir = walk_project_files_with_limits(
        &root,
        &WalkLimits {
            max_dir_entries: 1,
            ..WalkLimits::default()
        },
        None,
    )
    .unwrap();
    assert!(by_per_dir.truncated);
    assert_eq!(by_per_dir.stop_reason, Some(WalkStopReason::MaxDirEntries));

    let by_timeout = walk_project_files_with_limits(
        &root,
        &WalkLimits {
            timeout: Duration::ZERO,
            ..WalkLimits::default()
        },
        None,
    )
    .unwrap();
    assert!(by_timeout.truncated);
    assert_eq!(by_timeout.stop_reason, Some(WalkStopReason::Timeout));

    let cancel = AtomicBool::new(true);
    let by_cancel =
        walk_project_files_with_limits(&root, &WalkLimits::default(), Some(&cancel)).unwrap();
    assert!(by_cancel.truncated);
    assert_eq!(by_cancel.stop_reason, Some(WalkStopReason::Cancelled));

    let by_total_bytes = walk_project_files_with_limits(
        &root,
        &WalkLimits {
            max_total_path_bytes: 12,
            ..WalkLimits::default()
        },
        None,
    )
    .unwrap();
    assert!(by_total_bytes.truncated);
    assert_eq!(
        by_total_bytes.stop_reason,
        Some(WalkStopReason::MaxTotalPathBytes)
    );

    let by_single_bytes = walk_project_files_with_limits(
        &root,
        &WalkLimits {
            max_single_path_bytes: 5,
            ..WalkLimits::default()
        },
        None,
    )
    .unwrap();
    assert!(by_single_bytes.truncated);
    assert_eq!(
        by_single_bytes.stop_reason,
        Some(WalkStopReason::MaxSinglePathBytes)
    );

    let _ = fs::remove_dir_all(&outside);
    let _ = fs::remove_dir_all(&root);
    assert!(walk_project_files(&root, 10).is_err());
}

#[test]
fn project_launcher_sessions_and_launch_menu_match_reference() {
    assert_eq!(REMOTE_AGENT_IDS.len(), 17);
    assert_eq!(ALL_HISTORY_AGENTS.len(), 19);
    assert_eq!(SSH_DROPPED_EXIT_CODE, 255);
    assert_eq!(DROP_RETRIES, 3);
    assert_eq!(DROP_RETRY_WINDOW_MS, 300_000);
    assert_eq!(agent_display_name("claude"), "Claude Code");

    assert_eq!(
        valid_project_open("agent:codex"),
        ProjectOpenSetting::Agent("codex".to_string())
    );
    assert_eq!(valid_project_open("shell"), ProjectOpenSetting::Shell);
    assert_eq!(valid_project_open("browser"), ProjectOpenSetting::Browser);
    assert_eq!(valid_project_open("bogus"), ProjectOpenSetting::Empty);
    assert_eq!(emptied_workspace_choice(), None);

    let candidates = vec![
        AgentCandidate {
            id: "claude".to_string(),
            name: "Claude Code".to_string(),
            local_found: true,
            version: Some("1.0".to_string()),
            can_install: true,
            install_url: None,
        },
        AgentCandidate {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            local_found: false,
            version: None,
            can_install: true,
            install_url: None,
        },
    ];
    let mut host_map = HashMap::new();
    host_map.insert(
        "claude".to_string(),
        HostAgentProbe {
            found: Some(true),
            version: Some("1.2".to_string()),
            can_install: true,
            install_url: None,
        },
    );
    host_map.insert(
        "codex".to_string(),
        HostAgentProbe {
            found: Some(false),
            version: None,
            can_install: true,
            install_url: None,
        },
    );
    assert_eq!(
        remote_agent_found(Some(&host_map), "claude"),
        RemoteAgentStatus::Found
    );
    let remote_views = launcher_agents(&candidates, true, Some(&host_map));
    assert_eq!(remote_views.len(), 2);
    assert_eq!(
        launcher_shells(
            &[ShellCandidate {
                id: "pwsh".to_string(),
                name: "PowerShell".to_string()
            }],
            true
        )
        .len(),
        0
    );

    let open = project_open_choice("agent:claude", &candidates, Some("pwsh"), false, None);
    assert_eq!(open, Some(LauncherChoice::Agent("claude".to_string())));
    assert_eq!(remembered_value(open.as_ref()), "agent:claude");

    assert!(dropped_remote_pane(&RemoteDropCheck {
        is_remote: true,
        exit_code: Some(255),
        closing: false,
        sleeping: false,
        restarting: false,
    }));
    let mut attempts = Vec::new();
    assert!(allow_drop_retry(&mut attempts, 1_000));
    assert!(allow_drop_retry(&mut attempts, 2_000));
    assert!(allow_drop_retry(&mut attempts, 3_000));
    assert!(!allow_drop_retry(&mut attempts, 4_000));
    assert!(allow_drop_retry(&mut attempts, 400_000)); // window expired

    // Session history filtering & folder helpers
    assert_eq!(folder_key("C:\\Tessel-rust\\"), "c:/tessel-rust");
    assert!(in_folder("C:\\Tessel-rust\\src", "c:/tessel-rust"));
    assert_eq!(
        folder_label("C:\\work\\Tessel-rust\\src", LauncherLang::Fr),
        "Tessel-rust/src"
    );
    let sessions = vec![
        SessionRecord {
            id: "s1".to_string(),
            agent: "claude".to_string(),
            title: "Fix sidebar".to_string(),
            cwd: "C:\\Tessel-rust".to_string(),
            started_ms: 1_000,
            updated_ms: 5_000,
        },
        SessionRecord {
            id: "s2".to_string(),
            agent: "codex".to_string(),
            title: "Review grid".to_string(),
            cwd: "C:\\Tessel-rust\\src".to_string(),
            started_ms: 2_000,
            updated_ms: 9_000,
        },
    ];
    let ws_only = filter_sessions(
        &sessions,
        &SessionFilterOpts {
            query: "sidebar",
            agents: Some(&["claude"]),
            scope: SessionScope::Workspace,
            sort: SessionSort::Created,
            cwd: Some("C:\\Tessel-rust"),
        },
    );
    assert_eq!(ws_only.len(), 1);
    assert_eq!(ws_only[0].id, "s1");
    let recent = recent_project_sessions(&sessions, "C:\\Tessel-rust", 5);
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].id, "s2");

    let mut launcher =
        ProjectLauncherState::build(&remote_views, Some("pwsh".to_string()), &recent);
    assert!(!launcher.remember_for_new_projects);
    assert_eq!(ProjectLauncherState::shortcut_number(0), Some(1));
    assert_eq!(ProjectLauncherState::shortcut_number(9), None);
    // Item 2 (`'2'`) is missing remote Codex -> `InstallAgent`!
    assert_eq!(
        launcher.handle_key("2", false, false, false),
        ProjectLauncherAction::Launch(LauncherItem::InstallAgent {
            id: "codex".to_string(),
            name: "Codex".to_string(),
        })
    );
    assert_eq!(
        launcher.handle_key("Escape", false, false, false),
        ProjectLauncherAction::StayEmpty
    );

    // LaunchMenu placements, descriptions, clamping & navigation
    assert_eq!(LAUNCH_MENU_VIEWPORT_MARGIN, 6.0);
    assert_eq!(
        available_placements(true, true),
        vec![
            LaunchPlacement::Auto,
            LaunchPlacement::Left,
            LaunchPlacement::Right,
            LaunchPlacement::Down
        ]
    );
    assert_eq!(LaunchPlacement::Down.label(LauncherLang::Fr), "Dessous");
    assert_eq!(
        where_text(
            LaunchPlacement::Auto,
            true,
            true,
            Some("Pane 1"),
            LauncherLang::Fr
        ),
        "Rejoint la grille : les panneaux sont réorganisés"
    );
    assert_eq!(
        where_text(
            LaunchPlacement::Right,
            true,
            false,
            Some("Pane 1"),
            LauncherLang::En
        ),
        "Opens right of Pane 1"
    );
    assert_eq!(
        clamp_launch_menu_pos(990.0, 790.0, 240.0, 300.0, 1000.0, 800.0),
        (754.0, 494.0)
    );

    let mut menu = LaunchMenuState::new(
        vec![
            LaunchMenuItem::Shell {
                id: "pwsh".to_string(),
                name: "PowerShell".to_string(),
            },
            LaunchMenuItem::Agent {
                id: "claude".to_string(),
                name: "Claude Code".to_string(),
            },
            LaunchMenuItem::InstallAgent {
                id: "kimi".to_string(),
                name: "Kimi".to_string(),
            },
            LaunchMenuItem::ChatAgent {
                id: "claude".to_string(),
                name: "Claude Code (chat)".to_string(),
            },
            LaunchMenuItem::Browser,
        ],
        true,
        true,
        false,
    );
    assert!(!menu.host_menu);
    assert_eq!(
        menu.handle_key("ArrowUp"),
        LaunchMenuAction::MoveSelection(4)
    );
    assert_eq!(
        menu.handle_key("Enter"),
        LaunchMenuAction::Launch {
            item: LaunchMenuItem::Browser,
            placement: LaunchPlacement::Auto,
            in_worktree: false,
        }
    );
    assert_eq!(menu.handle_key("Escape"), LaunchMenuAction::Close);
}
