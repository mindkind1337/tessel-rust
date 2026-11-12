use super::{
    editor_view::{layout_mermaid_flowchart, layout_mermaid_sequence},
    icons::GpuiIcon,
    theme::{GpuiThemeTokens, SPLIT_DIVIDER_PX, color_mix_srgb},
};
use crate::{
    app::{
        Tessel,
        gpui_bridge::{GpuiAction, GpuiCenterMode, GpuiFocusTarget},
    },
    rich_previews::{MermaidDiagram, parse_mermaid_diagram},
    syntax::SyntaxLang,
    terminal::{BroadcastInput, encode_gpui_terminal_key, extract_vt_styled_lines},
    theme_palettes::{THEMES, ThemeId},
    workspace::{KeepAwakeSetting, RendererPreference, SideTab, resolve_startup_renderer},
};
use serde_json::json;

#[test]
fn gpui_remote_files_routes_palette_commands_and_editable_fields_to_the_real_panel() {
    use crate::remote_panel::control::Command;
    let temp = tempfile::tempdir().unwrap();
    let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().to_path_buf()), false, || {});
    app.apply_gpui_action(GpuiAction::Remote(Command::Name("hidden".into())));
    assert!(app.gpui_snapshot(960., 640., 4.).remote.new_name.is_empty());
    app.apply_gpui_action(GpuiAction::PaletteExecute("remote".into()));
    assert_eq!(
        app.gpui_snapshot(960., 640., 4.).center_mode,
        GpuiCenterMode::RemoteSftp
    );
    assert!(app.gpui_handle_text_target_key(
        GpuiFocusTarget::RemoteName,
        "x",
        Some("x"),
        false,
        None
    ));
    assert_eq!(app.gpui_snapshot(960., 640., 4.).remote.new_name, "x");
    app.apply_gpui_action(GpuiAction::Remote(Command::Connect(usize::MAX)));
    let state = app.gpui_snapshot(960., 640., 4.);
    assert!(!state.remote.busy);
    assert!(state.remote.connected.is_none());
    app.apply_gpui_action(GpuiAction::SelectWorkTab);
    assert!(!app.gpui_handle_text_target_key(
        GpuiFocusTarget::RemoteName,
        "y",
        Some("y"),
        false,
        None
    ));
    assert_eq!(app.gpui_snapshot(960., 640., 4.).remote.new_name, "x");
}

#[test]
fn gpui_bridge_initializes_and_mutates_shared_workspace_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let proj_dir = temp.path().join("alpha-repo");
    std::fs::create_dir_all(&proj_dir).expect("create project dir");

    let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().to_path_buf()), false, || {});
    let snap0 = app.gpui_snapshot(960.0, 640.0, SPLIT_DIVIDER_PX);
    assert_eq!(snap0.center_mode, GpuiCenterMode::NoProjectsWelcome);
    assert!(snap0.projects.is_empty());
    assert_eq!(snap0.renderer, RendererPreference::AutoGpui);

    // Add a project folder and verify EmptyProjectLauncher mode + initials
    app.apply_gpui_action(GpuiAction::AddProjectPath(proj_dir.clone()));
    let snap1 = app.gpui_snapshot(960.0, 640.0, SPLIT_DIVIDER_PX);
    assert_eq!(snap1.projects.len(), 1);
    assert_eq!(snap1.projects[0].name, "alpha-repo");
    assert_eq!(snap1.projects[0].initials, "AR");
    assert!(snap1.projects[0].active);
    assert_eq!(snap1.center_mode, GpuiCenterMode::EmptyProjectLauncher);

    // Toggle sidebars, right fullscreen, and cycle all 7 SidePanel tabs
    let initial_left = snap1.toolbar.left_open;
    app.apply_gpui_action(GpuiAction::ToggleLeftSidebar);
    assert_eq!(
        app.gpui_snapshot(960.0, 640.0, 4.0).toolbar.left_open,
        !initial_left
    );

    for tab in [
        SideTab::Dashboard,
        SideTab::Files,
        SideTab::Changes,
        SideTab::Tasks,
        SideTab::TaskHistory,
        SideTab::History,
        SideTab::Conversations,
    ] {
        app.apply_gpui_action(GpuiAction::SelectSideTab(tab));
        let snap = app.gpui_snapshot(960.0, 640.0, 4.0);
        assert!(snap.toolbar.right_open);
        assert_eq!(snap.right_tab, tab);
    }

    // Switch themes, language, font zoom, keep-awake, and renderer preference
    for theme in THEMES {
        app.apply_gpui_action(GpuiAction::SetTheme(theme));
        assert_eq!(app.gpui_snapshot(960.0, 640.0, 4.0).theme, theme);
    }

    app.apply_gpui_action(GpuiAction::SetLanguage { french: true });
    assert!(app.gpui_snapshot(960.0, 640.0, 4.0).french);
    app.apply_gpui_action(GpuiAction::SetLanguage { french: false });
    assert!(!app.gpui_snapshot(960.0, 640.0, 4.0).french);

    let f0 = app.gpui_snapshot(960.0, 640.0, 4.0).font_size;
    app.apply_gpui_action(GpuiAction::ZoomFont(1));
    assert!(app.gpui_snapshot(960.0, 640.0, 4.0).font_size > f0);
    app.apply_gpui_action(GpuiAction::ZoomFont(0));
    assert!((app.gpui_snapshot(960.0, 640.0, 4.0).font_size - f0).abs() < f32::EPSILON);

    app.apply_gpui_action(GpuiAction::CycleKeepAwake);
    assert_ne!(
        app.gpui_snapshot(960.0, 640.0, 4.0).status_bar.keep_awake,
        KeepAwakeSetting::Off
    );

    app.apply_gpui_action(GpuiAction::SetRenderer(RendererPreference::CompatEgui));
    assert_eq!(
        app.gpui_snapshot(960.0, 640.0, 4.0).renderer,
        RendererPreference::CompatEgui
    );
}

#[test]
fn gpui_theme_tokens_and_svg_icons_cover_all_palettes_and_glyphs() {
    for theme in THEMES {
        let tokens = GpuiThemeTokens::from_theme(theme);
        assert_eq!(tokens.id, theme);
        assert!(tokens.radius_px >= 4.0);
        assert!((tokens.active_pane_ring.alpha - 0.70).abs() < 1e-5);
    }

    let mixed = color_mix_srgb(gpui::rgb(0xff0000), 0.25, gpui::rgb(0x0000ff));
    assert!((mixed.color.red - 0.25).abs() < 1e-4);
    assert!((mixed.color.blue - 0.75).abs() < 1e-4);

    assert_eq!(GpuiIcon::ALL.len(), 45);
    for icon in GpuiIcon::ALL {
        let svg_text = std::str::from_utf8(icon.svg_bytes()).expect("valid utf8 svg");
        assert!(svg_text.starts_with("<svg "));
        assert!(svg_text.ends_with("</svg>"));
    }

    let args = vec!["tessel".into(), "--renderer=compat".into()];
    assert_eq!(
        resolve_startup_renderer(&args, Some("gpui"), RendererPreference::AutoGpui, false),
        RendererPreference::CompatEgui
    );
}

#[test]
fn gpui_vt_styled_lines_and_key_encoder_preserve_ansi_unicode_and_modes() {
    let mut parser = vt100::Parser::new(6, 40, 100);
    parser.process(
        b"\x1b[1;38;2;255;128;64;48;2;10;20;30mRGB\x1b[0m \x1b[3;4mItalUnder\x1b[0m A\xf0\x9f\x99\x82B\r\nSecond line",
    );
    let palette = ThemeId::TokyoNight.spec().term;
    let lines = extract_vt_styled_lines(&parser, &palette, Some(((1, 0), (1, 6))));
    assert_eq!(lines.len(), 6);
    assert!(lines[0].iter().any(|s| {
        s.text == "RGB" && s.bold && s.fg == [255, 128, 64] && s.bg == Some([10, 20, 30])
    }));
    assert!(
        lines[0]
            .iter()
            .any(|s| s.text.contains("ItalUnder") && s.italic && s.underline)
    );
    let joined_row0: String = lines[0].iter().map(|s| s.text.as_str()).collect();
    assert!(joined_row0.contains("A🙂B"));
    assert!(lines[1].iter().any(|s| s.text == "Second" && s.selected));

    assert_eq!(
        encode_gpui_terminal_key("c", Some("c"), true, false, false, false, None),
        Some((vec![3], None))
    );
    assert_eq!(
        encode_gpui_terminal_key("up", None, false, false, false, false, None),
        Some((b"\x1b[A".to_vec(), Some(BroadcastInput::CursorKey(b'A'))))
    );
    assert_eq!(
        encode_gpui_terminal_key("up", None, false, false, false, true, None),
        Some((b"\x1bOA".to_vec(), Some(BroadcastInput::CursorKey(b'A'))))
    );
    assert_eq!(
        encode_gpui_terminal_key("tab", None, false, false, true, false, None),
        Some((b"\x1b[Z".to_vec(), None))
    );
    let (shift_enter, _) =
        encode_gpui_terminal_key("enter", None, false, false, true, false, Some("claude"))
            .expect("agent shift+enter");
    if cfg!(windows) {
        assert_eq!(shift_enter, b"\x1b\r");
    } else {
        assert_eq!(shift_enter, b"\n");
    }
    assert_eq!(
        encode_gpui_terminal_key("é", Some("Québec"), false, false, false, false, None),
        Some(("Québec".as_bytes().to_vec(), None))
    );
}

#[test]
fn gpui_native_chat_approvals_questions_and_mode_switch_roundtrip() {
    let temp = tempfile::tempdir().expect("tempdir");
    let proj_dir = temp.path().join("chat-repo");
    std::fs::create_dir_all(&proj_dir).expect("create project dir");

    let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().to_path_buf()), false, || {});
    app.apply_gpui_action(GpuiAction::AddProjectPath(proj_dir));
    app.apply_gpui_action(GpuiAction::LaunchClaudeChat);

    let snap = app.gpui_snapshot(960.0, 640.0, 4.0);
    assert_eq!(snap.tiles.len(), 1);
    let pane_id = snap.tiles[0].session_id;
    assert!(snap.tiles[0].is_native_chat);
    assert!(snap.tiles[0].can_switch_chat_mode);
    assert_eq!(
        snap.tiles[0].native_chat.as_ref().map(|c| c.provider),
        Some("claude")
    );

    // Type into composer via headless focus input handler
    assert!(app.gpui_handle_text_target_key(
        GpuiFocusTarget::ChatComposer(pane_id),
        "h",
        Some("Hello GPUI"),
        false,
        None,
    ));
    assert!(app.gpui_handle_text_target_key(
        GpuiFocusTarget::ChatComposer(pane_id),
        "enter",
        None,
        true,
        None,
    ));
    let snap2 = app.gpui_snapshot(960.0, 640.0, 4.0);
    assert_eq!(
        snap2.tiles[0]
            .native_chat
            .as_ref()
            .expect("chat")
            .composer
            .as_str(),
        "Hello GPUI\n"
    );

    // Inject structured AskUserQuestion + tool approval frames
    app.inject_claude_frame_for_test(
        pane_id,
        json!({
            "type": "control_request",
            "request_id": "req-q-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "AskUserQuestion",
                "input": {
                    "questions": [{
                        "header": "Renderer",
                        "question": "Which renderer?",
                        "multiSelect": false,
                        "options": [
                            {"label": "GPUI", "description": "Hardware GPU"},
                            {"label": "Compat", "description": "OpenGL"}
                        ]
                    }]
                }
            }
        }),
    );
    app.inject_claude_frame_for_test(
        pane_id,
        json!({
            "type": "control_request",
            "request_id": "req-tool-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "Bash",
                "input": {"command": "cargo test"}
            }
        }),
    );

    let snap3 = app.gpui_snapshot(960.0, 640.0, 4.0);
    let chat3 = snap3.tiles[0].native_chat.as_ref().expect("chat3");
    assert_eq!(chat3.questions.len(), 1);
    assert!(chat3.approval.is_some());
    assert_eq!(snap3.toolbar.waiting_agents_count, 1);

    app.apply_gpui_action(GpuiAction::ChatSelectQuestionOption {
        pane_id,
        q_idx: 0,
        opt_idx: 0,
    });
    app.apply_gpui_action(GpuiAction::ChatSubmitQuestions {
        pane_id,
        skip: false,
    });
    app.apply_gpui_action(GpuiAction::ChatRespondApproval {
        pane_id,
        allow: false,
    });
    let snap4 = app.gpui_snapshot(960.0, 640.0, 4.0);
    let chat4 = snap4.tiles[0].native_chat.as_ref().expect("chat4");
    assert!(chat4.questions.is_empty());
    assert!(chat4.approval.is_none());

    // Switch session mode in-place to CLI terminal and back to Native Chat
    app.apply_gpui_action(GpuiAction::SwitchSessionChatMode(pane_id));
    let snap5 = app.gpui_snapshot(960.0, 640.0, 4.0);
    assert_eq!(snap5.tiles[0].session_id, pane_id);
    assert!(!snap5.tiles[0].is_native_chat);
    assert!(snap5.tiles[0].can_switch_chat_mode);

    app.apply_gpui_action(GpuiAction::SwitchSessionChatMode(pane_id));
    let snap6 = app.gpui_snapshot(960.0, 640.0, 4.0);
    assert_eq!(snap6.tiles[0].session_id, pane_id);
    assert!(snap6.tiles[0].is_native_chat);
}

#[test]
fn gpui_document_editor_rich_previews_and_mermaid_layouts_work_end_to_end() {
    let temp = tempfile::tempdir().expect("tempdir");
    let proj_dir = temp.path().join("editor-repo");
    std::fs::create_dir_all(&proj_dir).expect("create project dir");

    let rs_path = proj_dir.join("lib.rs");
    std::fs::write(&rs_path, "pub fn answer() -> i32 {\n    41\n}\n").expect("write rs");
    let csv_path = proj_dir.join("table.csv");
    std::fs::write(&csv_path, "name,score\nAlice,98\nBob,95\n").expect("write csv");
    let jsonl_path = proj_dir.join("log.jsonl");
    std::fs::write(
        &jsonl_path,
        "{\"step\":1,\"ok\":true}\n{\"step\":2,\"ok\":false}\n",
    )
    .expect("write jsonl");

    let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().to_path_buf()), false, || {});
    app.apply_gpui_action(GpuiAction::AddProjectPath(proj_dir));

    // Open Rust source file, Find/Replace `41` -> `42`, and atomic Save
    app.apply_gpui_action(GpuiAction::OpenDocumentPath(rs_path.clone()));
    let snap = app.gpui_snapshot(960.0, 640.0, 4.0);
    assert_eq!(snap.center_mode, GpuiCenterMode::DocumentEditor);
    let doc = snap.active_document.expect("active doc");
    assert_eq!(doc.lang, SyntaxLang::Rust);
    assert!(!doc.dirty);

    app.apply_gpui_action(GpuiAction::ToggleEditorFind);
    app.apply_gpui_action(GpuiAction::SetEditorFindQuery("41".into()));
    app.apply_gpui_action(GpuiAction::SetEditorReplaceText("42".into()));
    app.apply_gpui_action(GpuiAction::ReplaceEditorAll);
    let snap_edited = app.gpui_snapshot(960.0, 640.0, 4.0);
    let doc_edited = snap_edited.active_document.expect("edited doc");
    assert!(doc_edited.dirty);
    assert!(doc_edited.text.contains("42"));
    assert!(!doc_edited.hunk_lines.is_empty());

    app.apply_gpui_action(GpuiAction::SaveActiveDocument);
    assert!(
        std::fs::read_to_string(&rs_path)
            .expect("read saved")
            .contains("42")
    );
    assert!(
        !app.gpui_snapshot(960.0, 640.0, 4.0)
            .active_document
            .expect("saved doc")
            .dirty
    );

    // Open CSV (auto-opens in SheetView preview) & JSONL (toggles into JsonDocumentPreview)
    app.apply_gpui_action(GpuiAction::OpenDocumentPath(csv_path));
    let csv_doc = app
        .gpui_snapshot(960.0, 640.0, 4.0)
        .active_document
        .expect("csv doc");
    assert!(csv_doc.preview_mode);
    assert_eq!(csv_doc.sheet.as_ref().map(|s| s.rows.len()), Some(3));
    app.apply_gpui_action(GpuiAction::ToggleActiveDocumentPreview);
    assert!(
        !app.gpui_snapshot(960.0, 640.0, 4.0)
            .active_document
            .expect("csv source doc")
            .preview_mode
    );

    app.apply_gpui_action(GpuiAction::OpenDocumentPath(jsonl_path));
    app.apply_gpui_action(GpuiAction::ToggleActiveDocumentPreview);
    let jsonl_doc = app
        .gpui_snapshot(960.0, 640.0, 4.0)
        .active_document
        .expect("jsonl doc");
    assert!(jsonl_doc.json_preview.is_some());

    // Deterministic Mermaid flowchart & sequence geometry
    if let Ok(MermaidDiagram::Flowchart {
        direction,
        nodes,
        edges,
    }) = parse_mermaid_diagram("flowchart LR\n  A[Client] --> B[GPUI]\n  B --> C[PTY]")
    {
        let flow = layout_mermaid_flowchart(direction, &nodes, &edges);
        assert_eq!(flow.nodes.len(), 3);
        assert_eq!(flow.edges.len(), 2);
        assert!(flow.width > 400.0 && flow.height >= 96.0);
    } else {
        panic!("expected Mermaid flowchart");
    }

    if let Ok(MermaidDiagram::Sequence {
        participants,
        messages,
    }) =
        parse_mermaid_diagram("sequenceDiagram\n  User->>Tessel: Ctrl+`\n  Tessel-->>PTY: spawn")
    {
        let seq = layout_mermaid_sequence(&participants, &messages);
        assert_eq!(seq.actor_centers_x.len(), 3);
        assert_eq!(seq.message_rows.len(), 2);
        assert!(seq.width >= 260.0 && seq.height >= 120.0);
    } else {
        panic!("expected Mermaid sequence diagram");
    }
}

#[test]
fn gpui_sidebar_and_seven_side_panels_roundtrip() {
    use crate::{
        project_panel::ProjectAction,
        side_panels_model::{
            ConversationWindow, DashboardGroupBy, DashboardState, SessionHistoryGroupBy,
            SessionHistoryScope, TaskHistoryFilter, TaskHistorySort,
        },
        tasks_board_panel::TasksBoardAction,
        workspace::SidebarPreferences,
    };

    let temp = tempfile::tempdir().expect("tempdir");
    let proj_dir = temp.path().join("panels-repo");
    std::fs::create_dir_all(proj_dir.join("src")).expect("create src");
    std::fs::write(
        proj_dir.join("src").join("main.rs"),
        "fn main() {\n    println!(\"needle_token\");\n}\n",
    )
    .expect("write main.rs");

    let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().to_path_buf()), false, || {});
    app.apply_gpui_action(GpuiAction::AddProjectPath(proj_dir));
    let pid = app.gpui_snapshot(960.0, 640.0, 4.0).projects[0].id;

    // Sidebar folder group assignment + collapse/expand + preferences
    app.apply_gpui_action(GpuiAction::Project(ProjectAction::EditProjectGroup(pid)));
    app.apply_gpui_action(GpuiAction::SetPromptDraft("Platform".into()));
    app.apply_gpui_action(GpuiAction::SubmitPromptModal);
    let snap_grp = app.gpui_snapshot(960.0, 640.0, 4.0);
    assert_eq!(snap_grp.projects[0].group_name.as_deref(), Some("Platform"));

    app.apply_gpui_action(GpuiAction::Project(ProjectAction::ToggleProjectGroup(
        "Platform".into(),
    )));
    assert!(
        app.gpui_snapshot(960.0, 640.0, 4.0)
            .sidebar
            .collapsed_groups
            .contains("Platform")
    );
    app.apply_gpui_action(GpuiAction::Project(ProjectAction::ToggleProjectGroup(
        "Platform".into(),
    )));

    app.apply_gpui_action(GpuiAction::SetSidebarPrefs(SidebarPreferences {
        compact_rows: true,
        hide_agent_task_line: true,
        ..Default::default()
    }));
    let snap_prefs = app.gpui_snapshot(960.0, 640.0, 4.0);
    assert!(snap_prefs.sidebar.prefs.compact_rows);
    assert!(snap_prefs.sidebar.prefs.hide_agent_task_line);

    // 1. Dashboard
    app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::Dashboard));
    app.apply_gpui_action(GpuiAction::SetDashboardGroup(DashboardGroupBy::Project));
    app.apply_gpui_action(GpuiAction::SetDashboardFilter(Some(
        DashboardState::Working,
    )));
    app.apply_gpui_action(GpuiAction::SetDashboardQuery("agent".into()));
    let dash = app.gpui_snapshot(960.0, 640.0, 4.0).side_panels;
    assert_eq!(dash.dashboard_group, DashboardGroupBy::Project);
    assert_eq!(dash.dashboard_filter, Some(DashboardState::Working));
    assert_eq!(dash.dashboard_query, "agent");

    // 2. Files (Explorer + async background content search)
    app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::Files));
    app.apply_gpui_action(GpuiAction::ExplorerSetFilter("needle_token".into()));
    app.apply_gpui_action(GpuiAction::ExplorerToggleSearchMode);
    let mut exp = app.gpui_snapshot(960.0, 640.0, 4.0).side_panels.explorer;
    for _ in 0..80 {
        if !exp.content_hits.is_empty() || !exp.loading {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        exp = app.gpui_snapshot(960.0, 640.0, 4.0).side_panels.explorer;
    }
    assert!(exp.content_search_mode);
    assert!(!exp.content_hits.is_empty());

    // 3. Changes (SCM + commits drawer toggle)
    app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::Changes));
    let initial_commits_open = app
        .gpui_snapshot(960.0, 640.0, 4.0)
        .side_panels
        .commits_open;
    app.apply_gpui_action(GpuiAction::ScmSetCommitMessage("feat: gpui scm".into()));
    app.apply_gpui_action(GpuiAction::ScmToggleCommitsOpen);
    let scm = app.gpui_snapshot(960.0, 640.0, 4.0).side_panels;
    assert_eq!(scm.scm_commit_msg, "feat: gpui scm");
    assert_eq!(scm.commits_open, !initial_commits_open);

    // 4. Tasks Kanban (Add -> Move -> Delete)
    app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::Tasks));
    app.apply_gpui_action(GpuiAction::TasksSetNewTitle("Ship Lot b176".into()));
    app.apply_gpui_action(GpuiAction::TasksAddFromDraft);
    let tasks1 = app.gpui_snapshot(960.0, 640.0, 4.0).side_panels.tasks;
    assert_eq!(tasks1.len(), 1);
    let tid = tasks1[0].id;
    assert_eq!(tasks1[0].column, 0);

    app.apply_gpui_action(GpuiAction::TasksBoard(TasksBoardAction::Move {
        id: tid,
        column: 3,
    }));
    assert_eq!(
        app.gpui_snapshot(960.0, 640.0, 4.0).side_panels.tasks[0].column,
        3
    );

    app.apply_gpui_action(GpuiAction::TasksBoard(TasksBoardAction::Delete(tid)));
    assert!(
        app.gpui_snapshot(960.0, 640.0, 4.0)
            .side_panels
            .tasks
            .is_empty()
    );

    // 5. TaskHistory, 6. History, 7. Conversations
    app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::TaskHistory));
    app.apply_gpui_action(GpuiAction::SetTaskHistoryWindow(TaskHistoryFilter::Days7));
    app.apply_gpui_action(GpuiAction::SetTaskHistorySort(TaskHistorySort::Time));
    app.apply_gpui_action(GpuiAction::SetTaskHistoryQuery("build".into()));

    app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::History));
    app.apply_gpui_action(GpuiAction::SetSessionsScope(SessionHistoryScope::All));
    app.apply_gpui_action(GpuiAction::SetSessionsGroup(SessionHistoryGroupBy::Agent));
    app.apply_gpui_action(GpuiAction::SetSessionsQuery("claude".into()));

    app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::Conversations));
    app.apply_gpui_action(GpuiAction::SetConversationsWindow(
        ConversationWindow::Days30,
    ));
    app.apply_gpui_action(GpuiAction::SetConversationsQuery("relay".into()));

    let sp_final = app.gpui_snapshot(960.0, 640.0, 4.0).side_panels;
    assert_eq!(sp_final.history_window, TaskHistoryFilter::Days7);
    assert_eq!(sp_final.history_sort, TaskHistorySort::Time);
    assert_eq!(sp_final.sessions_scope, SessionHistoryScope::All);
    assert_eq!(sp_final.conv_window, ConversationWindow::Days30);
    assert_eq!(sp_final.conv_query, "relay");
}

#[test]
fn gpui_settings_twenty_two_pages_and_preferences_roundtrip() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().to_path_buf()), false, || {});

    app.apply_gpui_action(GpuiAction::OpenSettings);
    let initial_settings = app
        .gpui_snapshot(960.0, 640.0, 4.0)
        .settings
        .expect("settings open");
    let all_page_ids: Vec<String> = initial_settings
        .groups
        .iter()
        .flat_map(|(_, pages)| pages.iter().map(|(id, _)| id.clone()))
        .collect();
    assert_eq!(all_page_ids.len(), 22);

    for page_id in &all_page_ids {
        app.apply_gpui_action(GpuiAction::OpenSettingsPage(page_id.clone()));
        let cur = app
            .gpui_snapshot(960.0, 640.0, 4.0)
            .settings
            .expect("settings page");
        assert_eq!(&cur.active_page_id, page_id);
    }

    // Search filtering + Escape query clear before closing modal
    app.apply_gpui_action(GpuiAction::SetSettingsSearch("git".into()));
    let filtered = app
        .gpui_snapshot(960.0, 640.0, 4.0)
        .settings
        .expect("filtered settings");
    let filtered_count: usize = filtered.groups.iter().map(|(_, p)| p.len()).sum();
    assert!((1..22).contains(&filtered_count));
    assert!(app.gpui_escape_clears_modal_search());
    assert!(
        app.gpui_snapshot(960.0, 640.0, 4.0)
            .settings
            .expect("cleared search")
            .search_query
            .is_empty()
    );

    // Quick commands, SSH hosts, and Custom CLI agent creation
    app.apply_gpui_action(GpuiAction::SetQuickCommandDraft {
        name: "Cargo Check".into(),
        text: "cargo check".into(),
        press_enter: true,
    });
    app.apply_gpui_action(GpuiAction::SubmitNewQuickCommand);

    app.apply_gpui_action(GpuiAction::SetSshDraft {
        alias: "prod-box".into(),
        folder: "/srv/app".into(),
    });
    app.apply_gpui_action(GpuiAction::AddSshHostFromDraft);

    app.apply_gpui_action(GpuiAction::SetCustomAgentDraft {
        name: "My Bot".into(),
        cmd: "my-bot --chat".into(),
    });
    app.apply_gpui_action(GpuiAction::SubmitNewCustomAgent);

    let updated = app
        .gpui_snapshot(960.0, 640.0, 4.0)
        .settings
        .expect("updated settings");
    assert_eq!(updated.quick_commands.len(), 1);
    assert_eq!(updated.quick_commands[0].name, "Cargo Check");
    assert_eq!(updated.ssh_hosts.len(), 1);
    assert_eq!(updated.ssh_hosts[0].alias, "prod-box");
    assert_eq!(updated.custom_agents.len(), 1);
    assert_eq!(updated.custom_agents[0].name, "My Bot");

    #[cfg(windows)]
    {
        app.apply_gpui_action(GpuiAction::OpenSettingsPage("passwords".into()));
        let pw = app
            .gpui_snapshot(960.0, 640.0, 4.0)
            .settings
            .expect("passwords page");
        assert_eq!(pw.active_page_id, "passwords");
        assert!(pw.vault.is_some());
    }

    app.apply_gpui_action(GpuiAction::CloseOverlays);
    assert!(app.gpui_snapshot(960.0, 640.0, 4.0).settings.is_none());
}

#[test]
fn gpui_modals_palette_finder_launch_notes_teams_and_popovers_roundtrip() {
    use crate::{
        dialogs_toolbar_ui::AddProjectTab, finder_model::LaunchPlacement,
        side_panels_ui::SidePanelsAction, status_strip::StatusBarItemId, teams_ui::TeamsUiAction,
    };

    let temp = tempfile::tempdir().expect("tempdir");
    let proj_dir = temp.path().join("modals-repo");
    std::fs::create_dir_all(&proj_dir).expect("create modals-repo");
    std::fs::write(proj_dir.join("hello.txt"), "hello world\n").expect("write hello.txt");

    let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().to_path_buf()), false, || {});
    app.apply_gpui_action(GpuiAction::AddProjectPath(proj_dir.clone()));
    let pid = app.gpui_snapshot(960.0, 640.0, 4.0).projects[0].id;

    // Command Palette
    app.apply_gpui_action(GpuiAction::ToggleCommandPalette);
    assert!(app.gpui_is_palette_open());
    app.apply_gpui_action(GpuiAction::PaletteSetQuery("theme".into()));
    app.apply_gpui_action(GpuiAction::PaletteStep(1));
    assert!(
        !app.gpui_snapshot(960.0, 640.0, 4.0)
            .modals
            .palette_rows
            .is_empty()
    );
    app.apply_gpui_action(GpuiAction::CloseOverlays);
    assert!(!app.gpui_is_palette_open());

    // Launch Menu
    app.apply_gpui_action(GpuiAction::ToggleLaunchMenu);
    app.apply_gpui_action(GpuiAction::SetLaunchPlacement(LaunchPlacement::Right));
    app.apply_gpui_action(GpuiAction::ToggleLaunchInWorktree);
    let m_launch = app.gpui_snapshot(960.0, 640.0, 4.0).modals;
    assert!(m_launch.launch_menu_open);
    assert_eq!(m_launch.launch_placement, LaunchPlacement::Right);
    assert!(m_launch.launch_in_worktree);
    app.apply_gpui_action(GpuiAction::CloseOverlays);

    // Help Dialog + AddProjectDialog clone URL auto-folder derivation
    app.apply_gpui_action(GpuiAction::OpenHelp);
    app.apply_gpui_action(GpuiAction::SetHelpQuery("palette".into()));
    assert!(
        !app.gpui_snapshot(960.0, 640.0, 4.0)
            .modals
            .help_groups
            .is_empty()
    );
    app.apply_gpui_action(GpuiAction::CloseOverlays);

    app.apply_gpui_action(GpuiAction::OpenAddProjectDialog);
    app.apply_gpui_action(GpuiAction::SetAddProjectTab(AddProjectTab::CloneRepo));
    app.apply_gpui_action(GpuiAction::SetAddProjectCloneUrl(
        "https://github.com/acme/super-widget.git".into(),
    ));
    assert_eq!(
        app.gpui_snapshot(960.0, 640.0, 4.0).modals.add_folder_name,
        "super-widget"
    );
    app.apply_gpui_action(GpuiAction::CloseOverlays);

    // Project Notes `.tessel/notes.md` modal
    app.apply_gpui_action(GpuiAction::OpenNotesModal);
    app.apply_gpui_action(GpuiAction::SetNotesText("# Sprint Notes\nAll green".into()));
    assert!(app.gpui_snapshot(960.0, 640.0, 4.0).modals.notes_dirty);
    app.apply_gpui_action(GpuiAction::SidePanels(SidePanelsAction::SaveNotes {
        force: true,
    }));
    assert!(
        std::fs::read_to_string(proj_dir.join(".tessel").join("notes.md"))
            .expect("notes.md")
            .contains("All green")
    );
    app.apply_gpui_action(GpuiAction::CloseOverlays);

    // Teams Manager + Toolbar & StatusBar popovers
    app.apply_gpui_action(GpuiAction::OpenTeamsManager(pid));
    app.apply_gpui_action(GpuiAction::Teams(TeamsUiAction::CreateTeam {
        project: pid,
        name: "Team 2".into(),
    }));
    assert_eq!(
        app.gpui_snapshot(960.0, 640.0, 4.0).modals.teams_list.len(),
        1
    );
    app.apply_gpui_action(GpuiAction::CloseTeamsManager);

    app.apply_gpui_action(GpuiAction::ToggleUsagePopover);
    assert!(
        app.gpui_snapshot(960.0, 640.0, 4.0)
            .modals
            .usage_popover_open
    );
    app.apply_gpui_action(GpuiAction::ToggleNotificationsPopover);
    let m_notif = app.gpui_snapshot(960.0, 640.0, 4.0).modals;
    assert!(m_notif.notifications_popover_open);
    assert!(!m_notif.usage_popover_open);

    app.apply_gpui_action(GpuiAction::ToggleStatusPopover(StatusBarItemId::Resources));
    assert_eq!(
        app.gpui_snapshot(960.0, 640.0, 4.0).modals.status_popover,
        Some(StatusBarItemId::Resources)
    );
    app.apply_gpui_action(GpuiAction::CloseOverlays);
    assert!(!app.gpui_has_open_modal());
}
