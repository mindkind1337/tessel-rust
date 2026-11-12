//! Renderer-independent commands over the existing SFTP panel state.
use super::*;

#[derive(Clone, Debug)]
pub(crate) enum Command {
    Connect(usize),
    Disconnect,
    Refresh,
    Parent,
    Open(String),
    Name(String),
    Create(bool),
    BeginRename(String),
    RenameDraft(String),
    ConfirmRename,
    BeginDelete(String),
    ConfirmDelete,
    CancelDialog,
    Edit(String),
    Save,
    Discard,
    Download,
    PdfPage(isize),
    PdfZoom(f32),
    OfficeSection(usize),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Snapshot {
    pub hosts: Vec<String>,
    pub selected: usize,
    pub connected: Option<String>,
    pub path: String,
    pub can_parent: bool,
    pub busy: bool,
    pub dirty: bool,
    pub error: String,
    pub entries: Vec<(String, String, bool, u64)>,
    pub new_name: String,
    pub rename: Option<(String, String)>,
    pub delete: Option<String>,
    pub document: Option<(String, Option<String>, usize)>,
    pub pixels: Option<std::sync::Arc<gpui::RenderImage>>,
    pub pdf: Option<crate::pdf::PdfSummaryTuple>,
    pub office: Option<crate::office::OfficeSummary>,
    pub preview_error: String,
}

impl RemotePanel {
    #[cfg(feature = "visual-test")]
    pub(crate) fn text_fixture() -> Self {
        let mut panel = Self::fixture();
        panel.entries.push(Entry {
            name: "notes.txt".into(),
            path: "/demo/notes.txt".into(),
            directory: false,
            size: 32,
        });
        panel.document = Some(RemoteDocument {
            path: "/demo/notes.txt".into(),
            original: b"Synthetic remote text\nSecond line\n".to_vec(),
            text: Some("Synthetic remote text\nSecond line\n".into()),
            preview: None,
            preview_error: String::new(),
        });
        panel
    }
    pub(crate) fn gpui_needs_frame(&self) -> bool {
        self.busy()
            || self
                .document
                .as_ref()
                .and_then(|doc| doc.preview.as_ref())
                .is_some_and(|doc| {
                    doc.pdf
                        .as_ref()
                        .is_some_and(crate::pdf::PdfView::is_rendering)
                        || doc
                            .office
                            .as_ref()
                            .is_some_and(crate::office::OfficeView::is_rendering)
                })
    }

    #[cfg(feature = "visual-test")]
    pub(crate) fn pdf_fixture() -> Self {
        let mut panel = Self::fixture();
        let original = crate::pdf::fixture();
        panel.document = Some(RemoteDocument {
            path: "/demo/original.pdf".into(),
            preview: prepare_preview("original.pdf", &original).unwrap().ok(),
            original,
            text: None,
            preview_error: String::new(),
        });
        panel
    }

    pub(crate) fn snapshot(&mut self, hosts: &[Host]) -> Snapshot {
        let mut pdf = None;
        let mut pixels = None;
        let mut office = None;
        if let Some(preview) = self.document.as_mut().and_then(|doc| doc.preview.as_mut()) {
            office = preview
                .office
                .as_mut()
                .map(crate::office::OfficeView::gpui_poll_summary);
            if let Some(view) = &mut preview.pdf {
                pdf = Some(view.gpui_poll_summary());
                pixels = view.gpui_image();
            } else {
                if preview.gpui_image.is_none() {
                    preview.gpui_image = preview
                        .image
                        .as_ref()
                        .and_then(crate::gpui_pixels::from_color_image);
                }
                pixels = preview.gpui_image.clone();
            }
        }
        Snapshot {
            office,
            pdf,
            pixels,
            preview_error: self
                .document
                .as_ref()
                .map(|doc| doc.preview_error.clone())
                .unwrap_or_default(),
            hosts: hosts.iter().map(|h| h.alias.clone()).collect(),
            selected: self.selected,
            connected: self.host.as_ref().map(|h| h.alias.clone()),
            path: self.path.clone(),
            can_parent: !self.path.is_empty() && self.path != self.root,
            busy: self.busy(),
            dirty: self.document.as_ref().is_some_and(RemoteDocument::dirty),
            error: self.error.clone(),
            entries: self
                .entries
                .iter()
                .map(|e| (e.path.clone(), e.name.clone(), e.directory, e.size))
                .collect(),
            new_name: self.new_name.clone(),
            rename: self.rename.clone(),
            delete: self.delete.as_ref().map(|e| e.path.clone()),
            document: self
                .document
                .as_ref()
                .map(|d| (d.path.clone(), d.text.clone(), d.original.len())),
        }
    }

    pub(crate) fn command(&mut self, command: Command, hosts: &[Host]) {
        if matches!(command, Command::CancelDialog) {
            self.rename = None;
            self.delete = None;
            return;
        }
        if self.busy() {
            return;
        }
        let dirty = self.document.as_ref().is_some_and(RemoteDocument::dirty);
        match command {
            Command::OfficeSection(index) => {
                if let Some(office) = self
                    .document
                    .as_mut()
                    .and_then(|doc| doc.preview.as_mut())
                    .and_then(|doc| doc.office.as_mut())
                {
                    office.select_section(index);
                }
            }
            Command::PdfPage(delta) => {
                if let Some(pdf) = self
                    .document
                    .as_mut()
                    .and_then(|doc| doc.preview.as_mut())
                    .and_then(|doc| doc.pdf.as_mut())
                {
                    pdf.step_page(delta);
                }
            }
            Command::PdfZoom(delta) => {
                if let Some(pdf) = self
                    .document
                    .as_mut()
                    .and_then(|doc| doc.preview.as_mut())
                    .and_then(|doc| doc.pdf.as_mut())
                {
                    pdf.step_zoom(delta);
                }
            }
            Command::Edit(text) => {
                if text.len() <= 2 * 1024 * 1024
                    && let Some(doc) = &mut self.document
                    && doc.text.is_some()
                {
                    doc.text = Some(text);
                }
            }
            Command::Save => {
                if let Some(doc) = &self.document
                    && doc.dirty()
                    && let Some(text) = &doc.text
                {
                    let replacement = text.as_bytes().to_vec();
                    self.submitted = Some(replacement.clone());
                    self.request(Action::Save {
                        path: doc.path.clone(),
                        original: doc.original.clone(),
                        replacement,
                    });
                }
            }
            Command::Discard => {
                if let Some(doc) = &mut self.document
                    && doc.text.is_some()
                {
                    doc.text = String::from_utf8(doc.original.clone()).ok();
                }
            }
            Command::Download => self.download_original(),
            _ if dirty => {}
            Command::RenameDraft(name) => {
                if let Some((_, draft)) = &mut self.rename
                    && name.len() <= 255
                {
                    *draft = name;
                }
            }
            Command::ConfirmRename => {
                if let Some((path, name)) = self.rename.take()
                    && !name.is_empty()
                {
                    self.request(Action::Rename { path, name });
                }
            }
            Command::ConfirmDelete => {
                if let Some(entry) = self.delete.take() {
                    self.request(Action::Delete {
                        path: entry.path,
                        directory: entry.directory,
                    });
                }
            }
            _ if self.rename.is_some() || self.delete.is_some() => {}
            Command::Connect(index) => {
                if let Some(host) = hosts.get(index) {
                    self.selected = index;
                    self.host = Some(host.clone());
                    self.root.clear();
                    self.path.clear();
                    self.entries.clear();
                    self.document = None;
                    self.request(Action::List(String::new()));
                }
            }
            Command::Disconnect => {
                self.host = None;
                self.root.clear();
                self.path.clear();
                self.entries.clear();
                self.document = None;
            }
            Command::Refresh => self.request(Action::List(self.path.clone())),
            Command::Parent if !self.path.is_empty() && self.path != self.root => {
                let parent = self.path.rsplit_once('/').map(|p| p.0).unwrap_or("");
                self.request(Action::List(if parent.is_empty() {
                    "/".into()
                } else {
                    parent.into()
                }));
            }
            Command::Open(path) => {
                if let Some(entry) = self.entries.iter().find(|entry| entry.path == path) {
                    self.request(if entry.directory {
                        Action::List(path)
                    } else {
                        Action::Read(path)
                    });
                }
            }
            Command::Name(name) if name.len() <= 255 => self.new_name = name,
            Command::Create(directory) if !self.new_name.is_empty() && !self.path.is_empty() => {
                self.request(Action::Create {
                    parent: self.path.clone(),
                    name: self.new_name.clone(),
                    directory,
                });
            }
            Command::BeginRename(path) => {
                if let Some(entry) = self.entries.iter().find(|e| e.path == path) {
                    self.rename = Some((entry.path.clone(), entry.name.clone()));
                }
            }
            Command::BeginDelete(path) => {
                self.delete = self.entries.iter().find(|e| e.path == path).cloned()
            }
            _ => {}
        }
    }

    fn download_original(&mut self) {
        let Some(doc) = &self.document else {
            return;
        };
        let name = doc
            .path
            .rsplit('/')
            .next()
            .unwrap_or("download.bin")
            .replace(['\\', ':'], "_");
        let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() else {
            return;
        };
        let result = (|| -> Result<(), String> {
            let mut file =
                tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing download folder")?)
                    .map_err(|e| e.to_string())?;
            file.write_all(&doc.original).map_err(|e| e.to_string())?;
            file.as_file().sync_all().map_err(|e| e.to_string())?;
            file.persist_noclobber(path)
                .map_err(|_| "Destination exists or cannot be saved; existing files were kept")?;
            Ok(())
        })();
        if let Err(error) = result {
            self.error = error;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_media_keeps_original_bytes_and_caches_native_pixels() {
        let image = image::RgbaImage::from_fn(2, 1, |x, _| {
            image::Rgba(if x == 0 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            })
        });
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let original = encoded.into_inner();
        let mut panel = RemotePanel {
            document: Some(RemoteDocument {
                path: "/fixture/colors.png".into(),
                preview: prepare_preview("colors.png", &original)
                    .unwrap()
                    .unwrap()
                    .into(),
                original: original.clone(),
                text: None,
                preview_error: String::new(),
            }),
            ..Default::default()
        };
        let first = panel.snapshot(&[]).pixels.unwrap();
        assert_eq!(
            first.as_bytes(0).unwrap(),
            &[0, 0, 255, 255, 255, 0, 0, 255]
        );
        panel.command(Command::PdfPage(1), &[]); // no PDF, never mutates image bytes
        let second = panel.snapshot(&[]).pixels.unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert_eq!(panel.document.as_ref().unwrap().original, original);
        assert!(!panel.snapshot(&[]).dirty);
    }

    fn text_panel() -> RemotePanel {
        let (sender, receiver) = mpsc::channel();
        let mut panel = RemotePanel {
            pending: Some(receiver),
            ..Default::default()
        };
        sender
            .send(Ok((
                Reply::File {
                    root: "/fixture".into(),
                    path: "/fixture/note.txt".into(),
                    bytes: b"original".to_vec(),
                },
                None,
            )))
            .unwrap();
        panel.poll();
        panel
    }

    #[test]
    fn remote_commands_preserve_dirty_text_and_block_navigation_until_discarded() {
        let mut panel = text_panel();
        panel.command(Command::Edit("modified".into()), &[]);
        assert!(panel.snapshot(&[]).dirty);
        panel.command(Command::Disconnect, &[]);
        panel.command(Command::Refresh, &[]);
        panel.command(Command::Open("/unlisted".into()), &[]);
        assert_eq!(
            panel.snapshot(&[]).document.unwrap().1.as_deref(),
            Some("modified")
        );
        assert!(!panel.busy());
        panel.command(Command::Discard, &[]);
        assert_eq!(
            panel.snapshot(&[]).document.unwrap().1.as_deref(),
            Some("original")
        );
        panel.command(Command::Disconnect, &[]);
        assert!(panel.snapshot(&[]).document.is_none());
    }

    #[test]
    fn remote_commands_reject_stale_rows_and_preserve_inflight_save_state() {
        let mut panel = text_panel();
        panel.command(Command::BeginDelete("/not-listed".into()), &[]);
        panel.command(Command::BeginRename("/not-listed".into()), &[]);
        assert!(panel.delete.is_none() && panel.rename.is_none());
        let (_sender, receiver) = mpsc::channel();
        panel.pending = Some(receiver);
        panel.submitted = Some(b"submitted".to_vec());
        panel.command(Command::Edit("must not overwrite".into()), &[]);
        panel.command(Command::Discard, &[]);
        panel.command(Command::Disconnect, &[]);
        assert_eq!(
            panel.snapshot(&[]).document.unwrap().1.as_deref(),
            Some("original")
        );
        assert_eq!(panel.submitted.as_deref(), Some(b"submitted".as_slice()));
        assert!(panel.busy());
    }
}
