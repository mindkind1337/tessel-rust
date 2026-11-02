use crate::{
    files::{self, Document, Entry},
    launch::{self, Launch},
    terminal::Terminal,
    workspace::{self, Host, Store, Task, Workspace},
};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

struct Session {
    id: u64,
    project: Option<u64>,
    title: String,
    launch: Launch,
    cwd: PathBuf,
    terminal: Option<Terminal>,
    loading: Option<Receiver<Result<Terminal, String>>>,
    error: String,
}
impl Session {
    fn start(&mut self) {
        if self.terminal.is_some() || self.loading.is_some() {
            return;
        }
        let launch = self.launch.clone();
        let cwd = self.cwd.clone();
        let (sender, receiver) = mpsc::channel();
        self.loading = Some(receiver);
        self.error.clear();
        thread::spawn(move || {
            let result = launch::command(&launch, &cwd)
                .and_then(|c| Terminal::spawn_command(c).map_err(|e| e.to_string()));
            let _ = sender.send(result);
        });
    }
    fn poll(&mut self) {
        if let Some(receiver) = &self.loading
            && let Ok(result) = receiver.try_recv()
        {
            self.loading = None;
            match result {
                Ok(t) => self.terminal = Some(t),
                Err(e) => self.error = e,
            }
        }
        if let Some(t) = &mut self.terminal {
            t.poll();
        }
    }
}
enum Message {
    Folder(Option<PathBuf>),
    Directory(PathBuf, Result<Vec<Entry>, String>),
    Document(PathBuf, Result<Box<Document>, String>),
}
#[derive(PartialEq)]
enum View {
    Work,
    Files,
    Tasks,
    Settings,
    Git,
    Chat,
    Share,
    #[cfg(windows)]
    Browser,
    #[cfg(windows)]
    Vault,
}
pub struct Tessel {
    state: Workspace,
    store: Option<Store>,
    error: String,
    sessions: Vec<Session>,
    next_session: u64,
    active_session: Option<u64>,
    documents: Vec<Document>,
    document_pending: std::collections::HashSet<PathBuf>,
    active_document: usize,
    view: View,
    directory: Option<PathBuf>,
    entries: Vec<Entry>,
    directory_loading: bool,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    agents: Vec<(String, bool)>,
    host_alias: String,
    host_folder: String,
    task_title: String,
    pending_close: Option<usize>,
    pending_exit: bool,
    allow_exit: bool,
    git_output: String,
    git_pending: Option<Receiver<Result<String, String>>>,
    git_path: String,
    commit_message: String,
    chats: Vec<crate::chat::Chat>,
    share: crate::share_panel::SharePanel,
    #[cfg(windows)]
    browser: crate::browser::Browser,
    #[cfg(windows)]
    vault: crate::vault_panel::VaultPanel,
}
impl Tessel {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let override_dir = std::env::args()
            .collect::<Vec<_>>()
            .windows(2)
            .find(|pair| pair[0] == "--data-dir")
            .map(|pair| PathBuf::from(&pair[1]));
        let opened = override_dir
            .or_else(|| {
                directories::ProjectDirs::from("com", "Tessel", "TesselRust")
                    .map(|d| d.data_local_dir().to_owned())
            })
            .ok_or_else(|| "Cannot locate app data.".to_string())
            .and_then(|dir| Store::open(&dir));
        let (store, state, error) = match opened {
            Ok((s, w)) => (Some(s), w, String::new()),
            Err(e) => (None, Workspace::default(), e),
        };
        cc.egui_ctx.set_visuals(if state.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        let mut app = Self {
            state,
            store,
            error,
            sessions: vec![],
            next_session: 1,
            active_session: None,
            documents: vec![],
            document_pending: Default::default(),
            active_document: 0,
            view: View::Work,
            directory: None,
            entries: vec![],
            directory_loading: false,
            sender,
            receiver,
            agents: vec![],
            host_alias: String::new(),
            host_folder: String::new(),
            task_title: String::new(),
            pending_close: None,
            pending_exit: false,
            allow_exit: false,
            git_output: String::new(),
            git_pending: None,
            git_path: String::new(),
            commit_message: String::new(),
            chats: Vec::new(),
            share: crate::share_panel::SharePanel::default(),
            #[cfg(windows)]
            browser: crate::browser::Browser::default(),
            #[cfg(windows)]
            vault: crate::vault_panel::VaultPanel::default(),
        };
        for saved in &app.state.sessions {
            app.next_session = app.next_session.max(saved.id + 1);
            app.sessions.push(Session {
                id: saved.id,
                project: saved.project,
                title: saved.title.clone(),
                launch: saved.launch.clone(),
                cwd: saved.cwd.clone(),
                terminal: None,
                loading: None,
                error: String::new(),
            });
        }
        app.detect_agents();
        app.refresh_project();
        #[cfg(feature = "visual-test")]
        if let Some(path) = std::env::args()
            .collect::<Vec<_>>()
            .windows(2)
            .find(|a| a[0] == "--preview-file")
            .map(|a| PathBuf::from(&a[1]))
            && let Some(root) = path.parent()
        {
            match Document::open(root, &path) {
                Ok(mut document) => {
                    if let Some(pdf) = &mut document.pdf
                        && let Err(e) = pdf.prime(&cc.egui_ctx)
                    {
                        app.error = e;
                    }
                    app.documents.push(document);
                    app.view = View::Files;
                }
                Err(error) => app.error = error,
            }
        }
        app
    }
    fn save(&mut self) {
        self.state.sessions = self
            .sessions
            .iter()
            .map(|s| workspace::SavedSession {
                id: s.id,
                project: s.project,
                title: s.title.clone(),
                launch: s.launch.clone(),
                cwd: s.cwd.clone(),
            })
            .collect();
        if let Some(s) = &self.store
            && let Err(e) = s.save(&self.state)
        {
            self.error = e;
        }
    }
    fn detect_agents(&mut self) {
        self.agents = launch::AGENTS
            .iter()
            .map(|n| (n.to_string(), launch::installed(n)))
            .collect();
    }
    fn choose_project(&self) {
        let sender = self.sender.clone();
        thread::spawn(move || {
            let _ = sender.send(Message::Folder(
                rfd::FileDialog::new()
                    .set_title("Open project folder")
                    .pick_folder(),
            ));
        });
    }
    fn list_directory(&mut self, path: PathBuf) {
        let Some(root) = self.state.active().map(|p| p.path.clone()) else {
            return;
        };
        self.directory = Some(path.clone());
        self.directory_loading = true;
        self.entries.clear();
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = files::list(&root, &path);
            let _ = sender.send(Message::Directory(path, result));
        });
    }
    fn refresh_project(&mut self) {
        if let Some(p) = self.state.active() {
            self.list_directory(p.path.clone());
        } else {
            self.directory = None;
            self.entries.clear();
        }
    }
    fn open_document(&mut self, path: PathBuf) {
        let Some(root) = self.state.active().map(|p| p.path.clone()) else {
            return;
        };
        self.open_document_at(root, path);
    }
    fn open_document_at(&mut self, root: PathBuf, path: PathBuf) {
        if let Some(i) = self.documents.iter().position(|d| d.path == path) {
            self.active_document = i;
            self.view = View::Files;
            return;
        }
        if self.document_pending.contains(&path) {
            return;
        }
        if self.documents.len() + self.document_pending.len() >= 24 {
            self.error = "Close a document before opening more (24-tab limit)".into();
            return;
        }
        self.document_pending.insert(path.clone());
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = Document::open(&root, &path).map(Box::new);
            let _ = sender.send(Message::Document(path, result));
        });
    }
    fn poll(&mut self) {
        let mut chat_metadata_changed = false;
        for chat in &mut self.chats {
            chat.poll();
            while let Some(call) = chat.tool_calls.pop_front() {
                let result = crate::agent_tools::execute(
                    &mut self.state,
                    chat.project,
                    &call.name,
                    &call.arguments,
                );
                let result = result.map(|(value, changed)| {
                    chat_metadata_changed |= changed;
                    value
                });
                chat.tool_result(call.id, result);
            }
            if let Some(id) = &chat.thread_id
                && self.state.chat_threads.get(&chat.project) != Some(id)
            {
                self.state.chat_threads.insert(chat.project, id.clone());
                chat_metadata_changed = true;
            }
        }
        if chat_metadata_changed {
            self.save();
        }
        for s in &mut self.sessions {
            s.poll();
        }
        while let Ok(message) = self.receiver.try_recv() {
            match message {
                Message::Folder(Some(path)) => match self.state.add_project(&path) {
                    Ok(_) => {
                        self.save();
                        self.refresh_project();
                    }
                    Err(e) => self.error = e,
                },
                Message::Folder(None) => {}
                Message::Directory(path, result) if self.directory.as_ref() == Some(&path) => {
                    self.directory_loading = false;
                    match result {
                        Ok(entries) => self.entries = entries,
                        Err(e) => self.error = e,
                    }
                }
                Message::Directory(_, _) => {}
                Message::Document(path, result) => {
                    self.document_pending.remove(&path);
                    match result {
                        Ok(d) => {
                            let d = *d;
                            if let Some(i) =
                                self.documents.iter().position(|old| old.path == d.path)
                            {
                                self.active_document = i;
                            } else {
                                self.documents.push(d);
                                self.active_document = self.documents.len() - 1;
                            }
                            self.view = View::Files;
                        }
                        Err(e) => self.error = e,
                    }
                }
            }
        }
    }
    fn new_session(&mut self, launch: Launch) {
        if self.sessions.len() >= 100 {
            self.error = "Session limit: 100".into();
            return;
        }
        let cwd = self
            .state
            .active()
            .map(|p| p.path.clone())
            .unwrap_or_else(launch::default_directory);
        let title = match &launch {
            Launch::Shell => "Terminal".into(),
            Launch::Agent(n) => n.clone(),
            Launch::Ssh(h) => format!("SSH · {}", h.alias),
        };
        let id = self.next_session;
        self.next_session += 1;
        let mut s = Session {
            id,
            project: self.state.active_project,
            title,
            launch,
            cwd,
            terminal: None,
            loading: None,
            error: String::new(),
        };
        s.start();
        self.sessions.push(s);
        self.active_session = Some(id);
        self.view = View::Work;
        self.save();
    }
    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("projects")
            .default_width(230.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Projects");
                    if ui.button("+").on_hover_text("Open folder").clicked() {
                        self.choose_project();
                    }
                });
                let mut selected = None;
                for p in &self.state.projects {
                    if ui
                        .selectable_label(Some(p.id) == self.state.active_project, &p.name)
                        .on_hover_text(p.path.display().to_string())
                        .clicked()
                    {
                        selected = Some(p.id);
                    }
                }
                if let Some(id) = selected {
                    self.state.active_project = Some(id);
                    self.save();
                    self.refresh_project();
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.strong("Files");
                    if ui.small_button("Refresh").clicked()
                        && let Some(path) = self.directory.clone()
                    {
                        self.list_directory(path);
                    }
                });
                if self.directory_loading {
                    ui.spinner();
                }
                if let (Some(dir), Some(root)) = (
                    self.directory.clone(),
                    self.state.active().map(|p| p.path.clone()),
                ) {
                    if dir != root
                        && let Some(parent) = dir.parent()
                        && ui.button(".. Parent folder").clicked()
                    {
                        self.list_directory(parent.to_owned());
                    }
                    ui.small(dir.file_name().unwrap_or_default().to_string_lossy());
                }
                let mut open = None;
                egui::ScrollArea::vertical()
                    .id_salt("files-list")
                    .show_rows(ui, 22.0, self.entries.len(), |ui, range| {
                        for i in range {
                            let e = &self.entries[i];
                            if ui
                                .selectable_label(
                                    false,
                                    format!("{} {}", if e.directory { ">" } else { "·" }, e.name),
                                )
                                .clicked()
                            {
                                open = Some((e.path.clone(), e.directory));
                            }
                        }
                    });
                if let Some((path, dir)) = open {
                    if dir {
                        self.list_directory(path);
                    } else {
                        self.open_document(path);
                    }
                }
            });
    }
    fn work(&mut self, ui: &mut egui::Ui) {
        let indices: Vec<usize> = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| s.project == self.state.active_project)
            .map(|(i, _)| i)
            .collect();
        if indices.is_empty() {
            ui.heading("Start working");
            ui.label("Open a project, then launch a terminal or installed agent. Switching projects keeps sessions running.");
            if ui.button("Open terminal").clicked() {
                self.new_session(Launch::Shell);
            }
            return;
        }
        if !indices
            .iter()
            .any(|&i| Some(self.sessions[i].id) == self.active_session)
        {
            self.active_session = Some(self.sessions[indices[0]].id);
        }
        let mut close = None;
        ui.horizontal_wrapped(|ui| {
            for &i in &indices {
                let s = &self.sessions[i];
                if ui
                    .selectable_label(
                        Some(s.id) == self.active_session,
                        format!("{} #{}", s.title, s.id),
                    )
                    .clicked()
                {
                    self.active_session = Some(s.id);
                }
            }
        });
        let columns = self.state.columns;
        let visible: Vec<usize> = if columns == 1 {
            indices
                .into_iter()
                .filter(|&i| Some(self.sessions[i].id) == self.active_session)
                .collect()
        } else {
            indices
        };
        let rows = visible.len().div_ceil(columns);
        let height = ((ui.available_height() - 8.0 * rows as f32) / rows as f32).max(100.0);
        for row in visible.chunks(columns) {
            ui.columns(columns, |uis| {
                for (column, &index) in row.iter().enumerate() {
                    let s = &mut self.sessions[index];
                    let ui = &mut uis[column];
                    ui.push_id(s.id, |ui| {
                        ui.set_height(height);
                        ui.horizontal(|ui| {
                            ui.strong(&s.title);
                            if ui.small_button("Sleep").clicked() {
                                s.terminal = None;
                                s.loading = None;
                            }
                            if ui.small_button("×").clicked() {
                                close = Some(s.id);
                            }
                        });
                        if !s.error.is_empty() {
                            ui.colored_label(egui::Color32::LIGHT_RED, &s.error);
                        }
                        if let Some(t) = &mut s.terminal {
                            t.show(ui, self.state.font_size);
                        } else if s.loading.is_some() {
                            ui.spinner();
                            ui.label("Starting…");
                        } else if ui.button("Wake / reconnect").clicked() {
                            s.start();
                        }
                    });
                }
            });
        }
        if let Some(id) = close {
            self.sessions.retain(|s| s.id != id);
            self.save();
        }
    }
    fn editor(&mut self, ui: &mut egui::Ui) {
        let mut close = None;
        ui.horizontal_wrapped(|ui| {
            for (i, d) in self.documents.iter().enumerate() {
                if ui
                    .selectable_label(
                        i == self.active_document,
                        format!(
                            "{}{}",
                            d.path.file_name().unwrap_or_default().to_string_lossy(),
                            if d.dirty() { " *" } else { "" }
                        ),
                    )
                    .on_hover_text(d.path.display().to_string())
                    .clicked()
                {
                    self.active_document = i;
                }
            }
        });
        let Some(d) = self.documents.get_mut(self.active_document) else {
            ui.label("Choose a UTF-8 file in the explorer.");
            return;
        };
        let extension = d
            .path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        let readable = matches!(extension.as_str(), "html" | "htm" | "md" | "markdown");
        ui.horizontal(|ui| {
            if !d.media
                && (ui.button("Save (Ctrl+S)").clicked()
                    || ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S)))
            {
                d.error = d.save().err().unwrap_or_default();
            }
            if readable {
                ui.selectable_value(&mut d.preview_mode, false, "Source");
                ui.selectable_value(&mut d.preview_mode, true, "Readable preview");
            }
            if ui.button("Close").clicked() {
                close = Some(self.active_document);
            }
            ui.small(d.path.display().to_string());
        });
        if !d.error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &d.error);
        }
        let mut link = None;
        if let Some(pdf) = &mut d.pdf {
            pdf.show(ui);
        } else if d.media {
            if d.texture.is_none()
                && let Some(image) = d.image.take()
            {
                d.texture = Some(ui.ctx().load_texture(
                    d.path.to_string_lossy(),
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
            if let Some(texture) = &d.texture {
                egui::ScrollArea::both()
                    .id_salt((&d.path, "image"))
                    .show(ui, |ui| {
                        ui.add(
                            egui::Image::new(texture)
                                .max_width(ui.available_width())
                                .maintain_aspect_ratio(true),
                        );
                    });
            }
        } else if readable && d.preview_mode {
            ui.small("Native readable view · scripts, CSS and external resources are not loaded");
            let preview = d.preview.get_or_insert_with(|| {
                crate::preview::Preview::parse(
                    &d.text,
                    matches!(extension.as_str(), "md" | "markdown"),
                )
            });
            if ui.button("Copy plain text").clicked() {
                ui.ctx().copy_text(preview.plain());
            }
            egui::ScrollArea::vertical()
                .id_salt((&d.path, "preview"))
                .show(ui, |ui| {
                    link = preview.show(ui);
                });
        } else {
            egui::ScrollArea::both()
                .id_salt((&d.path, "source"))
                .show(ui, |ui| {
                    if ui
                        .add(
                            egui::TextEdit::multiline(&mut d.text)
                                .code_editor()
                                .font(egui::FontId::monospace(self.state.font_size))
                                .desired_width(f32::INFINITY)
                                .desired_rows(30)
                                .lock_focus(true),
                        )
                        .changed()
                    {
                        d.preview = None;
                    }
                });
        }
        let resolved = link.map(|href| {
            crate::preview::resolve(&d.root, &d.path, &href).map(|link| (d.root.clone(), link))
        });
        if let Some(result) = resolved {
            match result {
                Ok((root, crate::preview::Link::File(path))) => self.open_document_at(root, path),
                Ok((_, crate::preview::Link::Web(url))) => {
                    #[cfg(windows)]
                    {
                        self.browser.open_address(url);
                        self.view = View::Browser;
                    }
                    #[cfg(not(windows))]
                    {
                        self.error = format!("Browser is not available on this platform: {url}");
                    }
                }
                Err(error) => self.error = error,
            }
        }
        if let Some(i) = close {
            if self.documents[i].dirty() {
                self.pending_close = Some(i);
            } else {
                self.documents.remove(i);
                self.active_document = self
                    .active_document
                    .min(self.documents.len().saturating_sub(1));
            }
        }
    }
    fn tasks(&mut self, ui: &mut egui::Ui) {
        let Some(project) = self.state.active_project else {
            ui.label("Select a project first.");
            return;
        };
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.task_title);
            if ui.button("Add task").clicked() && !self.task_title.trim().is_empty() {
                let id = self.state.id();
                self.state.tasks.push(Task {
                    id,
                    project,
                    title: self.task_title.trim().chars().take(1000).collect(),
                    column: 0,
                });
                self.task_title.clear();
                self.save();
            }
        });
        let mut movement = None;
        ui.columns(4, |uis| {
            for (column, title) in ["To do", "Doing", "Review", "Done"].iter().enumerate() {
                let ui = &mut uis[column];
                ui.heading(*title);
                for task in self
                    .state
                    .tasks
                    .iter()
                    .filter(|t| t.project == project && t.column == column)
                {
                    ui.push_id(task.id, |ui| {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.label(&task.title);
                            ui.horizontal(|ui| {
                                if column > 0 && ui.small_button("←").clicked() {
                                    movement = Some((task.id, column - 1));
                                }
                                if column < 3 && ui.small_button("→").clicked() {
                                    movement = Some((task.id, column + 1));
                                }
                            });
                        });
                    });
                }
            }
        });
        if let Some((id, column)) = movement {
            if let Some(t) = self.state.tasks.iter_mut().find(|t| t.id == id) {
                t.column = column;
            }
            self.save();
        }
    }
    fn settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Appearance");
        let mut changed = ui.checkbox(&mut self.state.dark, "Dark theme").changed();
        changed |= ui
            .add(egui::Slider::new(&mut self.state.font_size, 10.0..=28.0).text("Font size"))
            .changed();
        if changed {
            ui.ctx().set_visuals(if self.state.dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            self.save();
        }
        ui.separator();
        ui.heading("SSH hosts");
        ui.label("Uses OpenSSH and your SSH config. Authentication and host verification remain with OpenSSH.");
        ui.horizontal(|ui| {
            ui.label("Alias / user@host");
            ui.text_edit_singleline(&mut self.host_alias);
        });
        ui.horizontal(|ui| {
            ui.label("Remote folder (optional)");
            ui.text_edit_singleline(&mut self.host_folder);
        });
        if ui.button("Save host").clicked() {
            match workspace::validate_host(&self.host_alias) {
                Ok(()) => {
                    self.state.hosts.push(Host {
                        alias: self.host_alias.clone(),
                        folder: self.host_folder.clone(),
                    });
                    self.host_alias.clear();
                    self.host_folder.clear();
                    self.save();
                }
                Err(e) => self.error = e,
            }
        }
        let mut connect = None;
        for host in &self.state.hosts {
            if ui.button(format!("Connect {}", host.alias)).clicked() {
                connect = Some(host.clone());
            }
        }
        if let Some(h) = connect {
            self.new_session(Launch::Ssh(h));
        }
        ui.separator();
        ui.heading("Agent CLIs");
        if ui.button("Refresh detection").clicked() {
            self.detect_agents();
        }
        for (name, installed) in &self.agents {
            ui.label(format!(
                "{name}: {}",
                if *installed {
                    "available"
                } else {
                    "not on PATH"
                }
            ));
        }
        ui.separator();
        ui.small("Local Rust workspace. No import from Electron Tessel. Processes start only by your clicks.");
    }
    fn confirmations(&mut self, ctx: &egui::Context) {
        if let Some(index) = self.pending_close {
            egui::Modal::new(egui::Id::new("unsaved-file")).show(ctx, |ui| {
                ui.label("Save changes before closing this file?");
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.pending_close = None;
                    }
                    if ui.button("Save and close").clicked() {
                        match self.documents[index].save() {
                            Ok(()) => {
                                self.documents.remove(index);
                                self.pending_close = None;
                                self.active_document = 0;
                            }
                            Err(e) => self.error = e,
                        }
                    }
                    if ui.button("Discard changes").clicked() {
                        self.documents.remove(index);
                        self.pending_close = None;
                        self.active_document = 0;
                    }
                });
            });
        }
        if self.pending_exit {
            egui::Modal::new(egui::Id::new("close-app")).show(ctx, |ui| {
                ui.label("There are unsaved files or sessions. Closing ends sessions.");
                if ui.button("Keep working").clicked() {
                    self.pending_exit = false;
                }
                if ui.button("Save all and close").clicked() {
                    match self
                        .documents
                        .iter_mut()
                        .filter(|d| d.dirty())
                        .try_for_each(Document::save)
                    {
                        Ok(()) => {
                            self.allow_exit = true;
                            self.save();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        Err(e) => self.error = e,
                    }
                }
                if ui.button("Discard edits and close").clicked() {
                    self.allow_exit = true;
                    self.save();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        }
    }
    fn git_start(&mut self, args: Vec<String>, input: Option<Vec<u8>>) {
        if self.git_pending.is_some() {
            return;
        }
        let Some(root) = self.state.active().map(|p| p.path.clone()) else {
            self.error = "Select a project first.".into();
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.git_pending = Some(receiver);
        thread::spawn(move || {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let _ = sender.send(crate::git::run(&root, &args, input.as_deref()));
        });
    }
    fn chat_view(&mut self, ui: &mut egui::Ui) {
        let Some(project) = self.state.active().cloned() else {
            ui.label("Select a project for native chat.");
            return;
        };
        if let Some(chat) = self.chats.iter_mut().find(|c| c.project == project.id) {
            chat.show(ui);
        } else {
            ui.heading("Native Codex chat");
            ui.label("Connect your installed Codex CLI with its existing sign-in. No API key is copied into Tessel Rust.");
            if ui.button("Create chat").clicked() {
                let mut chat = crate::chat::Chat::new(project.id, project.path);
                if let Some(id) = self.state.chat_threads.get(&project.id) {
                    chat.set_resume(id.clone());
                }
                self.chats.push(chat);
            }
        }
    }
    fn git_view(&mut self, ui: &mut egui::Ui) {
        if let Some(rx) = &self.git_pending
            && let Ok(result) = rx.try_recv()
        {
            self.git_pending = None;
            self.git_output = result.unwrap_or_else(|e| format!("Error: {e}"));
        }
        ui.heading("Source control");
        ui.label("On-demand Git operations. Output is capped at 2 MiB; operations time out after 20 seconds.");
        ui.add_enabled_ui(self.git_pending.is_none(), |ui| {
            ui.horizontal(|ui| {
                for (label, args) in [
                    (
                        "Status",
                        vec!["status", "--short", "--untracked-files=normal"],
                    ),
                    ("Diff", vec!["diff", "--no-ext-diff", "--no-textconv"]),
                    (
                        "Staged",
                        vec!["diff", "--cached", "--no-ext-diff", "--no-textconv"],
                    ),
                    ("History", vec!["log", "-30", "--oneline", "--decorate"]),
                ] {
                    if ui.button(label).clicked() {
                        self.git_start(args.into_iter().map(str::to_owned).collect(), None);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Relative file path");
                ui.text_edit_singleline(&mut self.git_path);
                if ui.button("Stage file").clicked() && !self.git_path.trim().is_empty() {
                    self.git_start(
                        vec![
                            "--literal-pathspecs".into(),
                            "add".into(),
                            "--".into(),
                            self.git_path.clone(),
                        ],
                        None,
                    );
                }
            });
            ui.horizontal(|ui| {
                ui.label("Commit message");
                ui.text_edit_singleline(&mut self.commit_message);
                if ui.button("Commit staged changes").clicked()
                    && !self.commit_message.trim().is_empty()
                {
                    self.git_start(
                        vec!["commit".into(), "-F".into(), "-".into()],
                        Some(self.commit_message.as_bytes().to_vec()),
                    );
                }
            });
        });
        if self.git_pending.is_some() {
            ui.spinner();
        }
        egui::ScrollArea::both().show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.git_output)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
        });
    }
}
impl eframe::App for Tessel {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll();
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_exit
            && (self.documents.iter().any(Document::dirty)
                || !self.sessions.is_empty()
                || self.chats.iter().any(crate::chat::Chat::busy)
                || self.share.hosting())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_exit = true;
        }
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("Tessel Rust");
                ui.selectable_value(&mut self.view, View::Work, "Work");
                ui.selectable_value(&mut self.view, View::Files, "Editor");
                ui.selectable_value(&mut self.view, View::Tasks, "Tasks");
                ui.selectable_value(&mut self.view, View::Settings, "Settings");
                ui.selectable_value(&mut self.view, View::Git, "Git");
                ui.selectable_value(&mut self.view, View::Chat, "Chat");
                ui.selectable_value(&mut self.view, View::Share, "Share");
                #[cfg(windows)]
                ui.selectable_value(&mut self.view, View::Browser, "Browser");
                #[cfg(windows)]
                ui.selectable_value(&mut self.view, View::Vault, "Passwords");
                ui.separator();
                if ui.button("+ Terminal").clicked() {
                    self.new_session(Launch::Shell);
                }
                let mut agent = None;
                ui.menu_button("+ Agent", |ui| {
                    for (name, available) in &self.agents {
                        if ui
                            .add_enabled(*available, egui::Button::new(name))
                            .clicked()
                        {
                            agent = Some(name.clone());
                            ui.close();
                        }
                    }
                });
                if let Some(a) = agent {
                    self.new_session(Launch::Agent(a));
                }
                for columns in 1..=3 {
                    if ui
                        .selectable_value(
                            &mut self.state.columns,
                            columns,
                            format!("{columns} col"),
                        )
                        .changed()
                    {
                        self.save();
                    }
                }
            });
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.small("Rust native · development");
                if self.store.is_none() {
                    ui.colored_label(egui::Color32::YELLOW, "Persistence unavailable");
                }
                if !self.error.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
                    if ui.small_button("Dismiss").clicked() {
                        self.error.clear();
                    }
                }
            });
        });
        self.sidebar(ctx);
        #[cfg(windows)]
        self.browser.visible(
            self.view == View::Browser && !self.pending_exit && self.pending_close.is_none(),
        );
        egui::CentralPanel::default().show(ctx, |ui| match self.view {
            View::Work => self.work(ui),
            View::Files => self.editor(ui),
            View::Tasks => self.tasks(ui),
            View::Settings => self.settings(ui),
            View::Git => self.git_view(ui),
            View::Chat => self.chat_view(ui),
            View::Share => self
                .share
                .show(ui, self.state.active().map(|p| p.path.as_path())),
            #[cfg(windows)]
            View::Vault => self.vault.show(ui, &mut self.browser),
            #[cfg(windows)]
            View::Browser => {
                if !self.pending_exit && self.pending_close.is_none() {
                    self.browser.show(ui, frame);
                }
            }
        });
        self.confirmations(ctx);
        ctx.request_repaint_after(Duration::from_millis(if self.sessions.is_empty() {
            150
        } else {
            33
        }));
    }
}
