use crate::i18n::tr;
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
    FileAction(Result<crate::file_actions::Change, String>),
    Folder(Option<PathBuf>),
    Directory(u64, PathBuf, Result<Vec<Entry>, String>),
    Document(PathBuf, Result<Box<Document>, String>),
}
#[derive(PartialEq)]
enum View {
    Work,
    Files,
    Chat,
    Remote,
    #[cfg(windows)]
    Browser,
}
use crate::workspace::SideTab as RightTab;
#[derive(Clone, Copy, PartialEq)]
enum Overlay {
    Settings,
    Share,
}
#[derive(Clone, Copy, PartialEq)]
enum SettingsSection {
    Appearance,
    Hosts,
    Backup,
    Agents,
    #[cfg(windows)]
    Passwords,
    #[cfg(windows)]
    Account,
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
    overlay: Option<Overlay>,
    settings_section: SettingsSection,
    command_palette: crate::palette::CommandPaletteState,
    command_palette_focused: bool,
    directory: Option<PathBuf>,
    entries: Vec<Entry>,
    directory_loading: bool,
    directory_generation: u64,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    agents: Vec<(String, bool)>,
    host_alias: String,
    host_folder: String,
    task_title: String,
    pending_close: Option<usize>,
    pending_exit: bool,
    restore_preview: Option<Workspace>,
    file_action: Option<crate::file_actions::Action>,
    file_action_pending: bool,
    backup_status: String,
    allow_exit: bool,
    #[cfg(feature = "visual-test")]
    visual_frames: usize,
    git_output: String,
    git_pending: Option<Receiver<Result<String, String>>>,
    git_path: String,
    commit_message: String,
    chats: Vec<crate::chat::Chat>,
    claude_chats: Vec<crate::claude::Chat>,
    show_claude: bool,
    share: crate::share_panel::SharePanel,
    remote: crate::remote_panel::RemotePanel,
    #[cfg(windows)]
    browser: crate::browser::Browser,
    #[cfg(windows)]
    agent_browsers: crate::agent_browser::AgentBrowsers,
    #[cfg(windows)]
    show_agent_browser: bool,
    #[cfg(windows)]
    vault: crate::vault_panel::VaultPanel,
    #[cfg(windows)]
    account: crate::account_panel::AccountPanel,
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
        #[cfg(windows)]
        crate::terminal::configure_fonts(&cc.egui_ctx);
        crate::i18n::set_language(state.language);
        crate::theme::apply(&cc.egui_ctx, state.dark, state.theme);
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
            overlay: None,
            settings_section: SettingsSection::Appearance,
            command_palette: Default::default(),
            command_palette_focused: false,
            directory: None,
            entries: vec![],
            directory_loading: false,
            directory_generation: 0,
            sender,
            receiver,
            agents: vec![],
            host_alias: String::new(),
            host_folder: String::new(),
            task_title: String::new(),
            pending_close: None,
            pending_exit: false,
            restore_preview: None,
            file_action: None,
            file_action_pending: false,
            backup_status: String::new(),
            allow_exit: false,
            #[cfg(feature = "visual-test")]
            visual_frames: 0,
            git_output: String::new(),
            git_pending: None,
            git_path: String::new(),
            commit_message: String::new(),
            chats: Vec::new(),
            claude_chats: Vec::new(),
            show_claude: false,
            share: crate::share_panel::SharePanel::default(),
            remote: crate::remote_panel::RemotePanel::default(),
            #[cfg(windows)]
            browser: crate::browser::Browser::default(),
            #[cfg(windows)]
            agent_browsers: Default::default(),
            #[cfg(windows)]
            show_agent_browser: false,
            #[cfg(windows)]
            vault: crate::vault_panel::VaultPanel::default(),
            #[cfg(windows)]
            account: Default::default(),
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
        #[cfg(feature = "visual-test")]
        {
            if let Some(id) = std::env::args()
                .collect::<Vec<_>>()
                .windows(2)
                .find(|a| a[0] == "--theme-fixture")
                .and_then(|a| crate::theme_palettes::ThemeId::from_id(&a[1]))
            {
                app.state.theme = Some(id);
                crate::theme::apply(&cc.egui_ctx, true, Some(id));
            }
            if std::env::args().any(|a| a == "--palette-fixture") {
                app.command_palette.open = true;
            }
            if std::env::args().any(|a| a == "--claude-fixture") {
                let root = std::env::current_dir().expect("fixture cwd");
                app.state.add_project(&root).expect("fixture project");
                let project = app.state.active().expect("fixture project");
                app.claude_chats.push(crate::claude::Chat::fixture(
                    project.id,
                    project.path.clone(),
                ));
                app.show_claude = true;
                app.view = View::Chat;
            }
            if std::env::args().any(|a| a == "--french-fixture") {
                app.state.language = crate::i18n::Language::French;
                crate::i18n::set_language(app.state.language);
            }
            #[cfg(windows)]
            if std::env::args().any(|a| a == "--account-fixture") {
                app.overlay = Some(Overlay::Settings);
                app.settings_section = SettingsSection::Account;
            }
            if std::env::args().any(|a| a == "--remote-fixture") {
                app.view = View::Remote;
                app.remote = crate::remote_panel::RemotePanel::fixture();
            }
            if std::env::args().any(|a| a == "--file-action-fixture") {
                let root = std::env::current_dir().expect("fixture cwd");
                app.state.add_project(&root).expect("fixture project");
                app.file_action = Some(crate::file_actions::Action {
                    kind: crate::file_actions::Kind::File,
                    root: root.clone(),
                    path: root,
                    name: "example.txt".into(),
                });
                app.view = View::Files;
            }
            if std::env::args().any(|a| a == "--questions-fixture") {
                let root = std::env::current_dir().expect("fixture cwd");
                let project = app.state.add_project(&root).expect("fixture project");
                app.chats
                    .push(crate::chat::Chat::questions_fixture(project, root));
                app.view = View::Chat;
            }
            if std::env::args().any(|a| a == "--restore-fixture") {
                app.overlay = Some(Overlay::Settings);
                app.settings_section = SettingsSection::Backup;
                app.restore_preview = Some(Workspace::default());
            }
            if std::env::args().any(|a| a == "--settings-fixture") {
                app.overlay = Some(Overlay::Settings);
            }
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
                    if let Some(office) = &mut document.office
                        && let Err(e) = office.prime()
                    {
                        document.error = e;
                    }
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
        #[cfg(feature = "visual-test")]
        if std::env::args().any(|a| a == "--terminal-fixture") {
            match Terminal::visual_fixture() {
                Ok(terminal) => {
                    let id = app.next_session;
                    app.next_session += 1;
                    app.sessions.push(Session {
                        id,
                        project: app.state.active_project,
                        title: "Terminal rendering fixture".into(),
                        launch: Launch::Shell,
                        cwd: launch::default_directory(),
                        terminal: Some(terminal),
                        loading: None,
                        error: String::new(),
                    });
                    app.active_session = Some(id);
                    app.view = View::Work;
                }
                Err(e) => app.error = e.to_string(),
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
        self.directory_generation = self.directory_generation.wrapping_add(1);
        let generation = self.directory_generation;
        self.directory = Some(path.clone());
        self.directory_loading = true;
        self.entries.clear();
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = files::list(&root, &path);
            let _ = sender.send(Message::Directory(generation, path, result));
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
    fn account_busy(&self) -> bool {
        #[cfg(windows)]
        {
            self.account.busy()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
    fn poll(&mut self) {
        self.remote.poll();
        #[cfg(windows)]
        {
            self.browser.poll();
            self.account.poll();
            self.agent_browsers.retain(&self.state.projects);
        }
        let mut chat_metadata_changed = false;
        for chat in &mut self.chats {
            chat.poll();
            #[cfg(windows)]
            self.agent_browsers.poll(chat);
            while let Some(call) = chat.tool_calls.pop_front() {
                #[cfg(windows)]
                if call.name == "tessel_browser" {
                    self.agent_browsers.dispatch(chat, call);
                    continue;
                }
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
        for chat in &mut self.claude_chats {
            chat.poll();
            if self.state.claude_threads.get(&chat.project) != chat.saved_session.as_ref() {
                if let Some(id) = &chat.saved_session {
                    self.state.claude_threads.insert(chat.project, id.clone());
                } else {
                    self.state.claude_threads.remove(&chat.project);
                }
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
                Message::FileAction(result) => {
                    self.file_action_pending = false;
                    match result {
                        Err(e) => self.error = e,
                        Ok(change) => {
                            if let Some(before) = &change.before {
                                if let Some(after) = &change.after {
                                    for doc in &mut self.documents {
                                        if let Ok(suffix) = doc.path.strip_prefix(before) {
                                            doc.path = if suffix.as_os_str().is_empty() {
                                                after.clone()
                                            } else {
                                                after.join(suffix)
                                            };
                                        }
                                    }
                                    if let Some(dir) = &self.directory
                                        && let Ok(suffix) = dir.strip_prefix(before)
                                    {
                                        self.directory = Some(after.join(suffix));
                                    }
                                } else {
                                    self.documents.retain(|doc| !doc.path.starts_with(before));
                                    self.active_document = self
                                        .active_document
                                        .min(self.documents.len().saturating_sub(1));
                                    self.pending_close = None;
                                }
                            }
                            if let Some(dir) = self.directory.clone() {
                                self.list_directory(dir);
                            }
                        }
                    }
                }

                Message::Folder(Some(path)) => match self.state.add_project(&path) {
                    Ok(_) => {
                        self.save();
                        self.refresh_project();
                    }
                    Err(e) => self.error = e,
                },
                Message::Folder(None) => {}
                Message::Directory(generation, path, result)
                    if generation == self.directory_generation
                        && self.directory.as_ref() == Some(&path) =>
                {
                    self.directory_loading = false;
                    match result {
                        Ok(entries) => self.entries = entries,
                        Err(e) => self.error = e,
                    }
                }
                Message::Directory(_, _, _) => {}
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
        let Some(next) = self.next_session.checked_add(1) else {
            self.error = "Session identifiers exhausted".into();
            return;
        };
        self.next_session = next;
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
    fn file_actions_ready(&self) -> bool {
        !self.file_action_pending
            && self.document_pending.is_empty()
            && !self.documents.iter().any(Document::dirty)
    }
    fn file_action_dialog(&mut self, ctx: &egui::Context) {
        let ready = self.file_actions_ready();
        if let Some(action) = &mut self.file_action {
            let mut submit = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("file-action")).show(ctx, |ui| {
                use crate::file_actions::Kind;
                ui.heading(tr(match action.kind {
                    Kind::File => "Create file",
                    Kind::Folder => "Create folder",
                    Kind::Rename => "Rename",
                    Kind::Delete => "Delete permanently",
                }));
                ui.label(action.path.display().to_string());
                if action.kind == Kind::Delete {
                    ui.label(tr(
                        "This does not use a recycle bin. Only empty folders can be deleted.",
                    ));
                } else {
                    ui.text_edit_singleline(&mut action.name);
                }
                if !ready {
                    ui.label(tr("Save pending edits before changing files."));
                }
                submit = ui
                    .add_enabled(ready, egui::Button::new(tr("Confirm")))
                    .clicked();
                cancel = ui.button(tr("Cancel")).clicked();
            });
            if submit && let Some(action) = self.file_action.take() {
                self.file_action_pending = true;
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let _ = sender.send(Message::FileAction(crate::file_actions::execute(action)));
                });
            }
            if cancel {
                self.file_action = None;
            }
        }
    }
    fn center_tabs(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::horizontal()
            .id_salt("center-tabs")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.view, View::Work, tr("Terminal"));
                    if !self.chats.is_empty()
                        || !self.claude_chats.is_empty()
                        || self.view == View::Chat
                    {
                        ui.selectable_value(&mut self.view, View::Chat, tr("Native chat"));
                    }
                    #[cfg(windows)]
                    ui.selectable_value(&mut self.view, View::Browser, tr("Browser"));
                    for (i, document) in self.documents.iter().enumerate() {
                        if ui
                            .selectable_label(
                                self.view == View::Files && self.active_document == i,
                                format!(
                                    "{}{}",
                                    document
                                        .path
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy(),
                                    if document.dirty() { " *" } else { "" }
                                ),
                            )
                            .on_hover_text(document.path.display().to_string())
                            .clicked()
                        {
                            self.view = View::Files;
                            self.active_document = i;
                        }
                    }
                });
            });
        ui.separator();
    }
    fn show_command_palette(&mut self, ctx: &egui::Context) {
        if !self.command_palette.open {
            self.command_palette_focused = false;
            return;
        }
        use crate::palette::{PaletteCommand, filter_palette_commands};
        let commands: Vec<_> = [
            ("terminal", "Open terminal"),
            ("chat", "Native chat"),
            ("files", "Files"),
            ("changes", "Changes"),
            ("tasks", "Tasks"),
            ("settings", "Settings"),
            ("remote", "Remote files"),
            ("share", "Share"),
            #[cfg(windows)]
            ("browser", "Browser"),
            #[cfg(windows)]
            ("vault", "Passwords"),
            #[cfg(windows)]
            ("account", "Account"),
        ]
        .into_iter()
        .map(|(id, title)| PaletteCommand::new(id, "Tessel", tr(title)))
        .collect();
        let mut selected = None;
        let response = egui::Modal::new(egui::Id::new("commands")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 40.0).clamp(240.0, 560.0));
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.command_palette.query)
                    .desired_width(f32::INFINITY)
                    .hint_text(tr("Search panes, run a command")),
            );
            if !self.command_palette_focused {
                edit.request_focus();
                self.command_palette_focused = true;
            }
            if edit.changed() {
                self.command_palette.index = 0;
            }
            let rows = filter_palette_commands(&commands, &self.command_palette.query);
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
                self.command_palette.move_selection(1, rows.len());
            }
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
                self.command_palette.move_selection(-1, rows.len());
            }
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                selected = self.command_palette.selected_id(&rows).map(str::to_owned);
            }
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .show(ui, |ui| {
                    for (i, row) in rows.iter().enumerate() {
                        if ui
                            .selectable_label(i == self.command_palette.index, &row.command.title)
                            .clicked()
                        {
                            selected = Some(row.command.id.clone());
                        }
                    }
                });
        });
        if response.should_close() {
            self.command_palette.close();
        }
        if let Some(id) = selected {
            self.command_palette.close();
            match id.as_str() {
                "terminal" => self.new_session(Launch::Shell),
                "chat" => self.view = View::Chat,
                "files" | "changes" | "tasks" => {
                    self.state.panels.right_open = true;
                    self.state.panels.right_tab = match id.as_str() {
                        "changes" => RightTab::Changes,
                        "tasks" => RightTab::Tasks,
                        _ => RightTab::Files,
                    };
                }
                "settings" => self.overlay = Some(Overlay::Settings),
                "remote" => self.view = View::Remote,
                "share" => self.overlay = Some(Overlay::Share),
                #[cfg(windows)]
                "browser" => self.view = View::Browser,
                #[cfg(windows)]
                "vault" => {
                    self.overlay = Some(Overlay::Settings);
                    self.settings_section = SettingsSection::Passwords;
                }
                #[cfg(windows)]
                "account" => {
                    self.overlay = Some(Overlay::Settings);
                    self.settings_section = SettingsSection::Account;
                }
                _ => {}
            }
        }
        if !self.command_palette.open {
            self.command_palette_focused = false;
        }
    }
    fn sidebar(&mut self, ctx: &egui::Context) {
        if !self.state.panels.left_open {
            return;
        }
        egui::SidePanel::left("projects")
            .default_width(280.0)
            .width_range(220.0..=500.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(tr("Projects"));
                    if ui.button("+").on_hover_text(tr("Open folder")).clicked() {
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
                for session in self
                    .sessions
                    .iter()
                    .filter(|s| s.project == self.state.active_project)
                {
                    if ui
                        .selectable_label(Some(session.id) == self.active_session, &session.title)
                        .clicked()
                    {
                        self.active_session = Some(session.id);
                        self.view = View::Work;
                    }
                }
            });
    }
    fn explorer(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(tr("Open editors"))
            .default_open(true)
            .show(ui, |ui| {
                for (i, document) in self.documents.iter().enumerate() {
                    if ui
                        .selectable_label(
                            self.view == View::Files && i == self.active_document,
                            format!(
                                "{}{}",
                                document
                                    .path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy(),
                                if document.dirty() { " *" } else { "" }
                            ),
                        )
                        .on_hover_text(document.path.display().to_string())
                        .clicked()
                    {
                        self.active_document = i;
                        self.view = View::Files;
                    }
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong(tr("Files"));
            if ui.small_button(tr("Refresh")).clicked()
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
                && ui.button(tr(".. Parent folder")).clicked()
            {
                self.list_directory(parent.to_owned());
            }
            ui.small(dir.file_name().unwrap_or_default().to_string_lossy());
            ui.add_enabled_ui(self.file_actions_ready(), |ui| {
                ui.menu_button(tr("New…"), |ui| {
                    for (title, kind) in [
                        ("Create file", crate::file_actions::Kind::File),
                        ("Create folder", crate::file_actions::Kind::Folder),
                    ] {
                        if ui.button(tr(title)).clicked() {
                            self.file_action = Some(crate::file_actions::Action {
                                kind,
                                root: root.clone(),
                                path: dir.clone(),
                                name: String::new(),
                            });
                            ui.close();
                        }
                    }
                });
            });
        }
        let mut open = None;
        egui::ScrollArea::vertical()
            .id_salt("files-list")
            .show_rows(ui, 22.0, self.entries.len(), |ui, range| {
                for i in range {
                    let e = &self.entries[i];
                    let response = ui.selectable_label(
                        false,
                        format!("{} {}", if e.directory { ">" } else { "·" }, e.name),
                    );
                    if self.file_actions_ready()
                        && let Some(project) = self.state.active()
                    {
                        response.context_menu(|ui| {
                            for (title, kind) in [
                                ("Rename…", crate::file_actions::Kind::Rename),
                                ("Delete…", crate::file_actions::Kind::Delete),
                            ] {
                                if ui.button(tr(title)).clicked() {
                                    self.file_action = Some(crate::file_actions::Action {
                                        kind,
                                        root: project.path.clone(),
                                        path: e.path.clone(),
                                        name: e.name.clone(),
                                    });
                                    ui.close();
                                }
                            }
                        });
                    }
                    if response.clicked() && !self.file_action_pending {
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
    }
    fn right_panel(&mut self, ctx: &egui::Context) {
        if !self.state.panels.right_open {
            return;
        }
        let max_width = (ctx.available_rect().width() - 420.0).max(260.0);
        egui::SidePanel::right("tools-panel")
            .default_width(330.0)
            .width_range(260.0..=max_width)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (RightTab::Files, "Files"),
                        (RightTab::Changes, "Changes"),
                        (RightTab::Tasks, "Tasks"),
                    ] {
                        ui.selectable_value(&mut self.state.panels.right_tab, tab, tr(label));
                    }
                    if ui
                        .small_button("×")
                        .on_hover_text(tr("Close panel"))
                        .clicked()
                    {
                        self.state.panels.right_open = false;
                    }
                });
                ui.separator();
                match self.state.panels.right_tab {
                    RightTab::Files => self.explorer(ui),
                    RightTab::Changes => {
                        egui::ScrollArea::both()
                            .id_salt("side-git")
                            .show(ui, |ui| self.git_view(ui));
                    }
                    RightTab::Tasks => {
                        egui::ScrollArea::both()
                            .id_salt("side-tasks")
                            .show(ui, |ui| {
                                ui.set_min_width(880.0);
                                self.tasks(ui);
                            });
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
            ui.heading(tr("Start working"));
            ui.label(tr("Open a project, then launch a terminal or installed agent. Switching projects keeps sessions running."));
            if ui.button(tr("Open terminal")).clicked() {
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
                            if ui.small_button(tr("Sleep")).clicked() {
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
                            ui.label(tr("Starting…"));
                        } else if ui.button(tr("Wake / reconnect")).clicked() {
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
        if self.file_action_pending {
            ui.spinner();
            ui.label(tr("File operation in progress…"));
            return;
        }
        let mut close = None;
        let Some(d) = self.documents.get_mut(self.active_document) else {
            ui.label(tr("Choose a UTF-8 file in the explorer."));
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
                && (ui.button(tr("Save (Ctrl+S)")).clicked()
                    || ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S)))
            {
                d.error = d.save().err().unwrap_or_default();
            }
            if readable {
                ui.selectable_value(&mut d.preview_mode, false, tr("Source"));
                ui.selectable_value(&mut d.preview_mode, true, tr("Readable preview"));
            }
            if ui.button(tr("Close")).clicked() {
                close = Some(self.active_document);
            }
            ui.small(d.path.display().to_string());
        });
        if !d.error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &d.error);
        }
        let mut link = None;
        if let Some(office) = &mut d.office {
            office.show(ui);
        } else if let Some(pdf) = &mut d.pdf {
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
            ui.small(tr(
                "Native readable view · scripts, CSS and external resources are not loaded",
            ));
            let preview = d.preview.get_or_insert_with(|| {
                crate::preview::Preview::parse(
                    &d.text,
                    matches!(extension.as_str(), "md" | "markdown"),
                )
            });
            if ui.button(tr("Copy plain text")).clicked() {
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
                        self.show_agent_browser = false;
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
            ui.label(tr("Select a project first."));
            return;
        };
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.task_title);
            if ui.button(tr("Add task")).clicked() && !self.task_title.trim().is_empty() {
                let id = match self.state.id() {
                    Ok(id) => id,
                    Err(e) => {
                        self.error = e;
                        return;
                    }
                };
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
                ui.heading(tr(title));
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
    fn can_restore(&self) -> bool {
        self.store.is_some()
            && !self.file_action_pending
            && !self
                .sessions
                .iter()
                .any(|s| s.terminal.is_some() || s.loading.is_some())
            && !self.chats.iter().any(|chat| chat.busy())
            && !self.claude_chats.iter().any(|chat| chat.busy())
            && !self.remote.needs_close_confirmation()
            && !self.documents.iter().any(Document::dirty)
            && self.git_pending.is_none()
            && !self.account_busy()
    }
    fn restore_settings(
        &mut self,
        mut state: Workspace,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        if !self.can_restore() {
            return Err(
                "Stop active sessions and finish or save pending work before restoring settings"
                    .into(),
            );
        }
        state.active_project = None;
        self.save();
        let backup = self
            .store
            .as_ref()
            .ok_or("Settings store unavailable")?
            .restore(&self.state, &state)?;
        self.sessions.clear();
        self.chats.clear();
        self.claude_chats.clear();
        self.documents.clear();
        self.document_pending.clear();
        self.active_session = None;
        self.active_document = 0;
        self.pending_close = None;
        self.share = Default::default();
        self.remote = Default::default();
        #[cfg(windows)]
        {
            self.agent_browsers = Default::default();
            self.show_agent_browser = false;
        }
        let (sender, receiver) = mpsc::channel();
        self.sender = sender;
        self.receiver = receiver;
        self.state = state;
        self.next_session = 1;
        for saved in &self.state.sessions {
            self.next_session = self.next_session.max(saved.id + 1);
            self.sessions.push(Session {
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
        crate::i18n::set_language(self.state.language);
        crate::theme::apply(ctx, self.state.dark, self.state.theme);
        self.refresh_project();
        self.backup_status = format!(
            "{} {}",
            tr("Settings restored. Previous settings saved at:"),
            backup.display()
        );
        Ok(())
    }
    fn backup_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading(tr("Settings backup"));
        ui.label(tr("Exports project paths, tasks, SSH aliases and sleeping session metadata. No passwords, account tokens, browser data or project files. The JSON file is not encrypted."));
        if ui.button(tr("Export settings…")).clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_file_name("tessel-rust-settings.json")
                .save_file()
        {
            self.save();
            self.backup_status = match crate::backup::export(&path, &self.state) {
                Ok(()) => format!("{} {}", tr("Settings exported:"), path.display()),
                Err(e) => e,
            };
        }
        if ui.button(tr("Import settings…")).clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("JSON", &["json"])
                .pick_file()
        {
            match crate::backup::import(&path) {
                Ok(state) => self.restore_preview = Some(state),
                Err(e) => self.backup_status = e,
            }
        }
        if !self.backup_status.is_empty() {
            ui.label(&self.backup_status);
        }
        if let Some(state) = &self.restore_preview {
            let mut apply = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("restore-settings")).show(ui.ctx(), |ui| {
                ui.heading(tr("Replace current settings?"));
                ui.label(format!("{}: {} · {}: {} · {}: {}", tr("Projects"), state.projects.len(), tr("Tasks"), state.tasks.len(), tr("Sessions"), state.sessions.len()));
                ui.label(tr("Project folders are checked when opened, not during restore."));
                ui.collapsing(tr("Projects"), |ui| {
                    egui::ScrollArea::vertical().max_height(180.0).show_rows(ui, 22.0, state.projects.len(), |ui, rows| {
                        for index in rows { let project = &state.projects[index]; ui.label(format!("{} — {}", project.name, project.path.display())); }
                    });
                });
                ui.label(tr("Current settings are backed up first. Sharing and agent-browser access stop. Restored sessions stay asleep."));
                if !self.can_restore() { ui.label(tr("Stop active sessions and finish or save pending work before restoring settings")); }
                apply = ui.add_enabled(self.can_restore(), egui::Button::new(tr("Replace settings"))).clicked();
                cancel = ui.button(tr("Cancel")).clicked();
            });
            if apply
                && let Some(state) = self.restore_preview.take()
                && let Err(e) = self.restore_settings(state, ui.ctx())
            {
                self.backup_status = e;
            }
            if cancel {
                self.restore_preview = None;
            }
        }
    }
    fn appearance_settings(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(tr("Language"));
            let previous = self.state.language;
            egui::ComboBox::from_id_salt("ui-language")
                .selected_text(match previous {
                    crate::i18n::Language::English => "English",
                    crate::i18n::Language::French => "Français",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.state.language,
                        crate::i18n::Language::English,
                        "English",
                    );
                    ui.selectable_value(
                        &mut self.state.language,
                        crate::i18n::Language::French,
                        "Français",
                    );
                });
            if previous != self.state.language {
                crate::i18n::set_language(self.state.language);
                self.save();
            }
        });
        ui.heading(tr("Appearance"));
        let previous_theme = self.state.theme;
        let french = self.state.language == crate::i18n::Language::French;
        let selected = if self.state.theme.is_none() && !self.state.dark {
            tr("Legacy light theme")
        } else {
            self.state.theme.unwrap_or_default().label(french)
        };
        egui::ComboBox::from_id_salt("appearance-theme")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for theme in crate::theme_palettes::THEMES {
                    ui.selectable_value(&mut self.state.theme, Some(theme), theme.label(french))
                        .on_hover_text(theme.description(french));
                }
            });
        let mut changed = previous_theme != self.state.theme;
        changed |= ui
            .add(egui::Slider::new(&mut self.state.font_size, 10.0..=28.0).text(tr("Font size")))
            .changed();
        if changed {
            crate::theme::apply(ui.ctx(), self.state.dark, self.state.theme);
            self.save();
        }
    }
    fn host_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading(tr("SSH hosts"));
        ui.label(tr("Uses OpenSSH and your SSH config. Authentication and host verification remain with OpenSSH."));
        ui.horizontal(|ui| {
            ui.label(tr("Alias / user@host"));
            ui.text_edit_singleline(&mut self.host_alias);
        });
        ui.horizontal(|ui| {
            ui.label(tr("Remote folder (optional)"));
            ui.text_edit_singleline(&mut self.host_folder);
        });
        if ui.button(tr("Save host")).clicked() {
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
            if ui
                .button(format!("{} {}", tr("Connect"), host.alias))
                .clicked()
            {
                connect = Some(host.clone());
            }
        }
        if let Some(h) = connect {
            self.new_session(Launch::Ssh(h));
            self.overlay = None;
        }
    }
    fn agent_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading(tr("Agent CLIs"));
        if ui.button(tr("Refresh detection")).clicked() {
            self.detect_agents();
        }
        for (name, installed) in &self.agents {
            ui.label(format!(
                "{name}: {}",
                if *installed {
                    tr("available")
                } else {
                    tr("not on PATH")
                }
            ));
        }
    }
    fn show_overlay(&mut self, ctx: &egui::Context) {
        let Some(overlay) = self.overlay else {
            return;
        };
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("workspace-overlay")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 50.0).clamp(260.0, 880.0));
            ui.horizontal(|ui| {
                ui.heading(tr(if overlay == Overlay::Settings {
                    "Settings"
                } else {
                    "Share"
                }));
                if ui.button("×").on_hover_text(tr("Close")).clicked() {
                    close = true;
                }
            });
            ui.separator();
            let height = (ctx.content_rect().height() * 0.86 - 65.0).clamp(150.0, 615.0);
            if overlay == Overlay::Settings {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(150.0);
                        for (section, title) in [
                            (SettingsSection::Agents, "Agent CLIs"),
                            (SettingsSection::Appearance, "Appearance"),
                            (SettingsSection::Hosts, "SSH hosts"),
                            #[cfg(windows)]
                            (SettingsSection::Passwords, "Passwords"),
                            #[cfg(windows)]
                            (SettingsSection::Account, "Account"),
                            (SettingsSection::Backup, "Settings backup"),
                        ] {
                            ui.selectable_value(&mut self.settings_section, section, tr(title));
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("settings-content")
                        .max_height(height)
                        .show(ui, |ui| {
                            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                ui.set_max_width(ui.available_width());
                                ui.set_min_width(ui.available_width().min(320.0));
                                match self.settings_section {
                                    SettingsSection::Appearance => self.appearance_settings(ui),
                                    SettingsSection::Hosts => self.host_settings(ui),
                                    SettingsSection::Agents => self.agent_settings(ui),
                                    SettingsSection::Backup => self.backup_controls(ui),
                                    #[cfg(windows)]
                                    SettingsSection::Passwords => {
                                        self.vault.show(ui, &mut self.browser)
                                    }
                                    #[cfg(windows)]
                                    SettingsSection::Account => self.account.show(ui),
                                }
                            });
                        });
                });
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("sharing-content")
                    .max_height(height)
                    .show(ui, |ui| {
                        self.share
                            .show(ui, self.state.active().map(|p| p.path.as_path()));
                    });
            }
        });
        if self.restore_preview.is_none() && (close || response.should_close()) {
            self.overlay = None;
        }
    }
    fn confirmations(&mut self, ctx: &egui::Context) {
        if let Some(index) = self.pending_close {
            egui::Modal::new(egui::Id::new("unsaved-file")).show(ctx, |ui| {
                ui.label(tr("Save changes before closing this file?"));
                ui.horizontal(|ui| {
                    if ui.button(tr("Cancel")).clicked() {
                        self.pending_close = None;
                    }
                    if ui.button(tr("Save and close")).clicked() {
                        match self.documents[index].save() {
                            Ok(()) => {
                                self.documents.remove(index);
                                self.pending_close = None;
                                self.active_document = 0;
                            }
                            Err(e) => self.error = e,
                        }
                    }
                    if ui.button(tr("Discard changes")).clicked() {
                        self.documents.remove(index);
                        self.pending_close = None;
                        self.active_document = 0;
                    }
                });
            });
        }
        self.file_action_dialog(ctx);
        if self.pending_exit {
            egui::Modal::new(egui::Id::new("close-app")).show(ctx, |ui| {
                ui.label(tr("There are unsaved files or sessions. Closing ends sessions."));
                if ui.button(tr("Keep working")).clicked() {
                    self.pending_exit = false;
                }
                if self.file_action_pending { ui.label(tr("File operation in progress…")); }
                if self.account_busy() { ui.label(tr("Wait for the account request to finish before closing.")); }
                if self.remote.needs_close_confirmation() {ui.label(tr("Finish remote requests and save remote edits in Remote files before saving and closing."));}
                if ui.add_enabled(!self.remote.needs_close_confirmation() && !self.account_busy() && !self.file_action_pending,egui::Button::new(tr("Save all and close"))).clicked() {
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
                if ui.add_enabled(!self.remote.busy() && !self.account_busy() && !self.file_action_pending,egui::Button::new(tr("Discard edits and close"))).clicked() {
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
            ui.label(tr("Select a project for native chat."));
            return;
        };
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.show_claude, false, "Codex");
            ui.selectable_value(&mut self.show_claude, true, "Claude");
        });
        if self.show_claude {
            if let Some(chat) = self
                .claude_chats
                .iter_mut()
                .find(|c| c.project == project.id)
            {
                let height = ui.available_height();
                egui::ScrollArea::vertical()
                    .id_salt("claude-panel-scroll")
                    .show(ui, |ui| {
                        ui.set_max_height(height);
                        chat.show(ui);
                    });
            } else {
                ui.heading(tr("Native Claude chat"));
                if ui.button(tr("Create chat")).clicked() {
                    self.claude_chats.push(crate::claude::Chat::new(
                        project.id,
                        project.path,
                        self.state.claude_threads.get(&project.id).cloned(),
                    ));
                }
            }
            return;
        }
        if let Some(chat) = self.chats.iter_mut().find(|c| c.project == project.id) {
            let height = ui.available_height();
            egui::ScrollArea::vertical()
                .id_salt("codex-panel-scroll")
                .show(ui, |ui| {
                    ui.set_max_height(height);
                    chat.show(ui);
                });
        } else {
            ui.heading(tr("Native Codex chat"));
            ui.label(tr("Connect your installed Codex CLI with its existing sign-in. No API key is copied into Tessel Rust."));
            if ui.button(tr("Create chat")).clicked() {
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
        ui.heading(tr("Source control"));
        ui.label(tr("On-demand Git operations. Output is capped at 2 MiB; operations time out after 20 seconds."));
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
                    if ui.button(tr(label)).clicked() {
                        self.git_start(args.into_iter().map(str::to_owned).collect(), None);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label(tr("Relative file path"));
                ui.text_edit_singleline(&mut self.git_path);
                if ui.button(tr("Stage file")).clicked() && !self.git_path.trim().is_empty() {
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
                ui.label(tr("Commit message"));
                ui.text_edit_singleline(&mut self.commit_message);
                if ui.button(tr("Commit staged changes")).clicked()
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
        #[cfg(feature = "visual-test")]
        if let Ok(path) = std::env::var("TESSEL_SETTLED_SCREENSHOT") {
            self.visual_frames += 1;
            if self.visual_frames == 6 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            for event in ctx.input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    if let Err(e) = image::save_buffer(
                        &path,
                        &bytes,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    ) {
                        self.error = e.to_string();
                    }
                    self.allow_exit = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        self.poll();
        let previous_panels = self.state.panels;
        let terminal_focused = self.view == View::Work
            && self.sessions.iter().any(|session| {
                session.project == self.state.active_project
                    && session
                        .terminal
                        .as_ref()
                        .is_some_and(|terminal| terminal.has_focus(ctx))
            });
        let confirmation_open = self.pending_close.is_some()
            || self.pending_exit
            || self.file_action.is_some()
            || self.restore_preview.is_some();
        let keyboard_free = !ctx.wants_keyboard_input() || terminal_focused;
        if !confirmation_open
            && !self.command_palette.open
            && keyboard_free
            && matches!(self.overlay, None | Some(Overlay::Settings))
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Comma))
        {
            self.overlay = if self.overlay.is_some() {
                None
            } else {
                Some(Overlay::Settings)
            };
        }
        if self.overlay.is_none() && !confirmation_open {
            if (keyboard_free || self.command_palette.open)
                && ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::P)
                })
            {
                self.command_palette.toggle();
            }
            if keyboard_free && !self.command_palette.open {
                for (key, tab) in [
                    (egui::Key::X, RightTab::Files),
                    (egui::Key::G, RightTab::Changes),
                    (egui::Key::K, RightTab::Tasks),
                ] {
                    if ctx.input_mut(|i| {
                        i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, key)
                    }) {
                        self.state.panels.toggle_right(tab);
                    }
                }
                if ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::T)
                }) {
                    self.new_session(Launch::Shell);
                }
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_exit
            && (self.documents.iter().any(Document::dirty)
                || !self.sessions.is_empty()
                || self.chats.iter().any(crate::chat::Chat::busy)
                || self.claude_chats.iter().any(crate::claude::Chat::busy)
                || self.share.hosting()
                || self.remote.needs_close_confirmation()
                || self.account_busy()
                || self.file_action_pending)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_exit = true;
        }
        egui::TopBottomPanel::top("toolbar")
            .exact_height(40.0)
            .frame(
                egui::Frame::default()
                    .fill(crate::theme::chrome(ctx))
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.strong("Tessel").on_hover_text("Tessel Rust");
                    ui.small(env!("TESSEL_BUILD"));
                    let search_width = (ui.available_width() - 350.0).clamp(80.0, 380.0);
                    if ui
                        .add_sized(
                            [search_width, 26.0],
                            egui::Button::new(tr("Search panes, run a command")),
                        )
                        .on_hover_text("Ctrl+Shift+P")
                        .clicked()
                    {
                        self.command_palette.toggle();
                    }
                    if ui.button("+").on_hover_text(tr("Open terminal")).clicked() {
                        self.new_session(Launch::Shell);
                    }
                    let mut agent = None;
                    ui.menu_button("▾", |ui| {
                        if ui.button(tr("Open terminal")).clicked() {
                            self.new_session(Launch::Shell);
                            ui.close();
                        }
                        if ui.button(tr("Native chat")).clicked() {
                            self.view = View::Chat;
                            ui.close();
                        }
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
                    if let Some(agent) = agent {
                        self.new_session(Launch::Agent(agent));
                    }
                    ui.separator();
                    if ui.button(tr("Share")).clicked() {
                        self.overlay = Some(Overlay::Share);
                    }
                    ui.menu_button(tr("Layout"), |ui| {
                        ui.checkbox(&mut self.state.panels.left_open, tr("Projects"));
                        ui.checkbox(&mut self.state.panels.right_open, tr("Side panel"));
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
                    if ui.button("⚙").on_hover_text(tr("Settings")).clicked() {
                        self.overlay = Some(Overlay::Settings);
                    }
                });
            });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.small(tr("Rust native · development"));
                ui.small(env!("TESSEL_BUILD"));
                if self.store.is_none() {
                    ui.colored_label(egui::Color32::YELLOW, "Persistence unavailable");
                }
                if !self.error.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
                    if ui.small_button(tr("Dismiss")).clicked() {
                        self.error.clear();
                    }
                }
            });
        });
        self.sidebar(ctx);
        self.right_panel(ctx);
        #[cfg(windows)]
        self.agent_browsers.hide_all();
        #[cfg(windows)]
        self.browser.visible(
            !self.show_agent_browser
                && self.view == View::Browser
                && !self.pending_exit
                && self.pending_close.is_none()
                && self.file_action.is_none()
                && !self.command_palette.open
                && self.overlay.is_none(),
        );
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.command_palette.open
                || self.overlay.is_some()
                || self.pending_exit
                || self.pending_close.is_some()
                || self.file_action.is_some()
            {
                ui.disable();
            }
            self.center_tabs(ui);
            match self.view {
                View::Work => self.work(ui),
                View::Files => self.editor(ui),
                View::Chat => self.chat_view(ui),
                View::Remote => self.remote.show(ui, &self.state.hosts),
                #[cfg(windows)]
                View::Browser => {
                    if !self.pending_exit
                        && self.pending_close.is_none()
                        && self.file_action.is_none()
                        && !self.command_palette.open
                        && self.overlay.is_none()
                    {
                        ui.horizontal(|ui| {
                            ui.selectable_value(
                                &mut self.show_agent_browser,
                                false,
                                tr("Owner browser"),
                            );
                            ui.selectable_value(
                                &mut self.show_agent_browser,
                                true,
                                tr("Project agent"),
                            );
                        });
                        if self.show_agent_browser {
                            self.browser.visible(false);
                            if let Some(project) = self.state.active() {
                                self.agent_browsers
                                    .show(ui, frame, project.id, &project.path);
                            } else {
                                ui.label(tr("Select a project first."));
                            }
                        } else {
                            self.browser.show(ui, frame);
                        }
                    }
                }
            }
        });
        self.show_overlay(ctx);
        self.show_command_palette(ctx);
        self.confirmations(ctx);
        if previous_panels != self.state.panels {
            self.save();
        }
        ctx.request_repaint_after(Duration::from_millis(if self.sessions.is_empty() {
            150
        } else {
            33
        }));
    }
}
