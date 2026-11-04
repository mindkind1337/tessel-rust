//! Owner-local controls for the staged native host integration.
use crate::{
    i18n::tr,
    native_host_manager::{Failure, Manager},
    workspace::Host,
};
use eframe::egui;
use tessel_host_client::ConnectionState;

#[derive(Default)]
pub struct Panel {
    selected: String,
    error: Option<Failure>,
    initially_open: bool,
}

pub enum Action {
    NewTerminal(String),
    ExistingTerminals(String),
}

fn failure(error: Failure) -> &'static str {
    match error {
        Failure::InvalidHost => "Invalid SSH host",
        Failure::Capacity => "Close a native host connection before opening another.",
        Failure::Cancelled => "Connection cancelled",
        Failure::Worker => "The connection worker stopped.",
        Failure::Random => "Cannot create an installation nonce",
        Failure::Install(error) => error.message(),
        Failure::Connect => {
            "Native host connection failed. Key-based SSH access and a trusted host key are required; verify them in a terminal."
        }
    }
}

impl Panel {
    #[cfg(feature = "visual-test")]
    pub fn fixture() -> Self {
        Self {
            initially_open: true,
            ..Default::default()
        }
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        hosts: &[Host],
        manager: &mut Manager,
    ) -> Option<Action> {
        let mut action = None;
        egui::CollapsingHeader::new(tr("Native host service (preview)")).default_open(self.initially_open).show(ui, |ui| {
            ui.label(tr("Connect installs or reuses Tessel's bundled service on this Linux host. Existing SSH terminals and SFTP files keep their current connection."));
            ui.label(tr("Editors and files still use SSH/SFTP. Native terminal access is available on Windows."));
            let mut aliases: Vec<_> = hosts.iter().map(|host| host.alias.clone()).chain(manager.aliases().map(str::to_owned)).collect();
            aliases.sort();
            aliases.dedup();
            if !aliases.contains(&self.selected) {
                self.selected = aliases.first().cloned().unwrap_or_default();
                self.error = None;
            }
            let before = self.selected.clone();
            egui::ComboBox::from_id_salt("native-host-alias").selected_text(&self.selected).show_ui(ui, |ui| {
                for alias in &aliases {
                    ui.selectable_value(&mut self.selected, alias.clone(), alias);
                }
            });
            if before != self.selected { self.error = None; }
            if self.selected.is_empty() { return; }
            if manager.pending(&self.selected) {
                ui.horizontal(|ui| { ui.spinner();
                    let cancelling = manager.cancelling(&self.selected);
                    ui.label(tr(if cancelling { "Cancelling" } else { "Connecting" }));
                    if ui.add_enabled(!cancelling, egui::Button::new(tr("Cancel"))).clicked() { manager.cancel(&self.selected); }
                });
            } else if let Some(connection) = manager.connection(&self.selected) {
                ui.label(tr(match connection.state() {
                    ConnectionState::Connected { .. } => "Connected",
                    ConnectionState::Reconnecting { .. } => "Reconnecting",
                    ConnectionState::Restarted { .. } => "Connected; the host service restarted and its previous terminals have ended.",
                    ConnectionState::Closed => "Disconnected",
                }));
                if let Some(hello) = connection.client().hello() {
                    ui.label(format!("{} {}", tr("Granted capabilities:"), hello.capabilities.iter().map(|capability| capability.name()).collect::<Vec<_>>().join(", ")));
                    if cfg!(windows) && hello.capabilities.contains(&tessel_host_protocol::Capability::Pty) {
                        ui.horizontal(|ui| {
                            if ui.button(tr("New native terminal")).clicked() { action = Some(Action::NewTerminal(self.selected.clone())); }
                            if ui.button(tr("Existing native terminals")).clicked() { action = Some(Action::ExistingTerminals(self.selected.clone())); }
                        });
                    }
                }
                ui.small(tr("Disconnect closes this connection; the service continues on the host."));
                if ui.button(tr("Disconnect")).clicked() { manager.cancel(&self.selected); }
            } else {
                if let Some(error) = self.error.or_else(|| manager.failure(&self.selected)) {
                    ui.colored_label(ui.visuals().error_fg_color, tr(failure(error)));
                    if ui.button(tr("Dismiss")).clicked() { manager.cancel(&self.selected); self.error = None; }
                }
                if ui.button(tr("Connect native host")).clicked() {
                    self.error = manager.start(&self.selected).err();
                }
            }
        });
        action
    }
}
