#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(windows)]
mod agent_browser;
mod agent_tools;
mod app;
#[cfg(windows)]
mod browser;
mod chat;
mod files;
mod git;
mod launch;
mod pdf;
mod preview;
mod remote;
mod remote_panel;
mod share;
mod share_panel;
mod terminal;
#[cfg(windows)]
mod vault;
#[cfg(windows)]
mod vault_panel;
mod workspace;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--pdf-worker") {
        std::process::exit(if pdf::worker().is_ok() { 0 } else { 1 });
    }
    if std::env::args().any(|a| a == "--self-test-pdf") {
        return pdf::smoke();
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
            viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(app::Tessel::new(cc)))),
    )?;
    Ok(())
}
