#[allow(dead_code)]
#[path = "../src/settings_model.rs"]
mod settings_model;
#[allow(dead_code)]
#[path = "../src/theme.rs"]
mod theme;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

#[allow(dead_code)]
mod workspace {
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct EditorPreferences {
        pub syntax_highlighting: bool,
        pub line_numbers: bool,
        pub word_wrap: bool,
        pub tab_size: u8,
        pub auto_save: bool,
        pub auto_save_delay_ms: u32,
        pub preview_tabs: bool,
        pub show_git_ignored: bool,
        pub compact_folders: bool,
        pub diff_word_wrap: bool,
    }

    impl Default for EditorPreferences {
        fn default() -> Self {
            Self {
                syntax_highlighting: true,
                line_numbers: true,
                word_wrap: false,
                tab_size: 4,
                auto_save: false,
                auto_save_delay_ms: 1000,
                preview_tabs: true,
                show_git_ignored: true,
                compact_folders: false,
                diff_word_wrap: false,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct GeneralPreferences {
        pub restore_workspaces: bool,
        pub confirm_close_agent: bool,
        pub confirm_quit_running: bool,
        pub workspace_dir: String,
        pub confirm_delete_workspace: bool,
    }

    impl Default for GeneralPreferences {
        fn default() -> Self {
            Self {
                restore_workspaces: true,
                confirm_close_agent: true,
                confirm_quit_running: true,
                workspace_dir: String::new(),
                confirm_delete_workspace: true,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum TerminalCursorStyle {
        #[default]
        Block,
        Bar,
        Underline,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct TerminalPreferences {
        pub scrollback: usize,
        pub line_height: f32,
        pub cursor_style: TerminalCursorStyle,
        pub cursor_blink: bool,
        pub tile_new_panes: bool,
        pub copy_on_select: bool,
        pub right_click_paste: bool,
        pub confirm_multiline_paste: bool,
        pub focus_follows_mouse: bool,
    }

    impl Default for TerminalPreferences {
        fn default() -> Self {
            Self {
                scrollback: 5000,
                line_height: 1.0,
                cursor_style: TerminalCursorStyle::Block,
                cursor_blink: true,
                tile_new_panes: true,
                copy_on_select: true,
                right_click_paste: true,
                confirm_multiline_paste: true,
                focus_follows_mouse: false,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum OpenLinksTarget {
        #[default]
        Builtin,
        System,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct BrowserPreferences {
        pub open_links_in: OpenLinksTarget,
        pub separate_agent_session: bool,
    }

    impl Default for BrowserPreferences {
        fn default() -> Self {
            Self {
                open_links_in: OpenLinksTarget::Builtin,
                separate_agent_session: true,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum BranchPrefixMode {
        GitUsername,
        #[default]
        Custom,
        None,
    }

    #[allow(clippy::enum_variant_names)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum ScmOrderSetting {
        #[default]
        ChangesFirst,
        StagedFirst,
        UntrackedFirst,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum ScmViewModeSetting {
        #[default]
        Tree,
        List,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct GitPreferences {
        pub branch_prefix_mode: BranchPrefixMode,
        pub branch_prefix_custom: String,
        pub scm_group_order: ScmOrderSetting,
        pub scm_view_mode: ScmViewModeSetting,
    }

    impl Default for GitPreferences {
        fn default() -> Self {
            Self {
                branch_prefix_mode: BranchPrefixMode::Custom,
                branch_prefix_custom: "agent".to_owned(),
                scm_group_order: ScmOrderSetting::ChangesFirst,
                scm_view_mode: ScmViewModeSetting::Tree,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct QuickCommand {
        pub id: u64,
        pub name: String,
        pub text: String,
        pub press_enter: bool,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum UsagePercentDisplay {
        #[default]
        Used,
        Remaining,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct StatsPreferences {
        pub usage_percentage_display: UsagePercentDisplay,
        pub usage_refresh_minutes: u16,
    }

    impl Default for StatsPreferences {
        fn default() -> Self {
            Self {
                usage_percentage_display: UsagePercentDisplay::Used,
                usage_refresh_minutes: 5,
            }
        }
    }
}

#[path = "../src/settings_pages_panel.rs"]
mod settings_pages_panel;

use eframe::egui::{self, Event, PointerButton, Pos2, Rect, Shape, Vec2};
use settings_pages_panel::{
    SettingsPageAction, SettingsPagesState, StatsSnapshot, preview_branch_name,
    resolve_branch_prefix, sanitize_git_username_prefix, validate_git_prefix_segment,
};
use theme_palettes::THEMES;
use workspace::{
    BranchPrefixMode, BrowserPreferences, EditorPreferences, GeneralPreferences, GitPreferences,
    OpenLinksTarget, QuickCommand, ScmOrderSetting, ScmViewModeSetting, StatsPreferences,
    TerminalCursorStyle, TerminalPreferences, UsagePercentDisplay,
};

fn collect_shape_texts(shape: &Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Text(t) => {
            out.push((
                t.galley.job.text.clone(),
                Rect::from_min_size(t.pos, t.galley.size()),
            ));
        }
        Shape::Vec(list) => {
            for inner in list {
                collect_shape_texts(inner, out);
            }
        }
        _ => {}
    }
}

fn painted_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Rect)> {
    let mut out = Vec::new();
    for cs in shapes {
        collect_shape_texts(&cs.shape, &mut out);
    }
    out
}

fn raw_input_with_click(click_pos: Option<Pos2>) -> egui::RawInput {
    let mut events = Vec::new();
    if let Some(pos) = click_pos {
        events.push(Event::PointerMoved(pos));
        events.push(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        events.push(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
    }
    egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 5200.0))),
        events,
        ..Default::default()
    }
}

fn find_label_center(texts: &[(String, Rect)], needle: &str) -> Pos2 {
    texts
        .iter()
        .find(|(text, _)| text == needle || text.contains(needle))
        .unwrap_or_else(|| panic!("expected rendered label containing {needle:?}; saw {texts:?}"))
        .1
        .center()
}

#[test]
fn all_eight_settings_pages_render_across_all_themes_in_english_and_french() {
    let snapshot = StatsSnapshot {
        project_count: 3,
        session_count: 5,
        running_agents: 2,
        sleeping_sessions: 1,
        open_documents: 4,
        tasks_todo: 2,
        tasks_doing: 1,
        tasks_review: 1,
        tasks_done: 6,
        app_rss_bytes: 128 * 1024 * 1024,
        panes_rss_bytes: 256 * 1024 * 1024,
        panes_cpu_pct: 12.5,
        listening_ports: 2,
    };
    let quick = vec![QuickCommand {
        id: 1,
        name: "Run tests".into(),
        text: "cargo test".into(),
        press_enter: true,
    }];

    for &theme_id in &THEMES {
        for french in [false, true] {
            let ctx = egui::Context::default();
            theme::apply(&ctx, true, Some(theme_id));
            let mut pages = SettingsPagesState::new();
            let mut general = GeneralPreferences::default();
            let mut terminal = TerminalPreferences::default();
            let mut editor = EditorPreferences::default();
            let mut browser = BrowserPreferences::default();
            let mut git = GitPreferences::default();
            let mut stats = StatsPreferences::default();

            let out = ctx.run(raw_input_with_click(None), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let _ = pages.show_general(ui, &mut general, "Ready", true, french);
                        let _ = pages.show_terminal(
                            ui,
                            &mut terminal,
                            Some("powershell"),
                            14.0,
                            true,
                            french,
                        );
                        let _ = pages.show_editor(ui, &mut editor, french);
                        let _ = pages.show_browser(ui, &mut browser, true, french);
                        let _ = pages.show_git(ui, &mut git, Some("Jean Claude"), french);
                        let _ = pages.show_quick_commands(ui, &quick, true, french);
                        let _ = pages.show_stats(ui, &mut stats, &snapshot, french);
                        let _ = pages.show_updates(ui, true, french);
                    });
                });
            });
            let texts = painted_texts(&out.shapes);
            assert!(
                texts.len() >= 40,
                "expected rich painted settings labels for {theme_id:?} (fr={french}), got {}",
                texts.len()
            );
            let joined: String = texts
                .iter()
                .map(|(t, _)| t.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if french {
                assert!(joined.contains("Démarrage et fermeture"));
                assert!(joined.contains("Commandes enregistrées"));
                assert!(joined.contains("Affichage de l'utilisation"));
                assert!(joined.contains("Licences open source"));
            } else {
                assert!(joined.contains("Startup & closing"));
                assert!(joined.contains("Saved commands"));
                assert!(joined.contains("Usage display"));
                assert!(joined.contains("Open-source licenses"));
            }
        }
    }
}

#[test]
fn general_page_reset_confirmation_and_diagnostics_emit_actions() {
    let ctx = egui::Context::default();
    theme::apply(&ctx, true, None);
    let mut pages = SettingsPagesState::new();
    let mut prefs = GeneralPreferences {
        workspace_dir: r"C:\Worktrees".into(),
        ..Default::default()
    };

    // Frame 1: locate "Reset preferences…" and "Copy diagnostics"
    let out1 = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_general(ui, &mut prefs, "", true, false);
        });
    });
    let texts1 = painted_texts(&out1.shapes);
    let diag_pos = find_label_center(&texts1, "Copy diagnostics");

    let mut action = None;
    let _ = ctx.run(raw_input_with_click(Some(diag_pos)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let (_, act) = pages.show_general(ui, &mut prefs, "", true, false);
            action = act;
        });
    });
    assert_eq!(action, Some(SettingsPageAction::CopyDiagnostics));

    // Click "Reset preferences…" -> opens inline confirmation without immediately resetting
    let out2 = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_general(ui, &mut prefs, "", true, false);
        });
    });
    let reset_pos = find_label_center(&painted_texts(&out2.shapes), "Reset preferences…");
    let mut action_step1 = None;
    let out3 = ctx.run(raw_input_with_click(Some(reset_pos)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let (_, act) = pages.show_general(ui, &mut prefs, "", true, false);
            action_step1 = act;
        });
    });
    assert_eq!(action_step1, None);

    // Calling reset_transient cancels the pending inline confirmation
    pages.reset_transient();
    let out_cancelled = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_general(ui, &mut prefs, "", true, false);
        });
    });
    let cancelled_texts = painted_texts(&out_cancelled.shapes);
    assert!(
        !cancelled_texts
            .iter()
            .any(|(t, _)| t.contains("Confirm reset")),
        "reset_transient should clear pending reset confirmation"
    );
    let reset_pos_again = find_label_center(&cancelled_texts, "Reset preferences…");
    let _ = ctx.run(raw_input_with_click(Some(reset_pos_again)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_general(ui, &mut prefs, "", true, false);
        });
    });

    // Re-render to see "Confirm reset" and click it
    let out4 = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_general(ui, &mut prefs, "", true, false);
        });
    });
    let confirm_pos = find_label_center(&painted_texts(&out4.shapes), "Confirm reset");
    let _ = out3;
    let mut action_step2 = None;
    let _ = ctx.run(raw_input_with_click(Some(confirm_pos)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let (_, act) = pages.show_general(ui, &mut prefs, "", true, false);
            action_step2 = act;
        });
    });
    assert_eq!(action_step2, Some(SettingsPageAction::ResetAllSettings));
}

#[test]
fn terminal_browser_git_stats_and_updates_controls_mutate_preferences_and_emit_actions() {
    let ctx = egui::Context::default();
    theme::apply(&ctx, true, None);
    let mut pages = SettingsPagesState::new();

    // 1. Terminal cursor style & shell selection
    let mut term = TerminalPreferences::default();
    let out_t = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_terminal(ui, &mut term, None, 13.0, true, false);
        });
    });
    let bar_pos = find_label_center(&painted_texts(&out_t.shapes), "Bar");
    let mut changed_t = false;
    let _ = ctx.run(raw_input_with_click(Some(bar_pos)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let (ch, _) = pages.show_terminal(ui, &mut term, None, 13.0, true, false);
            changed_t = ch;
        });
    });
    assert!(changed_t);
    assert_eq!(term.cursor_style, TerminalCursorStyle::Bar);

    let out_t2 = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_terminal(ui, &mut term, None, 13.0, true, false);
        });
    });
    let wsl_pos = find_label_center(&painted_texts(&out_t2.shapes), "WSL");
    let mut shell_action = None;
    let _ = ctx.run(raw_input_with_click(Some(wsl_pos)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let (_, act) = pages.show_terminal(ui, &mut term, None, 13.0, true, false);
            shell_action = act;
        });
    });
    assert_eq!(
        shell_action,
        Some(SettingsPageAction::SelectPreferredShell(Some("wsl".into())))
    );

    // 2. Editor tab size
    let mut ed = EditorPreferences::default();
    let out_e = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_editor(ui, &mut ed, false);
        });
    });
    let two_pos = painted_texts(&out_e.shapes)
        .into_iter()
        .find(|(t, _)| t == "2")
        .expect("tab size 2 option")
        .1
        .center();
    let mut changed_e = false;
    let _ = ctx.run(raw_input_with_click(Some(two_pos)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            changed_e = pages.show_editor(ui, &mut ed, false);
        });
    });
    assert!(changed_e);
    assert_eq!(ed.tab_size, 2);

    // 3. Browser link target + Passwords shortcut
    let mut br = BrowserPreferences::default();
    let out_b = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_browser(ui, &mut br, true, false);
        });
    });
    let sys_pos = find_label_center(&painted_texts(&out_b.shapes), "System browser");
    let mut changed_b = false;
    let _ = ctx.run(raw_input_with_click(Some(sys_pos)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let (ch, _) = pages.show_browser(ui, &mut br, true, false);
            changed_b = ch;
        });
    });
    assert!(changed_b);
    assert_eq!(br.open_links_in, OpenLinksTarget::System);

    let out_b2 = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_browser(ui, &mut br, true, false);
        });
    });
    let vault_pos = find_label_center(&painted_texts(&out_b2.shapes), "Open Passwords…");
    let mut vault_act = None;
    let _ = ctx.run(raw_input_with_click(Some(vault_pos)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let (_, act) = pages.show_browser(ui, &mut br, true, false);
            vault_act = act;
        });
    });
    assert_eq!(vault_act, Some(SettingsPageAction::OpenPasswordsVault));

    // 4. Git branch prefix helpers + SCM view mode
    assert_eq!(
        validate_git_prefix_segment("  feature/ui/ ").unwrap(),
        "feature/ui"
    );
    assert!(validate_git_prefix_segment("bad..ref").is_err());
    assert!(validate_git_prefix_segment("bad.lock").is_err());
    assert_eq!(
        sanitize_git_username_prefix("  Jean-Claude D. ").as_deref(),
        Some("jean-claude-d")
    );
    assert_eq!(
        resolve_branch_prefix(BranchPrefixMode::GitUsername, "agent", Some("Alice Smith"))
            .as_deref(),
        Some("alice-smith")
    );
    assert_eq!(
        preview_branch_name(BranchPrefixMode::None, "agent", Some("Alice"), "fix-auth"),
        "fix-auth"
    );

    let mut git = GitPreferences::default();
    let out_g = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_git(ui, &mut git, Some("Alice"), false);
        });
    });
    let list_pos = find_label_center(&painted_texts(&out_g.shapes), "Flat list");
    let mut changed_g = false;
    let _ = ctx.run(raw_input_with_click(Some(list_pos)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            changed_g = pages.show_git(ui, &mut git, Some("Alice"), false);
        });
    });
    assert!(changed_g);
    assert_eq!(git.scm_view_mode, ScmViewModeSetting::List);
    assert_eq!(git.scm_group_order, ScmOrderSetting::ChangesFirst);

    // 5. Stats usage percentage display + Updates notice file
    let mut st = StatsPreferences::default();
    let snap = StatsSnapshot::default();
    let out_s = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_stats(ui, &mut st, &snap, false);
        });
    });
    let rem_pos = find_label_center(&painted_texts(&out_s.shapes), "% remaining");
    let mut changed_s = false;
    let _ = ctx.run(raw_input_with_click(Some(rem_pos)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            changed_s = pages.show_stats(ui, &mut st, &snap, false);
        });
    });
    assert!(changed_s);
    assert_eq!(st.usage_percentage_display, UsagePercentDisplay::Remaining);

    let out_u = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_updates(ui, true, false);
        });
    });
    let lic_pos = find_label_center(&painted_texts(&out_u.shapes), "MIT License");
    let mut notice_act = None;
    let _ = ctx.run(raw_input_with_click(Some(lic_pos)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            notice_act = pages.show_updates(ui, true, false);
        });
    });
    assert_eq!(
        notice_act,
        Some(SettingsPageAction::OpenNoticeFile("LICENSE"))
    );
}

#[test]
fn quick_commands_run_and_confirmed_remove_work_in_english_and_french() {
    let ctx = egui::Context::default();
    theme::apply(&ctx, true, None);
    let mut pages = SettingsPagesState::new();
    let commands = vec![QuickCommand {
        id: 42,
        name: "Clippy strict".into(),
        text: "cargo clippy -- -D warnings".into(),
        press_enter: true,
    }];

    // Click "Exécuter" in French -> emits RunQuickCommand(42)
    let out1 = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_quick_commands(ui, &commands, true, true);
        });
    });
    let run_pos = find_label_center(&painted_texts(&out1.shapes), "Exécuter");
    let mut run_act = None;
    let _ = ctx.run(raw_input_with_click(Some(run_pos)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            run_act = pages.show_quick_commands(ui, &commands, true, true);
        });
    });
    assert_eq!(run_act, Some(SettingsPageAction::RunQuickCommand(42)));

    // Click "Supprimer" -> requires confirmation -> click "Confirmer" -> RemoveQuickCommand(42)
    let out2 = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_quick_commands(ui, &commands, true, true);
        });
    });
    let rem_pos = find_label_center(&painted_texts(&out2.shapes), "Supprimer");
    let mut rem_step1 = None;
    let _ = ctx.run(raw_input_with_click(Some(rem_pos)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            rem_step1 = pages.show_quick_commands(ui, &commands, true, true);
        });
    });
    assert_eq!(rem_step1, None);

    let out3 = ctx.run(raw_input_with_click(None), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pages.show_quick_commands(ui, &commands, true, true);
        });
    });
    let conf_pos = find_label_center(&painted_texts(&out3.shapes), "Confirmer");
    let mut rem_step2 = None;
    let _ = ctx.run(raw_input_with_click(Some(conf_pos)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            rem_step2 = pages.show_quick_commands(ui, &commands, true, true);
        });
    });
    assert_eq!(rem_step2, Some(SettingsPageAction::RemoveQuickCommand(42)));
}
