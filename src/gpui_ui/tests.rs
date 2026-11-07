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

    assert_eq!(GpuiIcon::ALL.len(), 35);
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
