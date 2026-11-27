#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(windows)]
mod account;
#[cfg(windows)]
mod account_panel;
pub mod agent_arg_options;
mod agent_brand_assets;
pub mod agent_brand_icons;
#[cfg(windows)]
mod agent_browser;
pub mod agent_catalog;
mod agent_detection;
pub mod agent_env;
pub mod agent_installs;
pub mod agent_model_antigravity;
pub mod agent_model_cache;
pub mod agent_model_catalog;
pub mod agent_model_cursor;
pub mod agent_model_defaults;
pub mod agent_model_defaults_ui;
pub mod agent_model_discovery;
pub mod agent_model_launch;
pub mod agent_model_listed;
pub mod agent_model_options;
pub mod agent_model_probe;
pub mod agent_model_probe_parse;
pub mod agent_model_process;
pub mod agent_model_readers;
pub mod agent_names;
mod agent_paths;
pub mod agent_preferences;
pub mod agent_program;
pub mod agent_resume;
mod agent_tools;
pub mod agents_panel;
mod app;
pub mod appearance_panel;
pub mod automations;
mod backup;
#[cfg(windows)]
mod browser;
pub mod browser_clip;
pub mod browser_passwords_model;
pub mod browser_passwords_panel;
pub mod browser_popup;
mod chat;
mod chrome;
mod claude;
pub mod claude_model_file;
mod claude_terminal;
pub mod codex_sessions;
mod codex_terminal;
mod command_lookup;
mod command_palette_panel;
mod conversation_loader;
mod conversation_model;
pub mod cookie_import_model;
pub mod css_outline;
pub mod dev_update;
pub mod device_login;
pub mod dialogs_toolbar_model;
pub mod dialogs_toolbar_ui;
mod empty_state_panel;
pub mod explorer_scm_model;
pub mod explorer_tree;
mod file_actions;
mod file_timeline;
mod files;
pub mod finder_model;
mod finder_panel;
pub mod floating_terminal_panel;
pub mod floating_terminal_runtime;
mod git;
mod git_result;
mod gpui_pixels;
pub mod gpui_ui;
mod guest_file_editor;
mod guest_file_listing;
mod guest_git;
mod guest_task_board;
pub mod help_panel;
mod host_actions;
pub mod host_existing_terminals_panel;
#[cfg(windows)]
pub mod host_lease_store;
#[cfg(windows)]
pub mod host_opening_adapter;
pub mod host_remote_cache;
pub mod host_terminal_panel;
pub mod html_outline;
mod i18n;
pub mod icons;
pub mod json_outline;
mod json_outline_panel;
mod launch;
mod launch_fingerprint;
pub mod launcher_panel;
pub mod layout_broadcast_ui;
mod local_file_history;
pub mod muse_catalog;
pub mod muse_model_probe;
pub mod muse_protocol;
mod native_chat_options;
mod native_connection;
mod native_host_info;
pub mod native_host_install;
pub mod native_host_manager;
mod native_host_panel;
pub mod native_host_transport;
mod native_recycle;
mod native_review;
mod native_task_board;
mod office;
pub mod outline_tree;
pub mod palette;
pub mod pane_grid;
mod pane_header;
pub mod pane_menu_model;
mod pane_model_history;
pub mod pane_model_log;
pub mod pane_model_profile;
pub mod pane_model_runtime;
pub mod pane_model_settings;
pub mod pane_model_worker;
mod pane_numbers;
pub mod pane_session_model;
mod pdf;
mod preview;
#[cfg(windows)]
mod process_job;
pub mod project_panel;
pub mod prompt_before;
mod raster_preview;
mod regional_date;
mod saved_version_diff;
mod script_outline;
mod script_outline_worker;
#[cfg(all(windows, test))]
mod webview_test_support;
// Owner-local Linear UI; task-launch integration remains in progress.
mod external_issue;
pub mod initial_turn;
mod issues_menu;
mod linear_dialog;
#[allow(dead_code)]
mod linear_model;
mod new_task_form;
mod remote;
pub mod remote_agent_tools;
mod remote_panel;
mod remote_tree_watch;
pub mod rich_previews;
pub mod scm_changes_panel;
mod scm_drafts;
mod scm_state;
mod session_discovery;
pub mod session_option_picker_ui;
mod session_profile;
mod session_profile_identity;
mod session_profile_migration;
mod session_query;
pub mod settings_extra_model;
pub mod settings_extra_panel;
pub mod settings_model;
pub mod settings_nav;
pub mod settings_pages_panel;
mod settings_surface;
pub mod share;
pub mod share_panel;
mod shared_task_apply;
pub mod shortcuts;
pub mod side_panels_model;
pub mod side_panels_ui;
pub mod sidebar_model;
mod ssh_config;
pub mod ssh_hosts_panel;
mod ssh_reconnect;
pub mod ssh_tmux;
pub mod status_metrics;
pub mod status_strip;
pub mod syntax;
mod system_fonts;
mod system_identity;
mod task_actions;
pub mod task_agents_model;
pub mod task_card_timing;
mod task_completion;
mod task_decision;
mod task_decision_actions;
mod task_decision_panel;
mod task_dependencies;
mod task_history;
pub mod task_preparation;
pub mod task_prompt;
mod task_readiness;
mod task_report;
pub mod task_timeline;
pub mod tasks_board_panel;
mod team_board_api;
mod team_mcp_auth;
pub mod team_mcp_broker;
pub mod team_mcp_helper;
pub mod team_mcp_runtime;
#[cfg(debug_assertions)]
mod team_mcp_smoke;
pub mod team_mcp_stdio;
mod team_mcp_wire;
pub mod teams_relay;
pub mod teams_ui;
pub mod terminal;
pub mod terminal_exit;
mod terminal_layout;
mod terminal_model;
#[cfg(windows)]
pub mod tessel_sync;
mod theme;
pub mod theme_palettes;
pub mod toolbar_fit;
mod transcript_trash;
#[cfg(windows)]
mod vault;
#[cfg(windows)]
mod vault_panel;
#[cfg(windows)]
mod vault_worker;
mod view_tabs;
pub mod view_tabs_browser_model;
#[cfg(windows)]
mod window_frame;
mod workspace;
pub mod worktree;
pub mod worktree_cleanup_panel;
pub mod worktree_state;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--team-mcp-stdio") {
        let valid = std::env::args().count() == 2;
        let success = valid && team_mcp_helper::run_from_environment().is_ok();
        // A client's stderr pipe may also be full. Fatal helper failures use
        // only the exit status so no diagnostic write can defeat the deadline.
        std::process::exit(if success { 0 } else { 1 });
    }
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--self-test-team-mcp") {
        return team_mcp_smoke::run().map_err(Into::into);
    }
    if std::env::args().any(|arg| arg == "--version") {
        println!(
            "Tessel Rust {} {}",
            env!("CARGO_PKG_VERSION"),
            env!("TESSEL_BUILD")
        );
        return Ok(());
    }
    #[cfg(windows)]
    if std::env::args().nth(1).as_deref() == Some("--account-callback") {
        let raw = std::env::args().nth(2).unwrap_or_default();
        return account::receive_callback(&raw).map_err(Into::into);
    }
    if std::env::args().nth(1).as_deref() == Some("--script-outline-worker") {
        std::process::exit(if script_outline_worker::worker().is_ok() {
            0
        } else {
            1
        });
    }
    if std::env::args().nth(1).as_deref() == Some("--office-worker") {
        std::process::exit(if office::worker().is_ok() { 0 } else { 1 });
    }
    if std::env::args().nth(1).as_deref() == Some("--pdf-worker") {
        std::process::exit(if pdf::worker().is_ok() { 0 } else { 1 });
    }
    if std::env::args().any(|a| a == "--self-test-office") {
        return office::smoke();
    }
    if std::env::args().any(|a| a == "--self-test-pdf") {
        return pdf::smoke();
    }
    if std::env::args().any(|arg| arg == "--self-test-claude") {
        return claude::smoke(false);
    }
    if std::env::args().any(|arg| arg == "--self-test-claude-turn") {
        return claude::smoke(true);
    }
    if std::env::args().any(|arg| arg == "--self-test-codex") {
        return chat::smoke();
    }
    #[cfg(windows)]
    if std::env::args().any(|arg| arg == "--self-test-browser") {
        return browser::smoke::run();
    }
    #[cfg(windows)]
    if std::env::args().any(|arg| arg == "--self-test-agent-browser") {
        return browser::smoke::run_agent();
    }
    #[cfg(windows)]
    if std::env::args().any(|arg| arg == "--self-test-vault") {
        return browser::vault_smoke::run();
    }
    let args: Vec<String> = std::env::args().collect();
    let env_renderer = std::env::var("TESSEL_RENDERER").ok();
    let visual_test_fixture = std::env::var_os("TESSEL_SETTLED_SCREENSHOT").is_some()
        || args.iter().any(|a| a.ends_with("-fixture"));
    let saved_renderer = peek_saved_renderer(&args);
    let chosen_renderer = workspace::resolve_startup_renderer(
        &args,
        env_renderer.as_deref(),
        saved_renderer,
        visual_test_fixture,
    );

    if chosen_renderer == workspace::RendererPreference::AutoGpui {
        match gpui_ui::run_gpui_app() {
            Ok(()) => return Ok(()),
            Err(err) => {
                eprintln!(
                    "[Tessel] GPUI primary renderer unavailable ({err}); falling back to Compatibility (egui/OpenGL) renderer."
                );
            }
        }
    }

    run_compat_egui_app(&args)
}

fn peek_saved_renderer(args: &[String]) -> workspace::RendererPreference {
    #[derive(serde::Deserialize)]
    struct RendererPeek {
        #[serde(default)]
        renderer: workspace::RendererPreference,
    }

    let data_dir = args
        .windows(2)
        .find(|pair| pair[0] == "--data-dir")
        .map(|pair| std::path::PathBuf::from(&pair[1]))
        .or_else(|| {
            directories::ProjectDirs::from("com", "Tessel", "TesselRust")
                .map(|d| d.data_local_dir().to_owned())
        });

    data_dir
        .and_then(|dir| std::fs::read_to_string(dir.join("workspace.json")).ok())
        .and_then(|raw| serde_json::from_str::<RendererPeek>(&raw).ok())
        .map(|p| p.renderer)
        .unwrap_or_default()
}

fn run_compat_egui_app(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    eframe::run_native(
        "Tessel Rust",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size(
                    if cfg!(feature = "visual-test")
                        && args.iter().any(|a| a == "--compact-fixture")
                    {
                        [800.0, 520.0]
                    } else {
                        [1280.0, 800.0]
                    },
                )
                .with_min_inner_size([640.0, 420.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(app::Tessel::new(cc)))),
    )?;
    Ok(())
}

mod worktree_setup;
