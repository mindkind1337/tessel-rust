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
type PreparedReply = Result<(Reply, Option<Result<Box<crate::files::Document>, String>>), String>;
fn prepare_preview(
    path: &str,
    bytes: &[u8],
) -> Option<Result<Box<crate::files::Document>, String>> {
    let extension = path.rsplit('.').next()?.to_ascii_lowercase();
    if !crate::office::supported(&extension)
        && !matches!(
            extension.as_str(),
            "pdf" | "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
        )
    {
        return None;
    }
    Some((|| {
        let dir = tempfile::tempdir().map_err(|_| "Cannot create private preview buffer")?;
        let path = dir.path().join(format!("preview.{extension}"));
        std::fs::write(&path, bytes).map_err(|_| "Cannot write private preview buffer")?;
        // Media constructors fully load bounded bytes. The temporary file is removed here.
        crate::files::Document::open(dir.path(), &path).map(Box::new)
    })())
}
struct RemoteDocument {
    path: String,
    original: Vec<u8>,
    text: Option<String>,
    preview: Option<Box<crate::files::Document>>,
    preview_error: String,
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
    new_name: String,
    rename: Option<(String, String)>,
    delete: Option<Entry>,
    host: Option<Host>,
    root: String,
    path: String,
    entries: Vec<Entry>,
    pending: Option<Receiver<PreparedReply>>,
    submitted: Option<Vec<u8>>,
    document: Option<RemoteDocument>,
    error: String,
}
impl RemotePanel {
    #[cfg(feature = "visual-test")]
    pub fn fixture() -> Self {
        let image = image::RgbaImage::from_fn(320, 160, |x, y| {
            image::Rgba([(x % 256) as u8, (y + 70) as u8, 140, 255])
        });
        let mut bytes = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, image::ImageFormat::Png)
            .expect("fixture PNG");
        let original = bytes.into_inner();
        Self {
            host: Some(Host {
                alias: "fixture-host".into(),
                folder: "/demo".into(),
            }),
            root: "/demo".into(),
            path: "/demo".into(),
            entries: vec![Entry {
                name: "preview.png".into(),
                path: "/demo/preview.png".into(),
                directory: false,
                size: original.len() as u64,
            }],
            document: Some(RemoteDocument {
                path: "/demo/preview.png".into(),
                preview: prepare_preview("preview.png", &original).unwrap().ok(),
                original,
                text: None,
                preview_error: String::new(),
            }),
            ..Default::default()
        }
    }

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
            let result = remote::execute(host, action).map(|reply| {
                let preview = match &reply {
                    Reply::File { path, bytes, .. } => prepare_preview(path, bytes),
                    _ => None,
                };
                (reply, preview)
            });
            let _ = tx.send(result);
        });
    }
    pub fn poll(&mut self) {
        let result = self.pending.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                "SFTP worker stopped without a result; verify the remote file before retrying a save.".into(),
            )),
        });
        if let Some(result) = result {
            self.pending = None;
            let (result, preview) = match result {
                Ok((reply, preview)) => (Ok(reply), preview),
                Err(e) => (Err(e), None),
            };
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
                    let (preview, preview_error) = match preview {
                        Some(Ok(doc)) => (Some(doc), String::new()),
                        Some(Err(e)) => (None, e),
                        None => (None, String::new()),
                    };
                    self.document = Some(RemoteDocument {
                        path,
                        original: bytes,
                        text: if preview.is_some() { None } else { text },
                        preview,
                        preview_error,
                    });
                }
                Ok(Reply::Deleted { path, parent }) => {
                    if self.document.as_ref().is_some_and(|doc| doc.path == path) {
                        self.document = None;
                    }
                    self.request(Action::List(parent));
                }
                Ok(Reply::Renamed { old, new, parent }) => {
                    if let Some(doc) = &mut self.document {
                        if doc.path == old {
                            doc.path = new;
                        } else if let Some(suffix) = doc.path.strip_prefix(&(old + "/")) {
                            doc.path = format!("{new}/{suffix}");
                        }
                    }
                    self.request(Action::List(parent));
                }
                Ok(Reply::Created { parent }) => {
                    self.new_name.clear();
                    self.request(Action::List(parent));
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
            ui.add_enabled_ui(
                !busy && !dirty && self.rename.is_none() && self.delete.is_none(),
                |ui| {
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
                },
            );
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
        ui.add_enabled_ui(
            !busy && !dirty && self.rename.is_none() && self.delete.is_none(),
            |ui| {
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
                if self.host.is_some() && !self.path.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(tr("New name"));
                        ui.add(egui::TextEdit::singleline(&mut self.new_name).desired_width(190.0));
                        for (caption, directory) in
                            [("Create file", false), ("Create folder", true)]
                        {
                            if ui
                                .add_enabled(
                                    !self.new_name.is_empty(),
                                    egui::Button::new(tr(caption)),
                                )
                                .clicked()
                            {
                                action = Some(Action::Create {
                                    parent: self.path.clone(),
                                    name: self.new_name.clone(),
                                    directory,
                                });
                            }
                        }
                    });
                }
                egui::ScrollArea::vertical().max_height(180.0).show_rows(
                    ui,
                    22.0,
                    self.entries.len(),
                    |ui, rows| {
                        for i in rows {
                            let entry = &self.entries[i];
                            let response = ui.button(format!(
                                "{} {}{}",
                                if entry.directory { ">" } else { "·" },
                                entry.name,
                                if entry.directory {
                                    String::new()
                                } else {
                                    format!(" · {} B", entry.size)
                                }
                            ));
                            response.context_menu(|ui| {
                                if ui.button(tr("Delete…")).clicked() {
                                    self.delete = Some(entry.clone());
                                    ui.close();
                                }

                                if ui.button(tr("Rename…")).clicked() {
                                    self.rename = Some((entry.path.clone(), entry.name.clone()));
                                    ui.close();
                                }
                            });
                            if response.clicked() {
                                action = Some(if entry.directory {
                                    Action::List(entry.path.clone())
                                } else {
                                    Action::Read(entry.path.clone())
                                });
                            }
                        }
                    },
                );
            },
        );
        if let Some(entry) = &self.delete {
            let mut close = false;
            egui::Modal::new(egui::Id::new("remote-delete")).show(ui.ctx(), |ui| {
                ui.heading(tr("Delete remote item permanently?"));
                if let Some(host) = &self.host {
                    ui.label(&host.alias);
                }
                ui.label(&entry.path);
                ui.label(tr(
                    "This does not use a recycle bin. Only empty folders can be deleted.",
                ));
                if ui
                    .add_enabled(!busy, egui::Button::new(tr("Delete permanently")))
                    .clicked()
                {
                    action = Some(Action::Delete {
                        path: entry.path.clone(),
                        directory: entry.directory,
                    });
                    close = true;
                }
                if ui.button(tr("Cancel")).clicked() {
                    close = true;
                }
            });
            if close {
                self.delete = None;
            }
        }
        if let Some((path, name)) = &mut self.rename {
            let mut close = false;
            egui::Modal::new(egui::Id::new("remote-rename")).show(ui.ctx(), |ui| {
                ui.heading(tr("Rename remote item"));
                ui.label(path.as_str());
                ui.text_edit_singleline(name);
                if ui
                    .add_enabled(!busy && !name.is_empty(), egui::Button::new(tr("Rename")))
                    .clicked()
                {
                    action = Some(Action::Rename {
                        path: path.clone(),
                        name: name.clone(),
                    });
                    close = true;
                }
                if ui.button(tr("Cancel")).clicked() {
                    close = true;
                }
            });
            if close {
                self.rename = None;
            }
        }
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
            if !doc.preview_error.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &doc.preview_error);
            }
            if let Some(preview) = &mut doc.preview {
                if let Some(office) = &mut preview.office {
                    office.show(ui);
                } else if let Some(pdf) = &mut preview.pdf {
                    pdf.show(ui);
                } else {
                    if preview.texture.is_none()
                        && let Some(image) = preview.image.take()
                    {
                        preview.texture = Some(ui.ctx().load_texture(
                            &doc.path,
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                    if let Some(texture) = &preview.texture {
                        egui::ScrollArea::both()
                            .id_salt("remote-image")
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Image::new(texture)
                                        .max_width(ui.available_width())
                                        .maintain_aspect_ratio(true),
                                );
                            });
                    }
                }
            } else if let Some(text) = &mut doc.text {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_media_keeps_bytes_and_removes_temporary_files() {
        let pdf = crate::pdf::fixture();
        let view = prepare_preview("/server/document.pdf", &pdf)
            .unwrap()
            .unwrap();
        assert!(view.pdf.is_some());
        assert!(!view.path.exists());
        let mut data = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 3)
            .write_to(&mut data, image::ImageFormat::Png)
            .unwrap();
        let original = data.into_inner();
        let view = prepare_preview("/server/image.PNG", &original)
            .unwrap()
            .unwrap();
        assert_eq!(view.image.unwrap().size, [2, 3]);
        assert!(!view.path.exists());
        assert!(prepare_preview("broken.png", b"invalid").unwrap().is_err());
        assert!(prepare_preview("script.html", b"<script>bad</script>").is_none());
    }
    #[test]
    fn disconnected_sftp_worker_keeps_unsaved_text_and_releases_busy() {
        let (sender, receiver) = mpsc::channel();
        let mut panel = RemotePanel {
            pending: Some(receiver),
            submitted: Some(b"edited".to_vec()),
            document: Some(RemoteDocument {
                path: "/fixture/file.txt".into(),
                original: b"original".to_vec(),
                text: Some("edited".into()),
                preview: None,
                preview_error: String::new(),
            }),
            ..Default::default()
        };
        panel.poll();
        assert!(panel.busy());
        drop(sender);
        panel.poll();
        assert!(!panel.busy());
        assert!(panel.submitted.is_none());
        assert!(panel.error.contains("stopped without a result"));
        let doc = panel.document.as_ref().unwrap();
        assert_eq!(doc.original, b"original");
        assert_eq!(doc.text.as_deref(), Some("edited"));
        assert!(panel.needs_close_confirmation());
        // Receiving a later successful operation still updates the panel normally.
        let (sender, receiver) = mpsc::channel();
        panel.pending = Some(receiver);
        panel.submitted = Some(b"edited".to_vec());
        sender.send(Ok((Reply::Saved, None))).unwrap();
        panel.poll();
        assert!(!panel.busy());
        assert!(!panel.document.as_ref().unwrap().dirty());
    }
}
