#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(windows)]
mod account;
#[cfg(windows)]
mod account_panel;
pub mod agent_arg_options;
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
mod agents_panel;
mod app;
pub mod appearance_panel;
mod backup;
#[cfg(windows)]
mod browser;
pub mod browser_clip;
pub mod browser_passwords_model;
pub mod browser_passwords_panel;
mod chat;
mod chrome;
mod claude;
pub mod claude_model_file;
mod claude_terminal;
pub mod codex_sessions;
mod codex_terminal;
#[cfg(any(windows, test))]
mod command_lookup;
mod command_palette_panel;
mod empty_state_panel;
pub mod explorer_scm_model;
pub mod explorer_tree;
mod file_actions;
mod files;
pub mod finder_model;
mod finder_panel;
pub mod floating_terminal_panel;
pub mod floating_terminal_runtime;
mod git;
mod git_result;
pub mod help_panel;
mod host_actions;
pub mod host_existing_terminals_panel;
#[cfg(windows)]
pub mod host_lease_store;
#[cfg(windows)]
pub mod host_opening_adapter;
pub mod host_terminal_panel;
mod i18n;
pub mod icons;
mod launch;
mod launch_fingerprint;
pub mod launcher_panel;
pub mod layout_broadcast_ui;
pub mod muse_catalog;
pub mod muse_model_probe;
pub mod muse_protocol;
mod native_chat_options;
mod native_host_info;
pub mod native_host_install;
pub mod native_host_manager;
mod native_host_panel;
pub mod native_host_transport;
mod office;
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
pub mod pane_session_model;
mod pdf;
mod preview;
pub mod project_panel;
mod regional_date;
mod remote;
mod remote_panel;
pub mod scm_changes_panel;
mod scm_state;
pub mod session_option_picker_ui;
pub mod settings_model;
pub mod settings_nav;
mod settings_surface;
pub mod share;
pub mod share_panel;
pub mod shortcuts;
pub mod sidebar_model;
mod ssh_config;
pub mod ssh_hosts_panel;
pub mod status_metrics;
pub mod status_strip;
mod system_fonts;
mod system_identity;
mod task_actions;
pub mod tasks_board_panel;
pub mod terminal;
mod terminal_layout;
mod terminal_model;
mod theme;
pub mod theme_palettes;
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
fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    eframe::run_native(
        "Tessel Rust",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size(
                    if cfg!(feature = "visual-test")
                        && std::env::args().any(|a| a == "--compact-fixture")
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
