#[allow(dead_code)]
#[path = "../src/pane_grid.rs"]
mod pane_grid;
#[path = "../src/pane_menu_model.rs"]
mod pane_menu_model;
#[path = "../src/view_tabs_browser_model.rs"]
mod view_tabs_browser_model;

use std::collections::HashMap;

use pane_grid::{EditorTabEntry, EditorTabStrip, ShownTab};
use pane_menu_model::{
    AccountRestartChoice, AgentStateFactInput, LiveShareTextSize, OtherPaneTarget,
    PaneActionsMenuInput, PaneAgentActivity, PaneFactKind, PaneMenuAction, PaneMenuEntry,
    PaneMenuLang, QuietFactInput, QuietLevel, TeamMembership, VoiceLanguageOption,
    build_pane_actions_menu, format_agent_state_fact, open_or_switch_chat_label,
};
use view_tabs_browser_model::{
    BLANK_URL, DIFF_SEP, DiffTabArea, DiffTabMeta, MAX_SAVED_EDITOR_TABS, MAX_SAVED_TAB_PATH_LEN,
    MinimalTextEdit, OpenTabLabelInput, SearchEngine, TabsLang, ViewTabKind, ViewTabLeafSource,
    allowed_browser_url, auto_save_delay_ms, base_tab_label, build_view_tabs, can_drag_view_tab,
    can_middle_close_view_tab, diff_tab_path, display_url, doc_path_of, editor_tab_label,
    file_name_of, host_of, is_diff_tab_path, is_local_url, looks_like_search, main_eol,
    minimal_edit, normalize_browser_input, open_tab_in_strip, path_key, same_path, search_url,
    valid_saved_files,
};

#[test]
fn editor_tabs_compose_with_pane_grid_strip_and_match_vscode_rules() {
    assert_eq!(DIFF_SEP, '\0');
    assert_eq!(MAX_SAVED_EDITOR_TABS, 50);
    assert_eq!(MAX_SAVED_TAB_PATH_LEN, 4_000);
    assert_eq!(TabsLang::default(), TabsLang::En);
    assert_eq!(path_key("C:/Repo/Src/App.rs\\"), "c:\\repo\\src\\app.rs");
    assert!(same_path("C:/Repo/Src/App.rs", "c:\\repo\\src\\app.rs"));
    assert_eq!(file_name_of("C:/Repo/Src/App.rs"), "App.rs");

    let mut strip = EditorTabStrip::default();
    assert_eq!(
        open_tab_in_strip(&mut strip, "C:/repo/a.rs", false, true),
        None
    );
    assert_eq!(
        open_tab_in_strip(&mut strip, "C:/repo/b.rs", false, true),
        None
    );
    // Focus `a.rs`, then open `preview1.rs`: inserts right after `a.rs` (index 1)!
    strip.active_path = Some("C:/repo/a.rs".to_string());
    assert_eq!(
        open_tab_in_strip(&mut strip, "C:/repo/preview1.rs", true, true),
        None
    );
    assert_eq!(strip.files[1].path, "C:/repo/preview1.rs");
    assert!(!strip.files[1].pinned);

    // Next single-click preview replaces `preview1.rs` in place!
    assert_eq!(
        open_tab_in_strip(&mut strip, "C:/repo/preview2.rs", true, true),
        Some("C:/repo/preview1.rs".to_string())
    );
    assert_eq!(strip.files[1].path, "C:/repo/preview2.rs");

    // Auto-save delay clamps within 250..=10_000 with 1_000 fallback.
    assert_eq!(auto_save_delay_ms(None, 250, 10_000, 1_000), 1_000);
    assert_eq!(auto_save_delay_ms(Some(50.0), 250, 10_000, 1_000), 250);
    assert_eq!(
        auto_save_delay_ms(Some(25_000.0), 250, 10_000, 1_000),
        10_000
    );

    // Saved layout validation rejects relative paths, diff tabs, and case-insensitive duplicates.
    let staged_key = diff_tab_path("C:/repo/a.rs", &DiffTabArea::Staged);
    assert!(is_diff_tab_path(&staged_key));
    let raw_saved = vec![
        EditorTabEntry {
            path: "relative/nope.rs".to_string(),
            pinned: true,
            dirty: false,
            deleted: false,
            failed: false,
        },
        EditorTabEntry {
            path: staged_key,
            pinned: true,
            dirty: false,
            deleted: false,
            failed: false,
        },
        EditorTabEntry {
            path: "C:/repo/Main.rs".to_string(),
            pinned: true,
            dirty: true,
            deleted: true,
            failed: true,
        },
        EditorTabEntry {
            path: "c:\\repo\\main.rs".to_string(),
            pinned: false,
            dirty: false,
            deleted: false,
            failed: false,
        },
    ];
    let cleaned = valid_saved_files(&raw_saved);
    assert_eq!(cleaned.len(), 1);
    assert_eq!(cleaned[0].path, "C:/repo/Main.rs");
    assert!(!cleaned[0].dirty);

    // `minimal_edit` never splits a `\r\n` CRLF pair or multi-byte UTF-8 character,
    // and never underflows when the entire `before` or `after` string is a shared suffix!
    assert_eq!(minimal_edit("same", "same"), None);
    let crlf_edit = minimal_edit("a\r\nb", "a\rc").expect("different");
    assert_eq!(
        crlf_edit,
        MinimalTextEdit {
            start_byte: 1,
            end_before_byte: 4,
            replacement: "\rc".to_string(),
        }
    );
    let utf8_edit = minimal_edit("éà", "éè").expect("utf8 edit");
    assert_eq!(utf8_edit.start_byte, "é".len());
    assert_eq!(utf8_edit.end_before_byte, "éà".len());
    assert_eq!(utf8_edit.replacement, "è");

    // Regression cases: insertion/deletion at byte 0, full Unicode suffix, and full CRLF suffix!
    assert_eq!(
        minimal_edit("x", "ax"),
        Some(MinimalTextEdit {
            start_byte: 0,
            end_before_byte: 0,
            replacement: "a".to_string(),
        })
    );
    assert_eq!(
        minimal_edit("ax", "x"),
        Some(MinimalTextEdit {
            start_byte: 0,
            end_before_byte: 1,
            replacement: String::new(),
        })
    );
    let corpus = [
        "",
        "x",
        "ax",
        "é",
        "aé",
        "😀",
        "\r",
        "\n",
        "\r\n",
        "a\r\n",
        "\r\nx",
        "😀\r\né",
    ];
    for &before in &corpus {
        for &after in &corpus {
            if before == after {
                assert_eq!(minimal_edit(before, after), None);
                continue;
            }
            let ed = minimal_edit(before, after).expect("distinct strings");
            assert!(ed.start_byte <= ed.end_before_byte);
            assert!(before.is_char_boundary(ed.start_byte));
            assert!(before.is_char_boundary(ed.end_before_byte));
            // Neither cut boundary in `before` may split a `\r\n` pair:
            let b = before.as_bytes();
            for cut in [ed.start_byte, ed.end_before_byte] {
                let split = cut
                    .checked_sub(1)
                    .is_some_and(|p| b.get(p) == Some(&b'\r') && b.get(cut) == Some(&b'\n'));
                assert!(!split, "cut {cut} split CRLF in {before:?} -> {after:?}");
            }
            let mut patched = String::new();
            patched.push_str(&before[..ed.start_byte]);
            patched.push_str(&ed.replacement);
            patched.push_str(&before[ed.end_before_byte..]);
            assert_eq!(patched, after);
        }
    }

    assert_eq!(main_eol("no newlines"), None);
    assert_eq!(main_eol("a\r\nb\r\nc\n"), Some("\r\n"));
    assert_eq!(main_eol("a\nb\nc\r\n"), Some("\n"));
}

#[test]
fn tab_labels_and_view_tabs_strip_match_reference() {
    let diff_branch = DiffTabMeta {
        full_path: "C:/repo/src/mod.rs".to_string(),
        area: DiffTabArea::Branch {
            base: "abcdef12345".to_string(),
            commit: "9876543210".to_string(),
        },
    };
    let diff_commit = DiffTabMeta {
        full_path: "C:/repo/tests/mod.rs".to_string(),
        area: DiffTabArea::Commit {
            commit: "1122334455".to_string(),
        },
    };
    let diff_local = DiffTabMeta {
        full_path: "C:/repo/a.rs".to_string(),
        area: DiffTabArea::Local {
            id: "snap1".to_string(),
            formatted_date: "2026/11/03 10:00".to_string(),
        },
    };
    let diff_agent = DiffTabMeta {
        full_path: "C:/repo/b.rs".to_string(),
        area: DiffTabArea::Agent {
            edit_key: "ed1".to_string(),
        },
    };
    let diff_unstaged = DiffTabMeta {
        full_path: "C:/repo/c.rs".to_string(),
        area: DiffTabArea::Unstaged,
    };

    let p_branch = diff_tab_path("C:/repo/src/mod.rs", &diff_branch.area);
    let p_commit = diff_tab_path("C:/repo/tests/mod.rs", &diff_commit.area);
    let p_local = diff_tab_path("C:/repo/a.rs", &diff_local.area);
    let p_agent = diff_tab_path("C:/repo/b.rs", &diff_agent.area);
    let p_unstaged = diff_tab_path("C:/repo/c.rs", &diff_unstaged.area);
    assert_eq!(
        doc_path_of(&p_branch, Some(&diff_branch)),
        "C:/repo/src/mod.rs"
    );

    // Twin diff tabs on `mod.rs` disambiguate with parent folder `mod.rs · src` / `mod.rs · tests`!
    let inputs = vec![
        OpenTabLabelInput {
            tab_path: &p_branch,
            diff: Some(&diff_branch),
        },
        OpenTabLabelInput {
            tab_path: &p_commit,
            diff: Some(&diff_commit),
        },
        OpenTabLabelInput {
            tab_path: &p_local,
            diff: Some(&diff_local),
        },
        OpenTabLabelInput {
            tab_path: &p_agent,
            diff: Some(&diff_agent),
        },
        OpenTabLabelInput {
            tab_path: &p_unstaged,
            diff: Some(&diff_unstaged),
        },
    ];
    assert_eq!(base_tab_label(&inputs[0], &inputs), "mod.rs · src");
    assert_eq!(
        editor_tab_label(&inputs[0], &inputs, TabsLang::Fr),
        "mod.rs · src (abcdef1..9876543)"
    );
    assert_eq!(
        editor_tab_label(&inputs[1], &inputs, TabsLang::En),
        "mod.rs · tests (1122334)"
    );
    assert_eq!(
        editor_tab_label(&inputs[2], &inputs, TabsLang::Fr),
        "a.rs (Fichier enregistré • 2026-11-03 10:00) ↔ a.rs"
    );
    assert_eq!(
        editor_tab_label(&inputs[3], &inputs, TabsLang::Fr),
        "b.rs (modification de l'agent)"
    );
    assert_eq!(
        editor_tab_label(&inputs[4], &inputs, TabsLang::En),
        "c.rs (diff)"
    );

    // Build full `ViewTabs` strip (`Terminal` always first + Editor + Browser + LiveSharePage).
    let mut strip = EditorTabStrip::default();
    strip.open_file("C:/repo/src/lib.rs", false); // preview
    strip.open_file("C:/repo/src/main.rs", true); // pinned
    strip.set_dirty("C:/repo/src/main.rs", true);

    let empty_strip = EditorTabStrip::default();
    let leaves = vec![
        ViewTabLeafSource::Editor {
            leaf_id: "ed-1",
            strip: &strip,
            diffs_by_tab_path: &[],
            live_share_host: None,
        },
        ViewTabLeafSource::Editor {
            leaf_id: "ed-empty",
            strip: &empty_strip,
            diffs_by_tab_path: &[],
            live_share_host: None,
        },
        ViewTabLeafSource::Browser {
            leaf_id: "br-1",
            title: "",
            url: "http://localhost:5173/dashboard",
            live_share_host: None,
        },
        ViewTabLeafSource::LiveSharePage {
            leaf_id: "ls-1",
            host: "Alice",
            page_title: "Vite App",
            page_url: "http://localhost:3000",
        },
    ];
    let shown = ShownTab::Leaf("ed-1".to_string());
    let tabs = build_view_tabs(&leaves, &shown, TabsLang::Fr);
    assert_eq!(tabs.len(), 6);
    assert_eq!(tabs[0].key, "terminal");
    assert_eq!(tabs[0].kind, ViewTabKind::Terminal);
    assert_eq!(tabs[0].id, "terminal");
    assert!(!tabs[0].active);
    assert!(!can_drag_view_tab(&tabs[0]));
    assert!(!can_middle_close_view_tab(&tabs[0]));

    // `lib.rs` is preview & inactive; `main.rs` is active & dirty!
    assert_eq!(tabs[1].kind, ViewTabKind::Editor);
    assert!(tabs[1].preview);
    assert!(tabs[1].hint.contains("Aperçu : double-cliquez"));
    assert_eq!(tabs[1].file_for_icon.as_deref(), Some("C:/repo/src/lib.rs"));
    assert!(tabs[2].active);
    assert!(tabs[2].dirty);
    assert_eq!(tabs[2].path.as_deref(), Some("C:/repo/src/main.rs"));
    assert!(!tabs[2].deleted);
    assert!(!tabs[2].failed);
    assert!(tabs[2].close_title.as_deref().unwrap().contains("Ctrl+W"));
    assert!(
        tabs[2]
            .close_aria_label
            .as_deref()
            .unwrap()
            .contains("modifications non enregistrées")
    );
    assert!(can_drag_view_tab(&tabs[2]));
    assert!(can_middle_close_view_tab(&tabs[2]));

    // Empty editor fallback + Browser host fallback + LiveSharePage host hint.
    assert_eq!(tabs[3].label, "Éditeur");
    assert_eq!(tabs[4].kind, ViewTabKind::Browser);
    assert_eq!(tabs[4].label, "localhost:5173");
    assert_eq!(tabs[5].label, "Vite App");
    assert!(tabs[5].host);
    assert!(tabs[5].hint.starts_with("Onglet de Alice :"));
    assert!(!can_drag_view_tab(&tabs[5])); // Live Share host tabs cannot be dragged
}

#[test]
fn browser_url_normalization_enforces_http_https_and_refuses_unsafe_schemes() {
    assert_eq!(BLANK_URL, "about:blank");
    assert_eq!(SearchEngine::default(), SearchEngine::Google);
    assert_eq!(
        SearchEngine::Google.prefix_url(),
        "https://www.google.com/search?q="
    );
    assert!(
        search_url("rust egui", SearchEngine::DuckDuckGo).contains("duckduckgo.com/?q=rust%20egui")
    );
    assert!(search_url("a+b", SearchEngine::Bing).contains("bing.com/search?q=a%2Bb"));

    // Unsafe schemes & embedded credentials are strictly refused!
    assert_eq!(
        allowed_browser_url("file:///C:/Windows/win.ini", false),
        None
    );
    assert_eq!(allowed_browser_url("javascript:alert(1)", false), None);
    assert_eq!(
        allowed_browser_url("data:text/html,<h1>x</h1>", false),
        None
    );
    assert_eq!(
        allowed_browser_url("https://user:secret@example.com/", false),
        None
    );
    // `blob:` scheme passes syntactic check ONLY when `allow_session_blob` is true (runtime enforces session scope in `src/browser.rs`).
    assert_eq!(
        allowed_browser_url("blob:https://example.com/123", false),
        None
    );
    assert!(allowed_browser_url("blob:https://example.com/123", true).is_some());

    // Address bar normalization (`normalizeBrowserInput`):
    assert_eq!(
        normalize_browser_input("  ", true, SearchEngine::Google).as_deref(),
        Some("about:blank")
    );
    assert_eq!(
        normalize_browser_input("localhost:5173/app", true, SearchEngine::Google).as_deref(),
        Some("http://localhost:5173/app")
    );
    assert_eq!(
        normalize_browser_input("127.0.0.1:3000", true, SearchEngine::Google).as_deref(),
        Some("http://127.0.0.1:3000/")
    );
    assert_eq!(
        normalize_browser_input("[::1]:8080/health", true, SearchEngine::Google).as_deref(),
        Some("http://[::1]:8080/health")
    );
    assert_eq!(
        normalize_browser_input("api.localhost:8443/v1", true, SearchEngine::Google).as_deref(),
        Some("http://api.localhost:8443/v1")
    );
    assert_eq!(
        normalize_browser_input("app.example.com:8443/x", true, SearchEngine::Google).as_deref(),
        Some("https://app.example.com:8443/x")
    );
    assert_eq!(
        normalize_browser_input("tesselcode.com/docs", true, SearchEngine::Google).as_deref(),
        Some("https://tesselcode.com/docs")
    );
    assert_eq!(
        normalize_browser_input("file:///etc/passwd", true, SearchEngine::Google),
        None
    );
    assert!(looks_like_search("egui layout grid"));
    assert_eq!(
        normalize_browser_input("egui layout", true, SearchEngine::Google).as_deref(),
        Some("https://www.google.com/search?q=egui%20layout")
    );

    assert!(is_local_url("http://localhost:5173/"));
    assert!(is_local_url("http://[::1]:8080/"));
    assert!(!is_local_url("https://tesselcode.com/"));
    assert_eq!(display_url("about:blank"), "");
    assert_eq!(host_of("https://example.com:8443/path"), "example.com:8443");
}

#[test]
fn pane_actions_menu_facts_order_and_capabilities_match_reference() {
    assert_eq!(PaneMenuLang::default(), PaneMenuLang::En);
    assert_eq!(
        format_agent_state_fact(
            &AgentStateFactInput {
                sleeping: false,
                exited: false,
                asks_approval: false,
                limit_reset: Some(Some("14:00")),
                activity: PaneAgentActivity::Idle,
                estimated_state: false,
                needs_you: false,
            },
            PaneMenuLang::Fr,
        ),
        "limite d'utilisation · réinitialisation 14:00"
    );
    assert_eq!(
        format_agent_state_fact(
            &AgentStateFactInput {
                sleeping: false,
                exited: false,
                asks_approval: false,
                limit_reset: None,
                activity: PaneAgentActivity::Unknown,
                estimated_state: false,
                needs_you: false,
            },
            PaneMenuLang::En,
        ),
        "unknown"
    );
    assert_eq!(
        open_or_switch_chat_label(true, false, false, PaneMenuLang::Fr),
        "Continuer dans un terminal"
    );
    assert_eq!(
        open_or_switch_chat_label(false, true, false, PaneMenuLang::En),
        "Switch to chat view"
    );

    let mut disabled = HashMap::new();
    disabled.insert(
        "menuResetTerminal".to_string(),
        "Unavailable in native chat".to_string(),
    );

    let input = PaneActionsMenuInput {
        pane_title: "Codex #1".to_string(),
        shell_name: "PowerShell 7".to_string(),
        worktree: Some(("feat/ui".to_string(), "C:/repo-wt".to_string())),
        team: Some(TeamMembership {
            id: "team-2".to_string(),
            name: "Équipe 2".to_string(),
            is_lead: true,
            can_set_lead: true,
        }),
        is_agent: true,
        is_chat: false,
        is_floating: false,
        is_maximized: true,
        can_arrange_panes: true,
        account_info: Some(("Work".to_string(), "Account: Work".to_string())),
        launch_yolo: true,
        sleeping: false,
        exited: false,
        asks_approval: false,
        limit_reset: None,
        activity: Some(PaneAgentActivity::Busy),
        estimated_state: true,
        needs_you: false,
        status_title: "Busy".to_string(),
        quiet: Some(QuietFactInput {
            minutes: 6,
            level: QuietLevel::Mild,
            reason: String::new(),
        }),
        prompt_cache: Some(("4m left".to_string(), "Warm".to_string())),
        unsent: true,
        launch_stale: true,
        has_selection: false,
        session_id: Some("12345678-aaaa-bbbb".to_string()),
        has_model_choice: false,
        model_text: Some("GPT-6 Astra".to_string()),
        model_title: Some("Model: GPT-6 Astra".to_string()),
        ls_text_size: Some((LiveShareTextSize::Fit, "Hopper".to_string())),
        voice_language_name: "Français".to_string(),
        voice_shortcut_label: None,
        voice_languages: vec![VoiceLanguageOption {
            tip: "fr-CA".to_string(),
            tag: "fr-CA".to_string(),
            name: "Français (Canada)".to_string(),
        }],
        selected_voice_tip: "fr-CA".to_string(),
        other_panes: vec![
            OtherPaneTarget {
                id: "p-shell".to_string(),
                title: "pwsh".to_string(),
                where_hint: "main".to_string(),
                is_agent: false,
            },
            OtherPaneTarget {
                id: "p-agent".to_string(),
                title: "Claude".to_string(),
                where_hint: "feat/ui".to_string(),
                is_agent: true,
            },
        ],
        account_restart_choices: vec![AccountRestartChoice {
            id: Some("acc-2".to_string()),
            name: "Personal".to_string(),
            title_hint: None,
        }],
        host_name: Some("dev-box".to_string()),
        can_sign_in_on_host: true,
        can_make_host_default: true,
        can_check_host_account: true,
        can_view_transcript: true,
        transcript_open: false,
        can_open_as_chat: true,
        chat_view_available: false,
        chat_view_open: false,
        can_continue_on_host: true,
        continue_on_host_reason: None,
        can_switch_yolo: true,
        yolo_folder: Some(("C:/Tessel-rust".to_string(), true)),
        disabled_reasons: disabled,
    };

    let menu = build_pane_actions_menu(&input, PaneMenuLang::Fr);
    assert_eq!(menu.title, "Codex #1");
    assert_eq!(menu.subtitle, "PowerShell 7");

    // All 8 facts present in exact order!
    let fact_kinds: Vec<PaneFactKind> = menu.facts.iter().map(|f| f.kind).collect();
    assert_eq!(
        fact_kinds,
        vec![
            PaneFactKind::Branch,
            PaneFactKind::Team,
            PaneFactKind::Account,
            PaneFactKind::StartedYolo,
            PaneFactKind::State,
            PaneFactKind::Quiet,
            PaneFactKind::PromptCache,
            PaneFactKind::ModelReadOnly,
        ]
    );
    assert_eq!(menu.facts[0].label, "Branche");
    assert_eq!(menu.facts[1].value, "Équipe 2 · chef");
    assert_eq!(menu.facts[4].value, "au travail (estimé)");
    assert!(!menu.facts[5].tooltip.is_empty());

    // Also exercise `QuietLevel::Long` and `ChooseModel` item branch.
    let mut input2 = input.clone();
    input2.has_model_choice = true;
    input2.quiet = Some(QuietFactInput {
        minutes: 25,
        level: QuietLevel::Long,
        reason: "Long tool".to_string(),
    });
    let menu2 = build_pane_actions_menu(&input2, PaneMenuLang::En);
    assert!(menu2.entries.iter().any(|e| matches!(
        e,
        PaneMenuEntry::Item {
            action: PaneMenuAction::ChooseModel,
            ..
        }
    )));

    // Verify key items, disabled state (`Copy` without selection, `ResetTerminal` with reason),
    // Live Share & Voice chip rows, `AskReview` filtered ONLY to agent panes, and danger `ClosePane` last!
    let mut send_targets = Vec::new();
    let mut ask_review_targets = Vec::new();
    let mut section_labels = Vec::new();
    let mut toggled_lead_team = None;
    let mut left_team = None;
    let mut restart_accounts = Vec::new();
    let mut reset_tooltip = None;
    let mut copy_enabled = true;
    let mut has_ls_row = false;
    let mut has_voice_row = false;
    for entry in &menu.entries {
        match entry {
            PaneMenuEntry::SectionLabel(sec) => section_labels.push(sec.clone()),
            PaneMenuEntry::Item {
                action: PaneMenuAction::Copy,
                enabled,
                ..
            } => copy_enabled = *enabled,
            PaneMenuEntry::Item {
                action: PaneMenuAction::ResetTerminal,
                tooltip,
                ..
            } => reset_tooltip = tooltip.clone(),
            PaneMenuEntry::Item {
                action: PaneMenuAction::CopySessionId,
                shortcut,
                ..
            } => assert_eq!(shortcut.as_deref(), Some("12345678")),
            PaneMenuEntry::Item {
                action: PaneMenuAction::SendSelectionTo(id),
                ..
            } => send_targets.push(id.clone()),
            PaneMenuEntry::Item {
                action: PaneMenuAction::AskReview(id),
                ..
            } => ask_review_targets.push(id.clone()),
            PaneMenuEntry::Item {
                action: PaneMenuAction::ToggleTeamLead(tid),
                ..
            } => toggled_lead_team = Some(tid.clone()),
            PaneMenuEntry::Item {
                action: PaneMenuAction::LeaveTeam(tid),
                ..
            } => left_team = Some(tid.clone()),
            PaneMenuEntry::Item {
                action: PaneMenuAction::RestartOnAccount(acc),
                ..
            } => restart_accounts.push(acc.clone()),
            PaneMenuEntry::Item {
                action: PaneMenuAction::ToggleYoloFolder,
                shortcut,
                ..
            } => assert_eq!(shortcut.as_deref(), Some("✓ Tessel-rust")),
            PaneMenuEntry::LiveShareTextSizeRow { label, chips } => {
                has_ls_row = true;
                assert_eq!(label, "Taille du texte");
                assert_eq!(chips.len(), 3);
                assert!(chips[0].selected && chips[0].id == "fit" && !chips[0].tooltip.is_empty());
            }
            PaneMenuEntry::VoiceLanguageRow { label, chips } => {
                has_voice_row = true;
                assert_eq!(label, "Parler en");
                assert_eq!(chips.len(), 2); // FR + ⌨
                assert_eq!(chips[0].label, "FR");
            }
            PaneMenuEntry::Separator | PaneMenuEntry::Item { .. } => {}
        }
    }
    assert!(!copy_enabled);
    assert_eq!(reset_tooltip.as_deref(), Some("Unavailable in native chat"));
    assert_eq!(section_labels.len(), 2);
    assert_eq!(
        send_targets,
        vec!["p-shell".to_string(), "p-agent".to_string()]
    );
    assert_eq!(ask_review_targets, vec!["p-agent".to_string()]);
    assert_eq!(toggled_lead_team.as_deref(), Some("team-2"));
    assert_eq!(left_team.as_deref(), Some("team-2"));
    assert_eq!(restart_accounts, vec![Some("acc-2".to_string())]);
    assert!(has_ls_row && has_voice_row);

    match menu.entries.last().expect("non-empty menu") {
        PaneMenuEntry::Item {
            action,
            label,
            danger,
            ..
        } => {
            assert_eq!(*action, PaneMenuAction::ClosePane);
            assert_eq!(label, "Fermer le panneau");
            assert!(*danger);
        }
        other => panic!("expected ClosePane item last, got {other:?}"),
    }

    // Floating terminal menu omits grid-only items (`ToggleMaximize`, `ArrangeGrid`, `SplitRight`, `SplitDown`).
    let floating = PaneActionsMenuInput {
        is_floating: true,
        ..PaneActionsMenuInput::default()
    };
    let floating_menu = build_pane_actions_menu(&floating, PaneMenuLang::En);
    assert!(!floating_menu.entries.iter().any(|e| matches!(
        e,
        PaneMenuEntry::Item {
            action: PaneMenuAction::SplitRight
                | PaneMenuAction::SplitDown
                | PaneMenuAction::ToggleMaximize
                | PaneMenuAction::ArrangeGrid,
            ..
        }
    )));
}
