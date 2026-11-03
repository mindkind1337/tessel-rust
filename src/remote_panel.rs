use crate::i18n::tr;
use crate::{
    remote::{self, Action, Entry, Reply},
    workspace::Host,
};
use eframe::egui;
use std::{
    io::Write,
    sync::mpsc::{self, Receiver},
};
struct RemoteDocument {
    path: String,
    original: Vec<u8>,
    text: Option<String>,
}
impl RemoteDocument {
    fn dirty(&self) -> bool {
        self.text
            .as_ref()
            .is_some_and(|t| t.as_bytes() != self.original)
    }
}
#[derive(Default)]
pub struct RemotePanel {
    selected: usize,
    host: Option<Host>,
    root: String,
    path: String,
    entries: Vec<Entry>,
    pending: Option<Receiver<Result<Reply, String>>>,
    submitted: Option<Vec<u8>>,
    document: Option<RemoteDocument>,
    error: String,
}
impl RemotePanel {
    pub fn needs_close_confirmation(&self) -> bool {
        self.pending.is_some() || self.document.as_ref().is_some_and(RemoteDocument::dirty)
    }
    fn request(&mut self, action: Action) {
        if self.pending.is_some() {
            return;
        }
        let Some(mut host) = self.host.clone() else {
            return;
        };
        if !self.root.is_empty() {
            host.folder = self.root.clone();
        }
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.error.clear();
        std::thread::spawn(move || {
            let _ = tx.send(remote::execute(host, action));
        });
    }
    pub fn poll(&mut self) {
        if let Some(rx) = &self.pending
            && let Ok(result) = rx.try_recv()
        {
            self.pending = None;
            match result {
                Ok(Reply::Listing {
                    root,
                    path,
                    entries,
                }) => {
                    self.root = root;
                    self.path = path;
                    self.entries = entries;
                }
                Ok(Reply::File { root, path, bytes }) => {
                    self.root = root;
                    let text = if bytes.len() <= 2 * 1024 * 1024 && !bytes.contains(&0) {
                        String::from_utf8(bytes.clone()).ok()
                    } else {
                        None
                    };
                    self.document = Some(RemoteDocument {
                        path,
                        original: bytes,
                        text,
                    });
                }
                Ok(Reply::Saved) => {
                    if let Some(bytes) = self.submitted.take()
                        && let Some(doc) = &mut self.document
                    {
                        doc.original = bytes;
                    }
                }
                Err(e) => {
                    self.submitted = None;
                    self.error = e;
                }
            }
        }
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn show(&mut self, ui: &mut egui::Ui, hosts: &[Host]) {
        self.poll();
        ui.heading(tr("Remote files · SFTP"));
        ui.label(tr("Uses OpenSSH config and existing key/agent authentication. Unknown host keys are refused. Connect from a terminal first when trust or authentication needs setup."));
        if hosts.is_empty() && self.host.is_none() {
            ui.label(tr("Add an SSH host in Settings first."));
            return;
        }
        self.selected = self.selected.min(hosts.len().saturating_sub(1));
        let busy = self.pending.is_some();
        let dirty = self.document.as_ref().is_some_and(RemoteDocument::dirty);
        if !hosts.is_empty() {
            ui.add_enabled_ui(!busy && !dirty, |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("remote-host")
                        .selected_text(&hosts[self.selected].alias)
                        .show_ui(ui, |ui| {
                            for (index, host) in hosts.iter().enumerate() {
                                ui.selectable_value(&mut self.selected, index, &host.alias);
                            }
                        });
                    if ui.button(tr("Connect / change host")).clicked() {
                        self.host = Some(hosts[self.selected].clone());
                        self.root.clear();
                        self.path.clear();
                        self.entries.clear();
                        self.document = None;
                        self.request(Action::List(String::new()));
                    }
                    if self.host.is_some() && ui.button(tr("Disconnect")).clicked() {
                        self.host = None;
                        self.root.clear();
                        self.entries.clear();
                        self.document = None;
                    }
                });
            });
        }
        if let Some(host) = &self.host {
            ui.strong(format!("{}:{}", host.alias, self.path));
        }
        if busy {
            ui.spinner();
            ui.label(tr("SFTP request in progress (30-second limit)…"));
        }
        if !self.error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
        }
        if dirty {
            ui.label(tr(
                "Save or discard local edits before opening another remote file or host.",
            ));
        }
        let mut action = None;
        ui.add_enabled_ui(!busy && !dirty, |ui| {
            ui.horizontal(|ui| {
                if self.host.is_some() && ui.button(tr("Refresh")).clicked() {
                    action = Some(Action::List(self.path.clone()));
                }
                if self.path != self.root
                    && !self.path.is_empty()
                    && ui.button(tr("Parent folder")).clicked()
                {
                    let parent = self.path.rsplit_once('/').map(|p| p.0).unwrap_or("");
                    action = Some(Action::List(if parent.is_empty() {
                        "/".into()
                    } else {
                        parent.into()
                    }));
                }
            });
            egui::ScrollArea::vertical().max_height(180.0).show_rows(
                ui,
                22.0,
                self.entries.len(),
                |ui, rows| {
                    for i in rows {
                        let entry = &self.entries[i];
                        if ui
                            .button(format!(
                                "{} {}{}",
                                if entry.directory { ">" } else { "·" },
                                entry.name,
                                if entry.directory {
                                    String::new()
                                } else {
                                    format!(" · {} B", entry.size)
                                }
                            ))
                            .clicked()
                        {
                            action = Some(if entry.directory {
                                Action::List(entry.path.clone())
                            } else {
                                Action::Read(entry.path.clone())
                            });
                        }
                    }
                },
            );
        });
        if let Some(doc) = &mut self.document {
            ui.separator();
            ui.strong(&doc.path);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !busy && doc.dirty(),
                        egui::Button::new(tr("Save to remote host")),
                    )
                    .clicked()
                {
                    let replacement = doc.text.as_ref().unwrap().as_bytes().to_vec();
                    self.submitted = Some(replacement.clone());
                    action = Some(Action::Save {
                        path: doc.path.clone(),
                        original: doc.original.clone(),
                        replacement,
                    });
                }
                if ui
                    .add_enabled(
                        !busy && doc.dirty(),
                        egui::Button::new(tr("Discard local edits")),
                    )
                    .clicked()
                {
                    doc.text = String::from_utf8(doc.original.clone()).ok();
                }
                if ui.button(tr("Download original…")).clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_file_name(
                            doc.path
                                .rsplit('/')
                                .next()
                                .unwrap_or("download.bin")
                                .replace(['\\', ':'], "_"),
                        )
                        .save_file()
                {
                    let result = (|| -> Result<(), String> {
                        let mut file = tempfile::NamedTempFile::new_in(
                            path.parent().ok_or("Missing download folder")?,
                        )
                        .map_err(|e| e.to_string())?;
                        file.write_all(&doc.original).map_err(|e| e.to_string())?;
                        file.as_file().sync_all().map_err(|e| e.to_string())?;
                        file.persist_noclobber(path).map_err(
                            |_| "Destination exists or cannot be saved; existing files were kept",
                        )?;
                        Ok(())
                    })();
                    if let Err(e) = result {
                        self.error = e;
                    }
                }
            });
            if let Some(text) = &mut doc.text {
                ui.add_enabled_ui(!busy, |ui| {
                    egui::ScrollArea::both().show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(text)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(18),
                        );
                    });
                });
            } else {
                ui.label(format!(
                    "{} · {} {} · {}",
                    tr("Binary or large file"),
                    doc.original.len(),
                    tr("bytes"),
                    tr("Download the original to view it locally")
                ));
            }
        }
        if let Some(action) = action {
            self.request(action);
        }
    }
}
