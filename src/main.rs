#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(windows)]
mod account;
#[cfg(windows)]
mod account_panel;
#[cfg(windows)]
mod agent_browser;
mod agent_tools;
mod app;
mod backup;
#[cfg(windows)]
mod browser;
mod chat;
mod claude;
mod file_actions;
mod files;
pub mod finder_model;
mod finder_panel;
mod git;
mod i18n;
mod launch;
mod office;
pub mod palette;
pub mod pane_grid;
mod pdf;
mod preview;
mod remote;
mod remote_panel;
mod share;
mod share_panel;
mod terminal;
mod terminal_layout;
mod theme;
pub mod theme_palettes;
#[cfg(windows)]
mod vault;
#[cfg(windows)]
mod vault_panel;
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
            viewport: eframe::egui::ViewportBuilder::default().with_inner_size(
                if cfg!(feature = "visual-test")
                    && std::env::args().any(|a| a == "--compact-fixture")
                {
                    [800.0, 520.0]
                } else {
                    [1280.0, 800.0]
                },
            ),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(app::Tessel::new(cc)))),
    )?;
    Ok(())
}
