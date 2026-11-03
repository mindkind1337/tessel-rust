#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(windows)]
mod account;
#[cfg(windows)]
mod account_panel;
#[cfg(windows)]
mod agent_browser;
pub mod agent_catalog;
mod agent_detection;
pub mod agent_installs;
pub mod agent_names;
mod agent_paths;
mod agent_preferences;
mod agent_tools;
mod agents_panel;
mod app;
pub mod appearance_panel;
mod backup;
#[cfg(windows)]
mod browser;
mod chat;
mod chrome;
mod claude;
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
mod git;
mod git_result;
pub mod help_panel;
mod host_actions;
mod i18n;
pub mod icons;
mod launch;
pub mod launcher_panel;
mod office;
pub mod palette;
pub mod pane_grid;
mod pane_header;
pub mod pane_menu_model;
mod pdf;
mod preview;
pub mod project_panel;
mod remote;
mod remote_panel;
pub mod scm_changes_panel;
mod scm_state;
pub mod settings_model;
pub mod settings_nav;
mod settings_surface;
mod share;
mod share_panel;
pub mod shortcuts;
pub mod sidebar_model;
mod ssh_config;
pub mod ssh_hosts_panel;
mod status_strip;
mod system_fonts;
mod system_identity;
mod task_actions;
pub mod tasks_board_panel;
mod terminal;
mod terminal_layout;
mod theme;
pub mod theme_palettes;
#[cfg(windows)]
mod vault;
#[cfg(windows)]
mod vault_panel;
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
