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
    Compare(String, String),
    GitHistory(String, bool),
    GitDiff(String, Option<String>),
    CloseComparison,
    Discard,
    ImageMenu(String, Option<(f32, f32)>),
    Download(String),
    PdfZoom(f32),
    PdfFit(String),
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
    pub live: bool,
    pub dirty: bool,
    pub error: String,
    pub entries: Vec<(String, String, bool, u64)>,
    pub new_name: String,
    pub rename: Option<(String, String)>,
    pub delete: Option<String>,
    pub document: Option<(String, Option<String>, usize)>,
    pub image_menu: Option<(f32, f32)>,
    pub pixels: Option<std::sync::Arc<gpui::RenderImage>>,
    pub pdf: Option<crate::pdf::PdfSummary>,
    pub office: Option<std::sync::Arc<crate::office::OfficeSummary>>,
    pub preview_error: String,
    pub versions: Vec<(String, i64)>,
    pub git_history: Option<std::sync::Arc<remote::file_history::Page>>,
    pub history_loading: bool,
    pub comparison: Option<std::sync::Arc<Vec<String>>>,
}

impl RemotePanel {
    pub(crate) fn input_snapshot(&self) -> Snapshot {
        Snapshot {
            connected: self.host.as_ref().map(|host| host.alias.clone()),
            path: self.path.clone(),
            busy: self.busy(),
            dirty: self.document.as_ref().is_some_and(RemoteDocument::dirty),
            new_name: self.new_name.clone(),
            rename: self.rename.clone(),
            document: self
                .document
                .as_ref()
                .map(|doc| (doc.path.clone(), doc.text.clone(), doc.original.len())),
            ..Default::default()
        }
    }
    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn hold_input_fixture(&mut self) -> impl FnOnce() + use<> {
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        move || drop(sender)
    }
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
            versions: Vec::new(),
            comparison: Default::default(),
            git_history: Default::default(),
            image_menu: None,
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
            || self.tree_watch.has_update()
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
            versions: Vec::new(),
            comparison: Default::default(),
            git_history: Default::default(),
            image_menu: None,
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
            image_menu: self.document.as_ref().and_then(|doc| doc.image_menu),
            pixels,
            git_history: self
                .document
                .as_ref()
                .and_then(|d| d.git_history.page.clone()),
            history_loading: self
                .document
                .as_ref()
                .is_some_and(|d| d.git_history.pending.is_some()),
            comparison: self
                .document
                .as_ref()
                .and_then(|d| d.comparison.lines.clone()),
            versions: self
                .document
                .iter()
                .flat_map(|d| d.versions.iter().map(|v| (v.id.clone(), v.timestamp)))
                .collect(),
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
            live: self.tree_live,
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

    fn document_pdf(&mut self, path: &str) -> Option<&mut crate::pdf::PdfView> {
        self.document
            .as_mut()
            .filter(|doc| doc.path == path)?
            .preview
            .as_mut()?
            .pdf
            .as_mut()
    }
    pub(crate) fn set_pdf_viewport(&mut self, path: &str, viewport: egui::Rect) -> bool {
        self.document_pdf(path)
            .is_some_and(|pdf| pdf.set_native_viewport(viewport))
    }

    pub(crate) fn command(&mut self, command: Command, hosts: &[Host]) {
        if let Some(doc) = &mut self.document {
            doc.image_menu = None;
        }
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
            Command::GitDiff(path, id) => self.compare_git(&path, id.as_deref()),
            Command::GitHistory(path, more) => self.load_git_history(&path, more),
            Command::Compare(path, id) => self.compare_version(&path, &id),
            Command::CloseComparison => {
                if let Some(doc) = &mut self.document {
                    doc.comparison.lines = None;
                }
            }
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
            Command::PdfFit(path) => {
                if let Some(pdf) = self.document_pdf(&path) {
                    pdf.fit_width();
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
            Command::ImageMenu(path, position) => {
                if let Some(doc) = &mut self.document
                    && doc.path == path
                    && doc.preview.as_ref().is_some_and(|p| {
                        p.pdf.is_none() && p.office.is_none() && p.gpui_image.is_some()
                    })
                {
                    doc.image_menu = position;
                }
            }
            Command::Download(path) => self.download_original(&path),
            Command::Name(name) if name.len() <= 255 => self.new_name = name,
            Command::RenameDraft(name) => {
                if let Some((_, draft)) = &mut self.rename
                    && name.len() <= 255
                {
                    *draft = name;
                }
            }
            _ if dirty => {}
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
                    self.tree_attempted_alias = None;
                    self.tree_alignment_attempted = false;
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
            Command::Refresh => {
                self.tree_alignment_attempted = false;
                self.request(Action::List(self.path.clone()));
            }
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

    pub(super) fn download_original(&mut self, path: &str) {
        self.download_original_with(path, |name| {
            crate::files::choose_download_destination(name.as_ref())
        });
    }
    fn download_original_with(
        &mut self,
        path: &str,
        choose: impl FnOnce(&str) -> Option<std::path::PathBuf>,
    ) {
        let Some(doc) = self.document.as_ref().filter(|d| d.path == path) else {
            return;
        };
        let name = doc
            .path
            .rsplit('/')
            .next()
            .unwrap_or("download.bin")
            .replace(['\\', ':'], "_");
        let Some(path) = choose(&name) else { return };
        if let Err(error) = crate::files::save_download(&path, &doc.original) {
            self.error = error.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_tree_never_starts_for_disabled_removed_or_busy_hosts() {
        let host = Host {
            alias: "fixture-no-network".into(),
            host_daemon: false,
            ..Default::default()
        };
        let mut panel = RemotePanel {
            host: Some(host.clone()),
            root: "/project".into(),
            path: "/project".into(),
            ..Default::default()
        };
        let mut manager = crate::native_host_manager::Manager::default();
        panel.poll_native_tree(&mut manager, std::slice::from_ref(&host));
        panel.poll_native_tree(&mut manager, &[]);
        let enabled = Host {
            host_daemon: true,
            ..host
        };
        let (_sender, receiver) = mpsc::channel();
        panel.pending = Some(receiver);
        panel.poll_native_tree(&mut manager, &[enabled]);
        assert_eq!(manager.aliases().count(), 0);
        assert!(!panel.tree_live);
    }

    #[test]
    fn native_tree_must_match_sftp_before_it_can_replace_post_write_results() {
        use crate::remote_tree_watch::Update;
        let entry = |size| Entry {
            name: "data".into(),
            path: "/project/data".into(),
            directory: false,
            size,
        };
        let mut panel = RemotePanel {
            entries: vec![entry(10)],
            ..Default::default()
        };
        panel.apply_tree_update(Update::Listing(vec![entry(5)]));
        assert_eq!(panel.entries[0].size, 10);
        assert!(!panel.tree_live);
        panel.apply_tree_update(Update::Listing(vec![entry(10)]));
        assert!(panel.tree_live);
        panel.apply_tree_update(Update::Listing(vec![entry(20)]));
        assert_eq!(panel.entries[0].size, 20);
        panel.apply_tree_update(Update::Fallback);
        assert!(!panel.tree_live);
        assert_eq!(panel.entries[0].size, 20);
        // A new epoch's full listing is accepted only for an already aligned watch.
        panel.apply_tree_update(Update::Listing(vec![]));
        assert!(panel.entries.is_empty());
    }

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
                versions: Vec::new(),
                comparison: Default::default(),
                git_history: Default::default(),
                image_menu: None,
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
        let destination = tempfile::tempdir().unwrap();
        let output = destination.path().join("renamed.png");
        panel.download_original_with("/old/image.png", |_| panic!("stale image"));
        panel.download_original_with("/fixture/colors.png", |name| {
            assert_eq!(name, "colors.png");
            Some(output.clone())
        });
        assert_eq!(std::fs::read(&output).unwrap(), original);
        std::fs::write(&output, b"existing").unwrap();
        panel.download_original_with("/fixture/colors.png", |_| Some(output.clone()));
        assert_eq!(std::fs::read(&output).unwrap(), b"existing");
        let first = panel.snapshot(&[]).pixels.unwrap();
        assert_eq!(
            first.as_bytes(0).unwrap(),
            &[0, 0, 255, 255, 255, 0, 0, 255]
        );
        panel.command(Command::PdfZoom(0.25), &[]); // no PDF, never mutates image bytes
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
                Ok(Vec::new()),
            )))
            .unwrap();
        panel.poll();
        panel
    }

    #[test]
    fn remote_drafts_remain_editable_with_dialog_and_dirty_document() {
        let mut panel = text_panel();
        panel.rename = Some(("/fixture/note.txt".into(), "note.txt".into()));
        panel.command(Command::Edit("modified".into()), &[]);
        panel.command(Command::Name("new.txt".into()), &[]);
        panel.command(Command::RenameDraft("renamed.txt".into()), &[]);
        assert_eq!(panel.new_name, "new.txt");
        assert_eq!(panel.rename.as_ref().unwrap().1, "renamed.txt");
        panel.command(Command::ConfirmRename, &[]);
        assert!(!panel.busy());
        assert!(panel.rename.is_some());
        assert!(panel.input_snapshot().dirty);
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
