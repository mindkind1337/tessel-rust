use crate::i18n::tr;
use crate::share::{self, Host, Invitation};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use eframe::egui;
use serde_json::Value;
use std::{
    io::Write,
    path::Path,
    sync::mpsc::{self, Receiver},
    thread,
};

type Reply = (String, Result<Value, String>);
pub struct SharePanel {
    host: Option<Host>,
    host_project: String,
    address: String,
    invitation_text: String,
    guest: Option<Invitation>,
    pending: Option<Receiver<Reply>>,
    entries: Vec<Value>,
    path: String,
    file: Option<(String, Vec<u8>)>,
    error: String,
}
impl Default for SharePanel {
    fn default() -> Self {
        Self {
            host: None,
            host_project: String::new(),
            address: "127.0.0.1".into(),
            invitation_text: String::new(),
            guest: None,
            pending: None,
            entries: vec![],
            path: String::new(),
            file: None,
            error: String::new(),
        }
    }
}
impl SharePanel {
    pub fn hosting(&self) -> bool {
        self.host.is_some()
    }
    fn fetch(&mut self, operation: &str, path: String) {
        if self.pending.is_some() {
            return;
        }
        let Some(invitation) = self.guest.clone() else {
            return;
        };
        let operation = operation.to_string();
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        thread::spawn(move || {
            let result = share::request(&invitation, &operation, &path);
            let _ = sender.send((operation, result));
        });
    }
    pub fn show(&mut self, ui: &mut egui::Ui, project: Option<&Path>) {
        if let Some(receiver) = &self.pending
            && let Ok((operation, result)) = receiver.try_recv()
        {
            self.pending = None;
            match result {
                Ok(value) if operation == "list" => {
                    self.entries = value["entries"].as_array().cloned().unwrap_or_default();
                    self.error.clear();
                }
                Ok(value) => {
                    let name = value["name"].as_str().unwrap_or("download.bin").to_string();
                    match B64.decode(value["data"].as_str().unwrap_or("")) {
                        Ok(bytes) => {
                            self.file = Some((name, bytes));
                            self.error.clear();
                        }
                        Err(_) => self.error = "Invalid file response.".into(),
                    }
                }
                Err(error) => self.error = error,
            }
        }
        ui.heading(tr("Live Share · native read-only preview"));
        ui.label(tr("Direct LAN connection, encrypted with TLS and a pinned certificate. This is a separate protocol from Electron Tessel's Live Share."));
        ui.separator();
        ui.strong(tr("Host a project"));
        if let Some(host) = &self.host {
            ui.label(format!("{}: {}", tr("Sharing"), self.host_project));
            ui.label(format!(
                "{}: {}",
                tr("Listening"),
                host.invitation.endpoint()
            ));
            if ui.button(tr("Copy private invitation")).clicked() {
                match host.invitation.encode() {
                    Ok(invite) => ui.ctx().copy_text(invite),
                    Err(e) => self.error = e,
                }
            }
            if ui
                .button(tr("Stop sharing and revoke invitation"))
                .clicked()
            {
                self.host = None;
            }
        } else {
            if let Some(project) = project {
                ui.label(project.display().to_string());
            }
            ui.horizontal(|ui| {
                ui.label(tr("This PC's LAN IP (127.0.0.1 = this PC only)"));
                ui.text_edit_singleline(&mut self.address);
            });
            ui.label(tr("Anyone with the invitation can read this project's ordinary files (up to 8 MiB each). Hidden paths, credential files and Rust-app private data are excluded. No shell, write, browser or vault access is shared."));
            if ui
                .add_enabled(
                    project.is_some(),
                    egui::Button::new(tr("Start read-only sharing")),
                )
                .clicked()
                && let Some(root) = project
            {
                let result = self
                    .address
                    .parse::<std::net::IpAddr>()
                    .map_err(|_| "Enter a local IP address.".to_string())
                    .and_then(|ip| Host::start(root, std::net::SocketAddr::new(ip, 0), ip));
                match result {
                    Ok(host) => {
                        self.host = Some(host);
                        self.host_project = root.display().to_string();
                        self.error.clear();
                    }
                    Err(e) => self.error = e,
                }
            }
        }
        ui.separator();
        ui.strong(tr("Join as a guest"));
        if self.guest.is_none() {
            ui.add(
                egui::TextEdit::multiline(&mut self.invitation_text)
                    .desired_rows(2)
                    .hint_text(tr("Paste a private Tessel Rust invitation")),
            );
            if ui.button(tr("Join read-only")).clicked() {
                match Invitation::decode(&self.invitation_text) {
                    Ok(invite) => {
                        self.guest = Some(invite);
                        self.invitation_text.clear();
                        self.path.clear();
                        self.fetch("list", String::new());
                    }
                    Err(e) => self.error = e,
                }
            }
        } else {
            let mut disconnect = false;
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Guest · {}",
                    self.guest
                        .as_ref()
                        .map(Invitation::endpoint)
                        .unwrap_or_default()
                ));
                if ui.button(tr("Disconnect")).clicked() {
                    disconnect = true;
                }
                if ui
                    .add_enabled(self.pending.is_none(), egui::Button::new(tr("Refresh")))
                    .clicked()
                {
                    self.fetch("list", self.path.clone());
                }
            });
            if disconnect {
                self.guest = None;
                self.pending = None;
                self.entries.clear();
                self.file = None;
            }
            let mut open = None;
            if !self.path.is_empty() && ui.button(tr(".. Parent")).clicked() {
                let parent = std::path::Path::new(&self.path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                self.path = parent;
                self.fetch("list", self.path.clone());
            }
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .show(ui, |ui| {
                    for entry in &self.entries {
                        let directory = entry["directory"].as_bool().unwrap_or(false);
                        if ui
                            .button(format!(
                                "{} {}",
                                if directory { ">" } else { "·" },
                                entry["name"].as_str().unwrap_or("")
                            ))
                            .clicked()
                        {
                            open =
                                Some((entry["path"].as_str().unwrap_or("").to_string(), directory));
                        }
                    }
                });
            if let Some((path, directory)) = open {
                if directory {
                    self.path = path.clone();
                    self.fetch("list", path);
                } else {
                    self.fetch("read", path);
                }
            }
        }
        if self.pending.is_some() {
            ui.spinner();
        }
        if let Some((name, bytes)) = &self.file {
            ui.separator();
            ui.label(format!("{name} · {} {}", bytes.len(), tr("bytes")));
            if ui.button(tr("Download original file…")).clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_file_name(name.replace(['/', '\\'], "_"))
                    .save_file()
            {
                let result = (|| -> Result<(), String> {
                    let parent = path.parent().ok_or("Missing destination folder")?;
                    let mut file =
                        tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
                    file.write_all(bytes).map_err(|e| e.to_string())?;
                    file.as_file().sync_all().map_err(|e| e.to_string())?;
                    file.persist_noclobber(&path).map_err(|e| e.to_string())?;
                    Ok(())
                })();
                if let Err(e) = result {
                    self.error =
                        format!("File was not saved: {e}. Existing files are never overwritten.");
                }
            }
            if bytes.len() <= 512 * 1024
                && !bytes.contains(&0)
                && let Ok(text) = std::str::from_utf8(bytes)
            {
                egui::ScrollArea::both().max_height(220.0).show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(text).monospace()).selectable(true),
                    );
                });
            }
        }
        if !self.error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
        }
    }
}
