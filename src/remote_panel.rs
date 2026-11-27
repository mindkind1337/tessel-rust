use crate::i18n::tr;
pub(crate) mod control;
use crate::{
    remote::{self, Action, Entry, Reply},
    workspace::Host,
};
use eframe::egui;
use std::sync::mpsc::{self, Receiver};
type Versions = Vec<crate::local_file_history::Entry>;
type PreparedReply = Result<
    (
        Reply,
        Option<Result<Box<crate::files::Document>, String>>,
        Result<Versions, String>,
    ),
    String,
>;

fn execute_for_panel(
    profile: Option<&std::path::Path>,
    host: Host,
    action: Action,
    run: impl FnOnce(Host, Action) -> Result<Reply, String>,
) -> PreparedReply {
    let alias = host.alias.clone();
    let save = match &action {
        Action::Save {
            path, replacement, ..
        } => Some((
            path.clone(),
            (replacement.len() <= crate::local_file_history::MAX_BYTES)
                .then(|| std::str::from_utf8(replacement).ok())
                .flatten()
                .map(str::to_owned),
        )),
        _ => None,
    };
    let reply = run(host, action)?;
    let preview = match &reply {
        Reply::File { path, bytes, .. } => prepare_preview(path, bytes),
        _ => None,
    };
    let history = match (&reply, profile) {
        (Reply::File { path, .. }, Some(profile)) => {
            crate::local_file_history::remote_versions(profile, &alias, path, None)
        }
        (Reply::Saved, Some(profile)) => match save {
            Some((path, text)) => {
                crate::local_file_history::remote_versions(profile, &alias, &path, text.as_deref())
            }
            None => Ok(Vec::new()),
        },
        _ => Ok(Vec::new()),
    }
    .map_err(|e| format!("Local history: {e}"));
    Ok((reply, preview, history))
}

fn prepare_preview(
    path: &str,
    bytes: &[u8],
) -> Option<Result<Box<crate::files::Document>, String>> {
    let extension = path.rsplit('.').next()?.to_ascii_lowercase();
    if let Some(mime) = crate::files::playback::mime(&extension) {
        return Some(crate::files::Document::remote_playback(path, mime, bytes).map(Box::new));
    }
    if !crate::office::supported(&extension)
        && extension != "pdf"
        && !crate::raster_preview::supported(&extension)
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
#[derive(Default)]
struct Comparison {
    pending: Option<Receiver<Result<Vec<String>, String>>>,
    lines: Option<std::sync::Arc<Vec<String>>>,
}

#[derive(Default)]
struct GitHistory {
    pending: Option<Receiver<Result<remote::file_history::Page, String>>>,
    page: Option<std::sync::Arc<remote::file_history::Page>>,
}
struct RemoteDocument {
    html_page: Option<(u64, std::sync::Arc<str>)>,
    html_plain: Option<std::sync::Arc<str>>,
    image_menu: Option<(f32, f32)>,
    path: String,
    original: Vec<u8>,
    versions: Versions,
    comparison: Comparison,
    git_history: GitHistory,
    text: Option<String>,
    preview: Option<Box<crate::files::Document>>,
    preview_error: String,
}
impl RemoteDocument {
    fn show_html_page(&mut self) {
        self.html_plain = None;
        self.html_page = self.text.as_ref().map(|s| {
            (
                crate::files::Document::next_instance_id(),
                crate::preview::html_plain_text(s).into(),
            )
        });
    }
    fn set_plain(&mut self, enabled: bool) {
        self.html_page = None;
        self.html_plain = self
            .text
            .as_deref()
            .filter(|_| enabled)
            .map(|text| crate::preview::html_plain_text(text).into());
    }
    fn discard_text(&mut self) {
        self.text = String::from_utf8(self.original.clone()).ok();
        if self.html_page.is_some() {
            self.show_html_page();
        } else {
            self.set_plain(self.html_plain.is_some());
        }
    }
    fn dirty(&self) -> bool {
        self.text
            .as_ref()
            .is_some_and(|t| t.as_bytes() != self.original)
    }
}
#[derive(Default)]
pub struct RemotePanel {
    profile: Option<std::path::PathBuf>,
    #[cfg(feature = "visual-test")]
    fixture_mode: bool,
    selected: usize,
    new_name: String,
    rename: Option<(String, String)>,
    delete: Option<Entry>,
    host: Option<Host>,
    pub(crate) terminal_request: Option<Host>,
    root: String,
    path: String,
    entries: Vec<Entry>,
    pending: Option<Receiver<PreparedReply>>,
    submitted: Option<Vec<u8>>,
    document: Option<RemoteDocument>,
    error: String,
    downloaded: Option<std::path::PathBuf>,
    folder_report: Option<remote::folder_download::Report>,
    folder_progress: Option<remote::folder_download::Progress>,
    file_menu: Option<(String, (f32, f32))>,
    tree_watch: crate::remote_tree_watch::Watch,
    tree_attempted_alias: Option<String>,
    tree_live: bool,
    tree_trusted: bool,
    tree_alignment_attempted: bool,
}
impl RemotePanel {
    pub fn with_profile(profile: Option<std::path::PathBuf>) -> Self {
        Self {
            profile,
            ..Default::default()
        }
    }

    pub(crate) fn saved_target_removed(&mut self, index: usize, target: &Host, remaining: usize) {
        let selected = if self.selected > index {
            self.selected - 1
        } else {
            self.selected.min(remaining.saturating_sub(1))
        };
        if self
            .host
            .as_ref()
            .is_some_and(|host| host.alias == target.alias && host.folder == target.folder)
        {
            // Drop the watch and response receiver along with the document. A late
            // reply from this target must never populate a later Files connection.
            *self = Self::with_profile(self.profile.take());
        }
        self.selected = selected;
    }

    #[cfg(windows)]
    pub(crate) fn html_page_id(&self) -> Option<u64> {
        Some(self.document.as_ref()?.html_page.as_ref()?.0)
    }
    #[cfg(windows)]
    pub(crate) fn active_html_page(&self, id: u64) -> Option<(Host, &str, &str, &str)> {
        let doc = self.document.as_ref()?;
        if doc.html_page.as_ref()?.0 != id {
            return None;
        }
        Some((
            self.host.clone()?,
            &self.root,
            &doc.path,
            doc.text.as_deref()?,
        ))
    }
    #[cfg(windows)]
    pub(crate) fn html_preview(&self, path: &str) -> Option<(Host, &str, &str)> {
        let doc = self.document.as_ref().filter(|doc| doc.path == path)?;
        Some((self.host.clone()?, self.root.as_str(), doc.text.as_deref()?))
    }
    pub fn terminal_host(&self, hosts: &[Host]) -> Option<Host> {
        self.host
            .clone()
            .or_else(|| hosts.get(self.selected).cloned())
    }
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
            fixture_mode: true,
            host: Some(Host {
                alias: "fixture-host".into(),
                folder: "/demo".into(),
                ..Default::default()
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
                versions: Vec::new(),
                comparison: Default::default(),
                git_history: Default::default(),
                html_page: None,
                html_plain: None,
                image_menu: None,
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
        #[cfg(feature = "visual-test")]
        if self.fixture_mode {
            self.error = "Synthetic fixture: remote network requests are disabled.".into();
            return;
        }
        if self.pending.is_some() {
            return;
        }
        let Some(mut host) = self.host.clone() else {
            return;
        };
        // A replacement watcher must agree with the authoritative SFTP listing
        // before its deltas can update the displayed directory.
        if !matches!(
            action,
            Action::Read(_) | Action::Download { .. } | Action::DownloadFolder { .. }
        ) {
            self.tree_watch.invalidate();
            self.tree_live = false;
            self.tree_trusted = false;
            if !matches!(action, Action::List(_)) {
                self.tree_alignment_attempted = false;
            }
        }
        if !self.root.is_empty() {
            host.folder = self.root.clone();
        }
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.error.clear();
        let profile = self.profile.clone();
        std::thread::spawn(move || {
            let result = execute_for_panel(profile.as_deref(), host, action, remote::execute);
            let _ = tx.send(result);
        });
    }
    fn compare_version(&mut self, path: &str, id: &str) {
        self.compare_version_with(path, id, remote::execute)
    }
    fn compare_version_with(
        &mut self,
        path: &str,
        id: &str,
        read: impl FnOnce(Host, Action) -> Result<Reply, String> + Send + 'static,
    ) {
        if self.busy() {
            return;
        }
        let (Some(doc), Some(mut host), Some(profile)) =
            (&mut self.document, self.host.clone(), self.profile.clone())
        else {
            return;
        };
        if doc.path != path || !doc.versions.iter().any(|entry| entry.id == id) {
            return;
        }
        if !self.root.is_empty() {
            host.folder = self.root.clone();
        }
        let alias = host.alias.clone();
        let path = path.to_owned();
        let id = id.to_owned();
        let (tx, rx) = mpsc::channel();
        doc.comparison = Comparison {
            pending: Some(rx),
            lines: None,
        };
        self.error.clear();
        std::thread::spawn(move || {
            let result = read(host, Action::Read(path.clone())).and_then(|reply| match reply {
                Reply::File {
                    path: returned,
                    bytes,
                    ..
                } if returned == path => {
                    crate::local_file_history::remote_diff(&profile, &alias, &path, &id, &bytes)
                }
                _ => Err("Remote file changed during comparison.".into()),
            });
            let _ = tx.send(result.map(|diff| diff.lines().map(str::to_owned).collect()));
        });
    }
    fn compare_git(&mut self, path: &str, id: Option<&str>) {
        if self.busy() {
            return;
        }
        let (Some(doc), Some(host)) = (&mut self.document, self.host.clone()) else {
            return;
        };
        let Some(page) = &doc.git_history.page else {
            return;
        };
        if doc.path != path {
            return;
        }
        let commit = match id {
            Some(id) => {
                let Some(row) = page.commits.iter().find(|row| row.id == id) else {
                    return;
                };
                Some(row.clone())
            }
            None if page.uncommitted => None,
            None => return,
        };
        let (tx, rx) = mpsc::channel();
        doc.comparison = Comparison {
            pending: Some(rx),
            lines: None,
        };
        self.error.clear();
        let root = self.root.clone();
        let path = doc.path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(remote::file_history::diff(
                host,
                &root,
                &path,
                commit.as_ref(),
            ));
        });
    }
    fn load_git_history(&mut self, path: &str, more: bool) {
        if self.busy() {
            return;
        }
        let (Some(doc), Some(host)) = (&mut self.document, self.host.clone()) else {
            return;
        };
        if doc.path != path || doc.git_history.pending.is_some() {
            return;
        }
        let limit = match &doc.git_history.page {
            Some(p) if more && p.more => (p.limit + 50).min(500),
            _ if more => return,
            Some(p) => p.limit,
            None => 50,
        };
        let (tx, rx) = mpsc::channel();
        doc.git_history.pending = Some(rx);
        let root = self.root.clone();
        let path = path.to_owned();
        self.error.clear();
        std::thread::spawn(move || {
            let _ = tx.send(remote::file_history::load(host, &root, &path, limit));
        });
    }
    pub fn poll(&mut self) {
        if let Some(doc) = &mut self.document
            && let Some(rx) = &doc.git_history.pending
        {
            match rx.try_recv() {
                Err(mpsc::TryRecvError::Empty) => {}
                reply => {
                    doc.git_history.pending = None;
                    match reply.unwrap_or_else(|_| Err("Remote history request stopped.".into())) {
                        Ok(page) => doc.git_history.page = Some(std::sync::Arc::new(page)),
                        Err(e) => {
                            doc.git_history.page = None;
                            self.error = e;
                        }
                    }
                }
            }
        }
        if let Some(doc) = &mut self.document
            && let Some(rx) = &doc.comparison.pending
        {
            match rx.try_recv() {
                Err(mpsc::TryRecvError::Empty) => {}
                result => {
                    doc.comparison.pending = None;
                    match result.unwrap_or_else(|_| Err("Remote comparison stopped.".into())) {
                        Ok(lines) => doc.comparison.lines = Some(std::sync::Arc::new(lines)),
                        Err(error) => self.error = error,
                    }
                }
            }
        }
        let result = self.pending.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                "SFTP worker stopped without a result; verify the remote file before retrying a save.".into(),
            )),
        });
        if let Some(result) = result {
            self.pending = None;
            self.folder_progress = None;
            let (result, preview, versions) = match result {
                Ok((reply, preview, versions)) => (
                    Ok(reply),
                    preview,
                    versions.unwrap_or_else(|e| {
                        self.error = e;
                        Vec::new()
                    }),
                ),
                Err(e) => (Err(e), None, Vec::new()),
            };
            match result {
                Ok(Reply::Listing {
                    root,
                    path,
                    entries,
                }) => {
                    self.root = root;
                    if self.path != path {
                        self.tree_alignment_attempted = false;
                    }
                    self.path = path;
                    self.entries = entries;
                }
                Ok(Reply::File { root, path, bytes }) => {
                    self.root = root;
                    let (preview, preview_error) = match preview {
                        Some(Ok(doc)) => (Some(doc), String::new()),
                        Some(Err(e)) => (None, e),
                        None => (None, String::new()),
                    };
                    let text = if preview.is_none()
                        && bytes.len() <= 2 * 1024 * 1024
                        && !bytes.contains(&0)
                    {
                        String::from_utf8(bytes.clone()).ok()
                    } else {
                        None
                    };
                    let mut doc = RemoteDocument {
                        path,
                        original: bytes,
                        versions,
                        comparison: Default::default(),
                        git_history: Default::default(),
                        html_page: None,
                        html_plain: None,
                        image_menu: None,
                        text,
                        preview,
                        preview_error,
                    };
                    if cfg!(windows) && doc.text.is_some() && control::is_html(&doc.path) {
                        doc.show_html_page();
                    }
                    self.document = Some(doc);
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
                            doc.versions.clear();
                            doc.git_history = Default::default();
                            doc.comparison = Default::default();
                        } else if let Some(suffix) = doc.path.strip_prefix(&(old + "/")) {
                            doc.path = format!("{new}/{suffix}");
                            doc.versions.clear();
                            doc.git_history = Default::default();
                            doc.comparison = Default::default();
                        }
                    }
                    self.request(Action::List(parent));
                }
                Ok(Reply::Created { parent }) => {
                    self.new_name.clear();
                    self.request(Action::List(parent));
                }
                #[cfg(windows)]
                Ok(Reply::PreviewTooLarge) => self.error = "Remote preview exceeds 50 MiB".into(),
                Ok(Reply::FolderDownloaded(report)) => self.folder_report = Some(report),
                Ok(Reply::Downloaded(path)) => self.downloaded = Some(path),
                Ok(Reply::Saved) => {
                    if let Some(bytes) = self.submitted.take()
                        && let Some(doc) = &mut self.document
                    {
                        doc.original = bytes;
                        doc.versions = versions;
                        doc.comparison = Default::default();
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
            || self
                .document
                .as_ref()
                .is_some_and(|d| d.comparison.pending.is_some() || d.git_history.pending.is_some())
    }

    pub(crate) fn poll_native_tree(
        &mut self,
        manager: &mut crate::native_host_manager::Manager,
        hosts: &[Host],
    ) {
        #[cfg(feature = "visual-test")]
        if self.fixture_mode {
            return;
        }
        manager.poll();
        let enabled = self.host.as_ref().and_then(|host| {
            hosts
                .iter()
                .find(|saved| saved.alias == host.alias && saved.host_daemon)
                .cloned()
        });
        let Some(host) = enabled else {
            self.tree_watch.poll(None, None);
            self.tree_live = false;
            self.tree_trusted = false;
            return;
        };
        // Read-only file loads keep the current subscription; destructive/list
        // requests have already invalidated it in request().
        if self.busy() || self.path.is_empty() {
            return;
        }
        if self.tree_attempted_alias.as_deref() != Some(&host.alias) {
            self.tree_attempted_alias = Some(host.alias.clone());
            // Reuse a shared connection if one exists; errors retain SFTP and
            // never create a per-frame retry loop.
            let _ = manager.start_background_if_enabled(&host);
        }
        let client = manager
            .connection(&host.alias)
            .map(|connection| connection.client().clone());
        let key = crate::remote_tree_watch::Key {
            alias: host.alias,
            root: self.root.clone(),
            path: self.path.clone(),
        };
        if let Some(update) = self.tree_watch.poll(Some(key), client) {
            self.apply_tree_update(update);
        }
    }

    fn apply_tree_update(&mut self, update: crate::remote_tree_watch::Update) {
        match update {
            crate::remote_tree_watch::Update::Listing(entries) => {
                let agrees = self.entries.len() == entries.len()
                    && self.entries.iter().zip(&entries).all(|(a, b)| {
                        a.path == b.path
                            && a.name == b.name
                            && a.directory == b.directory
                            && a.size == b.size
                    });
                if self.tree_trusted || agrees {
                    self.entries = entries;
                    self.tree_live = true;
                    self.tree_trusted = true;
                } else if !self.tree_alignment_attempted {
                    self.tree_alignment_attempted = true;
                    self.request(Action::List(self.path.clone()));
                }
            }
            crate::remote_tree_watch::Update::Fallback => self.tree_live = false,
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, hosts: &[Host], editor_font_size: u8) {
        self.poll();
        ui.heading(tr("Remote files · SFTP"));
        ui.label(tr("Uses OpenSSH config and existing key/agent authentication. Unknown host keys are refused. Connect from a terminal first when trust or authentication needs setup."));
        if hosts.is_empty() && self.host.is_none() {
            ui.label(tr("Add an SSH host in Settings first."));
            return;
        }
        self.selected = self.selected.min(hosts.len().saturating_sub(1));
        let busy = self.busy();
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
                            self.command(control::Command::Connect(self.selected), hosts);
                        }
                        if self.host.is_some() && ui.button(tr("Disconnect")).clicked() {
                            self.command(control::Command::Disconnect, hosts);
                        }
                    });
                },
            );
        }
        if let Some(host) = &self.host {
            ui.strong(format!("{}:{}", host.alias, self.path));
            if self.tree_live {
                ui.small(tr("Live file updates"));
            }
        }
        if busy {
            ui.spinner();
            ui.label(tr("SFTP request in progress (30-second limit)…"));
        }
        if !self.error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
        }
        if let Some(message) = self.folder_status(crate::i18n::is_french()) {
            ui.label(message);
        }
        if let Some(path) = &self.downloaded {
            ui.label(format!("{} {}", tr("File downloaded."), path.display()));
        }
        if dirty {
            ui.label(tr(
                "Save or discard local edits before opening another remote file or host.",
            ));
        }
        let mut action = None;
        let mut terminal_path = None;
        let mut download_entry = None;
        let mut compare = None;
        let mut download = None;
        let mut git_request = None;
        let mut git_diff = None;
        ui.add_enabled_ui(
            !busy && !dirty && self.rename.is_none() && self.delete.is_none(),
            |ui| {
                ui.horizontal(|ui| {
                    if self.host.is_some() && ui.button(tr("Refresh")).clicked() {
                        self.tree_alignment_attempted = false;
                        action = Some(Action::List(self.path.clone()));
                    }
                    if self.host.is_some()
                        && !self.path.is_empty()
                        && ui.button(tr("Open a terminal here")).clicked()
                    {
                        terminal_path = Some(self.path.clone());
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
                                if ui.button(tr("Download…")).clicked() {
                                    download_entry = Some(entry.path.clone());
                                    ui.close();
                                }
                                if ui.button(tr("Open a terminal here")).clicked() {
                                    terminal_path = Some(entry.path.clone());
                                    ui.close();
                                }
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
        if let (Some(host), Some(path)) = (&self.host, download_entry) {
            self.command(
                control::Command::DownloadEntry {
                    host: host.alias.clone(),
                    path,
                },
                hosts,
            );
        }
        if let (Some(host), Some(path)) = (&self.host, terminal_path) {
            self.command(
                control::Command::TerminalHere {
                    host: host.alias.clone(),
                    path,
                },
                hosts,
            );
        }
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
            if ui.small_button(crate::i18n::tr("Copy path")).clicked() {
                ui.ctx().copy_text(doc.path.clone());
            }
            let fr = crate::i18n::is_french();
            let ready = !busy && doc.git_history.pending.is_none();
            if ui
                .add_enabled(
                    ready,
                    egui::Button::new(crate::file_timeline::Source::Git.label(fr)),
                )
                .clicked()
            {
                git_request = Some((doc.path.clone(), false));
            }
            if doc.git_history.pending.is_some() {
                ui.spinner();
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(80));
            }
            if let Some(page) = &doc.git_history.page {
                if page.uncommitted
                    && ui
                        .add_enabled(
                            ready,
                            egui::Button::new(if fr {
                                "Modifications non commitées"
                            } else {
                                "Uncommitted Changes"
                            }),
                        )
                        .clicked()
                {
                    git_diff = Some((doc.path.clone(), None));
                }
                if !page.uncommitted && page.commits.is_empty() {
                    ui.label(if fr { "Aucun commit." } else { "No commits." });
                }
                egui::ScrollArea::vertical()
                    .id_salt("ssh-commits")
                    .max_height(180.)
                    .show(ui, |ui| {
                        for row in &page.commits {
                            if ui
                                .add_enabled(
                                    ready,
                                    egui::Button::new(
                                        row.message.lines().next().unwrap_or(&row.id),
                                    ),
                                )
                                .on_hover_text(row.details(fr))
                                .clicked()
                            {
                                git_diff = Some((doc.path.clone(), Some(row.id.clone())));
                            }
                        }
                    });
                if page.more
                    && ui
                        .add_enabled(
                            ready,
                            egui::Button::new(if fr { "Charger la suite" } else { "Load More" }),
                        )
                        .clicked()
                {
                    git_request = Some((doc.path.clone(), true));
                }
            }
            if !doc.versions.is_empty() {
                ui.collapsing(
                    crate::file_timeline::Source::Local.label(crate::i18n::is_french()),
                    |ui| {
                        for version in &doc.versions {
                            let date = crate::regional_date::long_local(
                                version.timestamp,
                                crate::i18n::is_french(),
                            )
                            .unwrap_or_default();
                            if ui.add_enabled(!busy, egui::Button::new(date)).clicked() {
                                compare = Some((doc.path.clone(), version.id.clone()));
                            }
                        }
                    },
                );
            }
            if let Some(lines) = &doc.comparison.lines {
                if ui
                    .small_button(if crate::i18n::is_french() {
                        "Fermer la comparaison"
                    } else {
                        "Close comparison"
                    })
                    .clicked()
                {
                    doc.comparison.lines = None;
                } else if lines.is_empty() {
                    ui.label(if crate::i18n::is_french() {
                        "Aucune différence."
                    } else {
                        "No differences."
                    });
                } else {
                    let height = ui.text_style_height(&egui::TextStyle::Monospace);
                    egui::ScrollArea::both()
                        .id_salt("remote-history-diff")
                        .max_height(200.)
                        .show_rows(ui, height, lines.len(), |ui, range| {
                            for i in range {
                                ui.monospace(&lines[i]);
                            }
                        });
                }
            }
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
                    doc.discard_text();
                }
                if ui
                    .add_enabled(!busy, egui::Button::new(tr("Download original…")))
                    .clicked()
                {
                    download = Some(doc.path.clone());
                }
            });
            if !doc.preview_error.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &doc.preview_error);
            }
            if let Some(preview) = &mut doc.preview {
                if let Some(source) = &preview.playback {
                    source.show(ui);
                } else if let Some(office) = &mut preview.office {
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
                        crate::files::show_fitted_image(ui, texture).context_menu(|ui| {
                            if ui
                                .add_enabled(!busy, egui::Button::new(tr("Download original…")))
                                .clicked()
                            {
                                download = Some(doc.path.clone());
                                ui.close();
                            }
                        });
                    }
                }
            } else if let Some(text) = &mut doc.text {
                ui.add_enabled_ui(!busy, |ui| {
                    egui::ScrollArea::both().show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(text)
                                .code_editor()
                                .font(egui::FontId::monospace(f32::from(editor_font_size)))
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
        if let Some(path) = download {
            self.download_original(&path);
        }
        if let Some((path, id)) = git_diff {
            self.compare_git(&path, id.as_deref());
        }
        if let Some((path, more)) = git_request {
            self.load_git_history(&path, more);
        }
        if let Some((path, id)) = compare {
            self.compare_version(&path, &id);
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
    fn removing_a_saved_target_only_drops_its_own_files_connection() {
        let host = Host {
            alias: "lab".into(),
            folder: "/work".into(),
            ..Host::default()
        };
        let profile = std::path::PathBuf::from("isolated-profile");
        let mut panel = RemotePanel::with_profile(Some(profile.clone()));
        panel.host = Some(host.clone());
        panel.selected = 1;
        panel.path = "/work/notes".into();
        let (sender, receiver) = std::sync::mpsc::channel();
        panel.pending = Some(receiver);
        let other = Host {
            folder: "/other".into(),
            ..host.clone()
        };
        panel.saved_target_removed(0, &other, 1);
        assert_eq!(panel.selected, 0);
        assert!(panel.pending.is_some());
        assert_eq!(panel.path, "/work/notes");
        // Changing terminal preferences never changes which Files connection belongs to the row.
        let updated = Host {
            host_daemon: !host.host_daemon,
            ..host
        };
        panel.saved_target_removed(0, &updated, 0);
        assert!(panel.host.is_none());
        assert!(panel.path.is_empty());
        assert!(panel.pending.is_none());
        assert!(sender.send(Err("late reply".into())).is_err());
        assert_eq!(panel.profile, Some(profile));
    }
    #[cfg(feature = "visual-test")]
    #[test]
    fn remote_image_right_click_opens_compatibility_popup() {
        let ctx = egui::Context::default();
        let mut panel = RemotePanel::fixture();
        let mut frame = |events: Vec<egui::Event>| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200., 1000.),
                    )),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| panel.show(ui, &[], 12));
                },
            )
        };
        frame(vec![]);
        let out = frame(vec![]);
        let meshes = ctx.tessellate(out.shapes, out.pixels_per_point);
        let position = meshes
            .iter()
            .find_map(|clipped| match &clipped.primitive {
                egui::epaint::Primitive::Mesh(mesh)
                    if mesh.texture_id != egui::TextureId::default() =>
                {
                    Some(mesh.calc_bounds().intersect(clipped.clip_rect).center())
                }
                _ => None,
            })
            .expect("image pixels");
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        assert!(egui::Popup::is_any_open(&ctx));
    }
    #[test]
    fn remote_version_compare_reads_current_host_and_preserves_draft() {
        let temp = tempfile::tempdir().unwrap();
        let path = "/root/note.txt";
        let versions = crate::local_file_history::remote_versions(
            temp.path(),
            "host",
            path,
            Some("saved version\n"),
        )
        .unwrap();
        let id = versions[0].id.clone();
        let mut panel = RemotePanel {
            profile: Some(temp.path().into()),
            host: Some(Host {
                alias: "host".into(),
                ..Default::default()
            }),
            root: "/root".into(),
            document: Some(RemoteDocument {
                path: path.into(),
                versions,
                comparison: Default::default(),
                git_history: Default::default(),
                html_page: None,
                html_plain: None,
                image_menu: None,
                original: b"opened earlier".to_vec(),
                text: Some("unsaved editor draft".into()),
                preview: None,
                preview_error: String::new(),
            }),
            ..Default::default()
        };
        panel.compare_version_with(path, "../missing", |_, _| {
            panic!("unlisted version must not read SSH")
        });
        assert!(!panel.busy());
        panel.compare_version_with(path, &id, move |host, action| {
            assert_eq!(host.alias, "host");
            assert_eq!(host.folder, "/root");
            assert!(matches!(action,Action::Read(p) if p==path));
            Ok(Reply::File {
                root: "/root".into(),
                path: path.into(),
                bytes: b"remote now\n".to_vec(),
            })
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while panel.busy() {
            assert!(std::time::Instant::now() < deadline);
            panel.poll();
            std::thread::yield_now();
        }
        let snapshot = panel.snapshot(&[]);
        let diff = snapshot.comparison.unwrap().join("\n");
        assert!(diff.contains("-saved version") && diff.contains("+remote now"));
        assert!(snapshot.dirty && !diff.contains("editor draft"));
        assert_eq!(panel.document.as_ref().unwrap().original, b"opened earlier");
        panel.command(control::Command::CloseComparison, &[]);
        assert!(panel.snapshot(&[]).comparison.is_none());
        assert_eq!(
            panel.document.as_ref().unwrap().text.as_deref(),
            Some("unsaved editor draft")
        );
    }

    #[test]
    fn remote_saved_history_requires_ack_and_reopens_by_host_and_exact_path() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path();
        let path = "/root/Notes.txt";
        let host = Host {
            alias: "alpha".into(),
            ..Host::default()
        };
        let save = |bytes: &[u8]| Action::Save {
            path: path.into(),
            original: Vec::new(),
            replacement: bytes.to_vec(),
        };
        assert!(
            execute_for_panel(Some(profile), host.clone(), save(b"rejected"), |_, _| Err(
                "conflict".into()
            ))
            .is_err()
        );
        assert!(!profile.join("History").exists());
        let ok = |_, _| Ok(Reply::Saved);
        let (_, _, history) =
            execute_for_panel(Some(profile), host.clone(), save(b"first"), ok).unwrap();
        let first = history.unwrap().remove(0).id;
        let (_, _, history) =
            execute_for_panel(Some(profile), host.clone(), save(b"first"), ok).unwrap();
        assert_eq!(history.unwrap().len(), 1);
        let (_, _, history) =
            execute_for_panel(Some(profile), host.clone(), save(b"second"), ok).unwrap();
        let versions = history.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[1].id, first);
        let (_, _, history) = execute_for_panel(
            Some(profile),
            host.clone(),
            save(&vec![b'x'; crate::local_file_history::MAX_BYTES + 1]),
            ok,
        )
        .unwrap();
        assert_eq!(history.unwrap().len(), 2);
        for (alias, file, count) in [
            ("alpha", path, 2),
            ("alpha", "/root/notes.txt", 0),
            ("beta", path, 0),
        ] {
            let host = Host {
                alias: alias.into(),
                ..Host::default()
            };
            let (_, _, history) =
                execute_for_panel(Some(profile), host, Action::Read(file.into()), |_, _| {
                    Ok(Reply::File {
                        root: "/root".into(),
                        path: file.into(),
                        bytes: b"current".to_vec(),
                    })
                })
                .unwrap();
            assert_eq!(history.unwrap().len(), count);
        }
    }

    #[cfg(windows)]
    #[test]
    fn playback_ssh_reply_opens_the_player_in_both_renderers_even_for_text_like_bytes() {
        let host = Host {
            alias: "fixture".into(),
            ..Default::default()
        };
        let path = "/media/clip.mp4".to_string();
        let bytes = b"broken movie data".to_vec();
        let (tx, rx) = mpsc::channel();
        tx.send(Ok((
            Reply::File {
                root: "/media".into(),
                path: path.clone(),
                bytes: bytes.clone(),
            },
            prepare_preview(&path, &bytes),
            Ok(vec![]),
        )))
        .unwrap();
        let mut panel = RemotePanel {
            host: Some(host.clone()),
            pending: Some(rx),
            ..Default::default()
        };
        panel.poll();
        assert!(panel.document.as_ref().unwrap().text.is_none());
        assert!(
            panel
                .snapshot(std::slice::from_ref(&host))
                .playback
                .is_some()
        );
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| panel.show(ui, std::slice::from_ref(&host), 12));
        });
        assert!(
            crate::files::playback::Source::take_presented(&ctx)
                .unwrap()
                .1
                .is_positive()
        );
    }

    #[test]
    fn remote_media_keeps_bytes_and_removes_temporary_files() {
        for (path, bytes) in [
            (
                "/server/recording.WAV",
                &include_bytes!("../tests/fixtures/player-silence.wav")[..],
            ),
            (
                "/server/clip.WEBM",
                &include_bytes!("../tests/fixtures/player-blue.webm")[..],
            ),
        ] {
            let doc = prepare_preview(path, bytes).unwrap().unwrap();
            assert!(doc.playback.is_some() && doc.media && !doc.dirty());
            assert!(!doc.path.exists());
        }
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
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 2"><rect width="2" height="2" fill="red" opacity="0.5"/></svg>"#;
        let view = prepare_preview("/server/image.SVG", svg).unwrap().unwrap();
        let image = view.image.unwrap();
        assert_eq!(image.size, [4, 2]);
        assert_eq!(image.pixels[0].to_array(), [128, 0, 0, 128]);
        assert_eq!(image.pixels[3].to_array(), [0; 4]);
        assert!(!view.path.exists());
        let avif = include_bytes!("../tests/fixtures/alpha-bars.avif");
        let view = prepare_preview("/server/image.AVIF", avif)
            .unwrap()
            .unwrap();
        assert_eq!(view.image.unwrap().size, [64, 48]);
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
                versions: Vec::new(),
                comparison: Default::default(),
                git_history: Default::default(),
                html_page: None,
                html_plain: None,
                image_menu: None,
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
        sender
            .send(Ok((Reply::Saved, None, Ok(Vec::new()))))
            .unwrap();
        panel.poll();
        assert!(!panel.busy());
        assert!(!panel.document.as_ref().unwrap().dirty());
    }
}
