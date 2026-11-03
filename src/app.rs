use crate::i18n::tr;
use crate::{
    files::Document,
    launch::{self, Launch},
    terminal::Terminal,
    workspace::{self, Host, Store, Workspace},
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
        if self.launch.native_chat().is_some() || self.terminal.is_some() || self.loading.is_some()
        {
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
        let result = self
            .loading
            .as_ref()
            .and_then(|receiver| match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "Terminal startup stopped without a result; retry the connection.".into(),
                )),
            });
        if let Some(result) = result {
            self.loading = None;
            match result {
                Ok(t) => {
                    self.error.clear();
                    self.terminal = Some(t);
                }
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
    Document(PathBuf, Result<Box<Document>, String>),
}
#[derive(PartialEq)]
enum View {
    Work,
    Files,
    Remote,
    #[cfg(windows)]
    Browser,
}
use crate::workspace::SideTab as RightTab;
#[derive(Clone, Copy, PartialEq)]
enum Overlay {
    Settings,
    Share,
    Help,
    Launcher,
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
    #[cfg(windows)]
    window_frame: Option<crate::window_frame::WindowFrame>,
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
    help: crate::help_panel::HelpPanel,
    launcher: crate::launcher_panel::LauncherPanel,
    project_panel: crate::project_panel::ProjectPanel,
    empty_state: crate::empty_state_panel::EmptyState,
    settings_section: SettingsSection,
    settings_nav: crate::settings_nav::SettingsNav,
    appearance_panel: crate::appearance_panel::AppearancePanel,
    command_palette: crate::palette::CommandPaletteState,
    command_palette_focused: bool,
    toolbar_menu_open: bool,
    finder: crate::finder_panel::Finder,
    grid_room: [f64; 2],
    grid_dirty: bool,
    explorer_tree: crate::explorer_tree::ExplorerTree,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    agents: Vec<(String, bool)>,
    host_alias: String,
    host_folder: String,
    tasks_board: crate::tasks_board_panel::TasksBoard,
    tasks_board_project: Option<u64>,
    pending_close: Option<usize>,
    pending_exit: bool,
    restore_preview: Option<Workspace>,
    file_action: Option<crate::file_actions::Action>,
    file_action_pending: bool,
    backup_status: String,
    allow_exit: bool,
    #[cfg(feature = "visual-test")]
    visual_frames: usize,
    git_result: crate::git_result::GitResult,
    scm_state: crate::scm_state::ScmState,
    scm_panel: crate::scm_changes_panel::ScmChangesPanel,
    git_path: String,
    commit_message: String,
    chats: Vec<crate::chat::Chat>,
    claude_chats: Vec<crate::claude::Chat>,
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
        crate::system_fonts::install(&cc.egui_ctx);
        #[cfg(windows)]
        let window_frame = if cfg!(feature = "visual-test")
            && std::env::args().any(|arg| arg == "--native-frame-fixture")
        {
            crate::window_frame::WindowFrame::install(cc)
        } else {
            None
        };
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
            #[cfg(windows)]
            window_frame,
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
            help: Default::default(),
            launcher: Default::default(),
            project_panel: Default::default(),
            empty_state: Default::default(),
            settings_section: SettingsSection::Appearance,
            settings_nav: Default::default(),
            appearance_panel: Default::default(),
            command_palette: Default::default(),
            command_palette_focused: false,
            toolbar_menu_open: false,
            finder: Default::default(),
            grid_room: [800.0, 600.0],
            grid_dirty: false,
            explorer_tree: Default::default(),
            sender,
            receiver,
            agents: vec![],
            host_alias: String::new(),
            host_folder: String::new(),
            tasks_board: Default::default(),
            tasks_board_project: None,
            pending_close: None,
            pending_exit: false,
            restore_preview: None,
            file_action: None,
            file_action_pending: false,
            backup_status: String::new(),
            allow_exit: false,
            #[cfg(feature = "visual-test")]
            visual_frames: 0,
            git_result: Default::default(),
            scm_state: Default::default(),
            scm_panel: Default::default(),
            git_path: String::new(),
            commit_message: String::new(),
            chats: Vec::new(),
            claude_chats: Vec::new(),
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
                app.new_session(Launch::ClaudeChat);
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
                app.new_session(Launch::CodexChat);
            }
            if std::env::args().any(|a| a == "--restore-fixture") {
                app.overlay = Some(Overlay::Settings);
                app.settings_section = SettingsSection::Backup;
                app.restore_preview = Some(Workspace::default());
            }
            if std::env::args().any(|a| a == "--settings-fixture") {
                app.overlay = Some(Overlay::Settings);
            }
            if std::env::args().any(|a| a == "--help-fixture") {
                app.open_help();
            }
            if std::env::args().any(|a| a == "--launcher-fixture") {
                app.open_launcher();
            }
        }
        app.detect_agents();
        app.refresh_project();
        #[cfg(feature = "visual-test")]
        if std::env::args().any(|a| a == "--finder-fixture") {
            let root = std::env::current_dir().expect("fixture cwd");
            app.state.add_project(&root).expect("fixture project");
            app.finder = crate::finder_panel::Finder::fixture(
                app.state.active().expect("fixture project").path.clone(),
            );
        }

        #[cfg(feature = "visual-test")]
        for path in std::env::args()
            .collect::<Vec<_>>()
            .windows(2)
            .filter(|a| a[0] == "--preview-file")
            .map(|a| PathBuf::from(&a[1]))
        {
            let Some(root) = path.parent() else {
                continue;
            };
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
        #[cfg(feature = "visual-test")]
        if std::env::args().any(|a| a == "--grid-fixture") {
            for _ in 0..3 {
                let id = app.next_session;
                app.next_session += 1;
                app.sessions.push(Session {
                    id,
                    project: app.state.active_project,
                    title: format!("Terminal {id}"),
                    launch: Launch::Shell,
                    cwd: launch::default_directory(),
                    terminal: Terminal::visual_fixture().ok(),
                    loading: None,
                    error: String::new(),
                });
            }
            app.active_session = app.sessions.first().map(|s| s.id);
            app.view = View::Work;
        }
        #[cfg(feature = "visual-test")]
        if std::env::args().any(|a| a == "--explorer-fixture") {
            let root = std::env::current_dir().expect("fixture cwd");
            app.state.add_project(&root).expect("fixture project");
            let root = app.state.active().expect("fixture project").path.clone();
            let src = root.join("src");
            let scripts = root.join("scripts");
            let entry = |path: PathBuf, directory| crate::explorer_tree::TreeDirEntry {
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                path,
                directory,
            };
            app.explorer_tree = crate::explorer_tree::ExplorerTree::fixture(
                root.clone(),
                vec![
                    (
                        root.clone(),
                        vec![
                            entry(src.clone(), true),
                            entry(scripts.clone(), true),
                            entry(root.join("README.md"), false),
                        ],
                    ),
                    (
                        src.clone(),
                        vec![
                            entry(src.join("app.rs"), false),
                            entry(src.join("terminal.rs"), false),
                        ],
                    ),
                    (
                        scripts.clone(),
                        vec![entry(scripts.join("package-windows.ps1"), false)],
                    ),
                ],
                vec![src, scripts],
            );
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
        self.state.sync_terminal_layouts(self.grid_room);
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
    fn refresh_project(&mut self) {
        if self.tasks_board_project != self.state.active_project {
            self.tasks_board.reset();
            self.tasks_board_project = self.state.active_project;
        }
        self.finder.close();
        self.sync_git_project();
        self.explorer_tree
            .set_root(self.state.active().map(|p| p.path.as_path()));
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
        self.sync_git_project();
        let git_owner = self.git_result.pending_owner().cloned();
        self.git_result.poll();
        if !self.git_result.busy()
            && let Some(owner) = git_owner
        {
            self.scm_state.refresh_for(&owner);
            if self.git_result.output.starts_with("Error:") {
                self.error = self.git_result.output.clone();
            }
        }
        self.scm_state.poll();
        self.explorer_tree.poll();
        self.remote.poll();
        self.finder.poll();
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
                                } else {
                                    self.documents.retain(|doc| !doc.path.starts_with(before));
                                    self.active_document = self
                                        .active_document
                                        .min(self.documents.len().saturating_sub(1));
                                    self.pending_close = None;
                                }
                            }
                            self.explorer_tree.refresh();
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
        self.new_session_placed(launch, None);
    }
    fn new_session_placed(
        &mut self,
        launch: Launch,
        split: Option<(u64, crate::pane_grid::SplitDir)>,
    ) {
        self.new_session_positioned(launch, split, false);
    }
    fn new_session_positioned(
        &mut self,
        launch: Launch,
        split: Option<(u64, crate::pane_grid::SplitDir)>,
        before: bool,
    ) {
        if let Some(provider) = launch.native_chat() {
            let Some(project) = self.state.active_project else {
                self.error = tr("Select a project for native chat.").into();
                return;
            };
            if let Some(session) = self
                .sessions
                .iter()
                .find(|s| s.project == Some(project) && s.launch.native_chat() == Some(provider))
            {
                self.active_session = Some(session.id);
                self.view = View::Work;
                if let Some(layout) = self
                    .state
                    .terminal_layouts
                    .get_mut(&crate::terminal_layout::project_key(Some(project)))
                    && layout.maximized.is_some()
                {
                    layout.maximized = Some(session.id);
                    self.grid_dirty = true;
                }
                return;
            }
        }
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
            Launch::CodexChat => "Codex".into(),
            Launch::ClaudeChat => "Claude".into(),
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
        if let Some((target, dir)) = split {
            let layout = self
                .state
                .terminal_layouts
                .entry(crate::terminal_layout::project_key(
                    self.state.active_project,
                ))
                .or_default();
            if before {
                layout.split_with_order(target, id, dir, true);
            } else {
                layout.split(target, id, dir);
            }
        }
        self.active_session = Some(id);
        self.view = View::Work;
        self.save();
    }
    fn close_session(&mut self, id: u64) {
        let Some(index) = self.sessions.iter().position(|s| s.id == id) else {
            return;
        };
        let session = self.sessions.remove(index);
        match session.launch {
            Launch::CodexChat => self.chats.retain(|c| Some(c.project) != session.project),
            Launch::ClaudeChat => self
                .claude_chats
                .retain(|c| Some(c.project) != session.project),
            _ => {}
        }
        if self.active_session == Some(id) {
            self.active_session = self
                .sessions
                .iter()
                .find(|s| s.project == session.project)
                .map(|s| s.id);
        }
        self.save();
        self.grid_dirty = false;
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
    fn remove_document(&mut self, index: usize) {
        if index >= self.documents.len() {
            return;
        }
        self.documents.remove(index);
        self.active_document =
            active_after_close(self.active_document, index, self.documents.len());
        if self.documents.is_empty() && self.view == View::Files {
            self.view = View::Work;
        }
    }
    fn request_document_close(&mut self, index: usize) {
        if let Some(document) = self.documents.get(index) {
            if document.dirty() {
                self.pending_close = Some(index);
            } else {
                self.remove_document(index);
            }
        }
    }
    fn center_tabs(&mut self, ui: &mut egui::Ui) {
        use crate::view_tabs_browser_model::{OpenTabLabelInput, base_tab_label};
        let paths: Vec<_> = self
            .documents
            .iter()
            .map(|d| d.path.to_string_lossy().into_owned())
            .collect();
        let inputs: Vec<_> = paths
            .iter()
            .map(|path| OpenTabLabelInput {
                tab_path: path,
                diff: None,
            })
            .collect();
        let mut close = None;
        egui::ScrollArea::horizontal()
            .id_salt("center-tabs")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let terminal = crate::view_tabs::show(
                        ui,
                        crate::view_tabs::Tab {
                            title: tr("Terminal"),
                            hint: tr("Terminal"),
                            kind: crate::view_tabs::Kind::Terminal,
                            selected: self.view == View::Work,
                            dirty: false,
                            closable: false,
                        },
                    );
                    if terminal.body.clicked() {
                        self.view = View::Work;
                    }
                    #[cfg(windows)]
                    {
                        let browser = crate::view_tabs::show(
                            ui,
                            crate::view_tabs::Tab {
                                title: tr("Browser"),
                                hint: tr("Browser"),
                                kind: crate::view_tabs::Kind::Browser,
                                selected: self.view == View::Browser,
                                dirty: false,
                                closable: false,
                            },
                        );
                        if browser.body.clicked() {
                            self.view = View::Browser;
                        }
                    }
                    for (i, document) in self.documents.iter().enumerate() {
                        let tab = crate::view_tabs::show(
                            ui,
                            crate::view_tabs::Tab {
                                title: &base_tab_label(&inputs[i], &inputs),
                                hint: &paths[i],
                                kind: crate::view_tabs::Kind::Editor,
                                selected: self.view == View::Files && self.active_document == i,
                                dirty: document.dirty(),
                                closable: true,
                            },
                        );
                        if tab.close {
                            close = Some(i);
                        } else if tab.body.clicked() {
                            self.view = View::Files;
                            self.active_document = i;
                        }
                        let response = tab.body;
                        response.context_menu(|ui| {
                            if ui.button(tr("Close")).clicked() {
                                close = Some(i);
                                ui.close();
                            }
                            if ui.button(tr("Copy path")).clicked() {
                                ui.ctx().copy_text(paths[i].clone());
                                ui.close();
                            }
                        });
                    }
                });
            });
        if let Some(index) = close {
            self.request_document_close(index);
        }
        ui.separator();
    }
    fn show_command_palette(&mut self, ctx: &egui::Context) {
        if !self.command_palette.open {
            self.command_palette_focused = false;
            return;
        }
        use crate::palette::PaletteCommand;
        let mut commands: Vec<_> = [
            ("terminal", "Open terminal"),
            ("launcher", "Open a terminal or agent"),
            ("chat", "Native Codex chat"),
            ("claude-chat", "Native Claude chat"),
            ("files", "Files"),
            ("finder", "Find a file"),
            ("changes", "Changes"),
            ("tasks", "Tasks"),
            ("settings", "Settings"),
            ("help", "Keyboard shortcuts"),
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
        .map(|(id, title)| {
            let command = PaletteCommand::new(id, "Tessel", tr(title));
            let shortcut = match id {
                "terminal" => "Ctrl+Shift+T",
                "launcher" => "Ctrl+Shift+Space",
                "finder" => "Ctrl+Shift+J",
                "files" => "Ctrl+Shift+X",
                "changes" => "Ctrl+Shift+G",
                "tasks" => "Ctrl+Shift+K",
                "settings" => "Ctrl+,",
                "help" => "F1",
                _ => "",
            };
            if shortcut.is_empty() {
                command
            } else {
                command.with_shortcut(shortcut)
            }
        })
        .collect();
        commands.extend(self.state.projects.iter().map(|project| {
            PaletteCommand::new(
                format!("project:{}", project.id),
                tr("Projects"),
                &project.name,
            )
            .with_hint(project.path.display().to_string())
        }));
        commands.extend(self.sessions.iter().map(|session| {
            let hint = self
                .state
                .projects
                .iter()
                .find(|p| Some(p.id) == session.project)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| tr("Sessions without a project").into());
            PaletteCommand::new(
                format!("session:{}", session.id),
                tr("Sessions"),
                &session.title,
            )
            .with_hint(hint)
        }));
        commands.extend(self.documents.iter().enumerate().map(|(index, document)| {
            PaletteCommand::new(
                format!("document:{index}"),
                tr("Open editors"),
                document
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
            )
            .with_hint(document.path.display().to_string())
        }));
        let selected = crate::command_palette_panel::show(
            ctx,
            &mut self.command_palette,
            &mut self.command_palette_focused,
            &commands,
        );
        if let Some(id) = selected {
            self.command_palette.close();
            match id.as_str() {
                "terminal" => self.new_session(Launch::Shell),
                "launcher" => self.open_launcher(),
                "chat" => self.new_session(Launch::CodexChat),
                "claude-chat" => self.new_session(Launch::ClaudeChat),
                "finder" => self.open_finder(),
                "files" | "changes" | "tasks" => {
                    self.state.panels.right_open = true;
                    self.state.panels.right_tab = match id.as_str() {
                        "changes" => RightTab::Changes,
                        "tasks" => RightTab::Tasks,
                        _ => RightTab::Files,
                    };
                }
                "settings" => self.overlay = Some(Overlay::Settings),
                "help" => self.open_help(),
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
                _ => {
                    if let Some(project) = id
                        .strip_prefix("project:")
                        .and_then(|s| s.parse::<u64>().ok())
                    {
                        if self.state.projects.iter().any(|p| p.id == project) {
                            self.state.active_project = Some(project);
                            self.view = View::Work;
                            self.refresh_project();
                            self.save();
                        }
                    } else if let Some(session_id) = id
                        .strip_prefix("session:")
                        .and_then(|s| s.parse::<u64>().ok())
                    {
                        if let Some(session) = self.sessions.iter().find(|s| s.id == session_id) {
                            let project = session.project;
                            if let Some(terminal) = &session.terminal {
                                terminal.request_focus(ctx);
                            }
                            self.active_session = Some(session_id);
                            self.view = View::Work;
                            if let Some(layout) = self
                                .state
                                .terminal_layouts
                                .get_mut(&crate::terminal_layout::project_key(project))
                                && layout.maximized.is_some()
                            {
                                layout.maximized = Some(session_id);
                                self.grid_dirty = true;
                            }
                            if self.state.active_project != project {
                                self.state.active_project = project;
                                self.refresh_project();
                            }
                            self.save();
                        }
                    } else if let Some(index) = id
                        .strip_prefix("document:")
                        .and_then(|s| s.parse::<usize>().ok())
                        && index < self.documents.len()
                    {
                        self.active_document = index;
                        self.view = View::Files;
                    }
                }
            }
        }
        if !self.command_palette.open {
            self.command_palette_focused = false;
        }
    }
    fn open_finder(&mut self) {
        if let Some(project) = self.state.active() {
            if let Err(error) = self.finder.open(&project.path) {
                self.error = error;
            }
        } else {
            self.error = tr("Select a project first.").into();
        }
    }
    fn sidebar(&mut self, ctx: &egui::Context) {
        use crate::project_panel::{
            ProjectAction, ProjectEntry, ProjectPanelInput, SessionEntry, SessionKind,
        };
        if !self.state.panels.left_open {
            return;
        }
        let entry = |session: &Session| SessionEntry {
            id: session.id,
            title: session.title.clone(),
            kind: match session.launch {
                Launch::Shell | Launch::Ssh(_) => SessionKind::Shell,
                Launch::Agent(_) => SessionKind::Agent,
                Launch::CodexChat | Launch::ClaudeChat => SessionKind::Chat,
            },
            sleeping: if let Some(provider) = session.launch.native_chat() {
                match provider {
                    "Codex" => !self
                        .chats
                        .iter()
                        .any(|c| Some(c.project) == session.project && c.busy()),
                    "Claude" => !self
                        .claude_chats
                        .iter()
                        .any(|c| Some(c.project) == session.project && c.busy()),
                    _ => true,
                }
            } else {
                session.terminal.is_none() && session.loading.is_none()
            },
        };
        let projects: Vec<_> = self
            .state
            .projects
            .iter()
            .map(|p| ProjectEntry {
                id: p.id,
                name: p.name.clone(),
                path: p.path.display().to_string(),
                sessions: self
                    .sessions
                    .iter()
                    .filter(|s| s.project == Some(p.id))
                    .map(&entry)
                    .collect(),
            })
            .collect();
        let global_sessions: Vec<_> = self
            .sessions
            .iter()
            .filter(|s| s.project.is_none())
            .map(&entry)
            .collect();
        let mut action = None;
        egui::SidePanel::left("projects")
            .default_width(280.0)
            .width_range(220.0..=500.0)
            .resizable(true)
            .show(ctx, |ui| {
                if self.overlay.is_some()
                    || self.command_palette.open
                    || self.finder.is_open()
                    || self.toolbar_menu_open
                    || self.pending_close.is_some()
                    || self.pending_exit
                    || self.file_action.is_some()
                    || self.restore_preview.is_some()
                {
                    ui.disable();
                }
                action = self.project_panel.show(
                    ui,
                    ProjectPanelInput {
                        projects: &projects,
                        global_sessions: &global_sessions,
                        active_project: self.state.active_project,
                        active_session: self.active_session,
                        french: self.state.language == crate::i18n::Language::French,
                    },
                );
            });
        let project = match action {
            Some(ProjectAction::Search) => {
                if !self.command_palette.open {
                    self.command_palette.toggle();
                }
                return;
            }
            Some(ProjectAction::OpenFolder) => {
                self.choose_project();
                return;
            }
            Some(ProjectAction::SelectProject(project)) => {
                if project.is_some_and(|id| !self.state.projects.iter().any(|p| p.id == id)) {
                    return;
                }
                project
            }
            Some(ProjectAction::SelectSession { project, id }) => {
                let Some(session) = self
                    .sessions
                    .iter()
                    .find(|s| s.id == id && s.project == project)
                else {
                    return;
                };
                self.active_session = Some(id);
                self.view = View::Work;
                if let Some(layout) = self
                    .state
                    .terminal_layouts
                    .get_mut(&crate::terminal_layout::project_key(project))
                    && layout.maximized.is_some()
                {
                    layout.maximized = Some(id);
                    self.grid_dirty = true;
                }
                if let Some(terminal) = &session.terminal {
                    terminal.request_focus(ctx);
                }
                project
            }
            None => return,
        };
        if self.state.active_project != project {
            self.state.active_project = project;
            self.refresh_project();
        }
        self.save();
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
        let root = self.state.active().map(|p| p.path.clone());
        let active = (self.view == View::Files)
            .then(|| self.documents.get(self.active_document))
            .flatten()
            .map(|d| d.path.clone());
        let actions_enabled = self.file_actions_ready();
        if let Some(action) =
            self.explorer_tree
                .show(ui, root.as_deref(), active.as_deref(), actions_enabled)
        {
            use crate::explorer_tree::{TreeAction, TreeContextKind};
            match action {
                TreeAction::OpenFile(path) if !self.file_action_pending => self.open_document(path),
                TreeAction::Context {
                    kind,
                    root: action_root,
                    path,
                    name,
                } if actions_enabled && root.as_ref() == Some(&action_root) => {
                    let kind = match kind {
                        TreeContextKind::CreateFile => crate::file_actions::Kind::File,
                        TreeContextKind::CreateFolder => crate::file_actions::Kind::Folder,
                        TreeContextKind::Rename => crate::file_actions::Kind::Rename,
                        TreeContextKind::Delete => crate::file_actions::Kind::Delete,
                    };
                    self.file_action = Some(crate::file_actions::Action {
                        kind,
                        root: action_root,
                        path,
                        name,
                    });
                }
                _ => {}
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
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (tab, label, icon, shortcut) in [
                        (
                            RightTab::Files,
                            "Files",
                            crate::icons::Icon::Files,
                            "Ctrl+Shift+X",
                        ),
                        (
                            RightTab::Changes,
                            "Changes",
                            crate::icons::Icon::GitBranch,
                            "Ctrl+Shift+G",
                        ),
                        (
                            RightTab::Tasks,
                            "Tasks",
                            crate::icons::Icon::ListChecks,
                            "Ctrl+Shift+K",
                        ),
                    ] {
                        if crate::chrome::side_tab(
                            ui,
                            icon,
                            &format!("{} ({shortcut})", tr(label)),
                            self.state.panels.right_tab == tab,
                        )
                        .clicked()
                        {
                            self.state.panels.right_tab = tab;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::pane_header::button(
                            ui,
                            crate::icons::Icon::PaneClose,
                            tr("Close panel"),
                        )
                        .clicked()
                        {
                            self.state.panels.right_open = false;
                        }
                    });
                });
                ui.separator();
                match self.state.panels.right_tab {
                    RightTab::Files => self.explorer(ui),
                    RightTab::Changes => self.git_view(ui),
                    RightTab::Tasks => self.tasks(ui),
                }
            });
    }
    fn tile_sessions(&mut self, columns: Option<usize>) {
        let key = crate::terminal_layout::project_key(self.state.active_project);
        if let Some(layout) = self.state.terminal_layouts.get_mut(&key) {
            layout.tile(self.grid_room, columns);
            self.save();
        }
    }
    fn work(&mut self, ui: &mut egui::Ui) {
        use crate::pane_grid::{self, SplitDir};
        let ids: Vec<u64> = self
            .sessions
            .iter()
            .filter(|s| s.project == self.state.active_project)
            .map(|s| s.id)
            .collect();
        if ids.is_empty() {
            use crate::empty_state_panel::Entry;
            let mut entries = Vec::new();
            for (name, installed) in &self.agents {
                if *installed {
                    entries.push(Entry {
                        id: format!("agent:{name}"),
                        label: name.clone(),
                        group: tr("Agent CLIs").into(),
                        enabled: true,
                    });
                }
            }
            if self.state.active_project.is_some() {
                for (id, label) in [
                    ("codex", "Native Codex chat"),
                    ("claude", "Native Claude chat"),
                ] {
                    entries.push(Entry {
                        id: format!("chat:{id}"),
                        label: tr(label).into(),
                        group: tr("Native chats").into(),
                        enabled: true,
                    });
                }
            }
            entries.push(Entry {
                id: "terminal".into(),
                label: tr("Terminal").into(),
                group: tr("Other").into(),
                enabled: true,
            });
            #[cfg(windows)]
            entries.push(Entry {
                id: "browser".into(),
                label: tr("Browser").into(),
                group: tr("Other").into(),
                enabled: true,
            });
            entries.push(Entry {
                id: "folder".into(),
                label: tr("Open folder").into(),
                group: tr("Other").into(),
                enabled: true,
            });
            let title = self
                .state
                .active()
                .map(|p| tr("What do you want to start in {{name}}?").replace("{{name}}", &p.name))
                .unwrap_or_else(|| tr("Start working").into());
            let subtitle = self
                .state
                .active()
                .map(|p| p.path.display().to_string())
                .unwrap_or_default();
            let action = self.empty_state.show(ui, &title, &subtitle, &entries);
            match action.as_deref() {
                Some("terminal") => {
                    self.new_session(Launch::Shell);
                }
                Some("folder") => self.choose_project(),
                Some("chat:codex") if self.state.active_project.is_some() => {
                    self.new_session(Launch::CodexChat);
                }
                Some("chat:claude") if self.state.active_project.is_some() => {
                    self.new_session(Launch::ClaudeChat);
                }
                #[cfg(windows)]
                Some("browser") => self.view = View::Browser,
                Some(id) => {
                    if let Some(name) = id.strip_prefix("agent:")
                        && self
                            .agents
                            .iter()
                            .any(|(known, installed)| *installed && known == name)
                    {
                        self.new_session(Launch::Agent(name.into()));
                    }
                }
                None => {}
            }
            return;
        }
        if !self.active_session.is_some_and(|id| ids.contains(&id)) {
            self.active_session = ids.first().copied();
        }
        let key = crate::terminal_layout::project_key(self.state.active_project);
        let room = ui.available_rect_before_wrap();
        self.grid_room = [room.width() as f64, room.height() as f64];
        let layout = self.state.terminal_layouts.entry(key.clone()).or_default();
        self.grid_dirty |= layout.sync(&ids, self.grid_room);
        let maximized = layout.maximized;
        let geometry = pane_grid::grid_layout(layout.tree.as_ref());
        if maximized.is_none()
            && ui.is_enabled()
            && (!ui.ctx().wants_keyboard_input()
                || self
                    .sessions
                    .iter()
                    .any(|s| s.terminal.as_ref().is_some_and(|t| t.has_focus(ui.ctx()))))
        {
            let positions: Vec<_> = geometry
                .tiles
                .iter()
                .map(|tile| {
                    (
                        tile.leaf.id.clone(),
                        tile.rect
                            .to_px(room.width() as f64, room.height() as f64, 4.0),
                    )
                })
                .collect();
            for (key, direction) in [
                (egui::Key::ArrowLeft, pane_grid::FocusDir::Left),
                (egui::Key::ArrowRight, pane_grid::FocusDir::Right),
                (egui::Key::ArrowUp, pane_grid::FocusDir::Up),
                (egui::Key::ArrowDown, pane_grid::FocusDir::Down),
            ] {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::ALT, key))
                    && let Some(next) = self
                        .active_session
                        .and_then(|id| {
                            pane_grid::move_focus(&positions, &id.to_string(), direction)
                        })
                        .and_then(|id| id.parse::<u64>().ok())
                {
                    self.active_session = Some(next);
                    if let Some(terminal) = self
                        .sessions
                        .iter()
                        .find(|s| s.id == next)
                        .and_then(|s| s.terminal.as_ref())
                    {
                        terminal.request_focus(ui.ctx());
                    }
                }
            }
        }
        ui.allocate_rect(room, egui::Sense::hover());
        let screen_rect = |relative: pane_grid::GridRect| {
            let px = relative.to_px(room.width() as f64, room.height() as f64, 4.0);
            egui::Rect::from_min_size(
                room.min + egui::vec2(px.left as f32, px.top as f32),
                egui::vec2(px.width as f32, px.height as f32),
            )
        };
        // Construct dormant native chats only; connecting remains an explicit action.
        if let Some(project) = self.state.active() {
            if self
                .sessions
                .iter()
                .any(|s| s.project == Some(project.id) && matches!(s.launch, Launch::CodexChat))
                && !self.chats.iter().any(|c| c.project == project.id)
            {
                let mut chat = crate::chat::Chat::new(project.id, project.path.clone());
                if let Some(id) = self.state.chat_threads.get(&project.id) {
                    chat.set_resume(id.clone());
                }
                self.chats.push(chat);
            }
            if self
                .sessions
                .iter()
                .any(|s| s.project == Some(project.id) && matches!(s.launch, Launch::ClaudeChat))
                && !self.claude_chats.iter().any(|c| c.project == project.id)
            {
                self.claude_chats.push(crate::claude::Chat::new(
                    project.id,
                    project.path.clone(),
                    self.state.claude_threads.get(&project.id).cloned(),
                ));
            }
        }
        let mut close = None;
        let mut split = None;
        let mut maximize = None;
        let mut arrange = false;
        for tile in &geometry.tiles {
            let Ok(id) = tile.leaf.id.parse::<u64>() else {
                continue;
            };
            if maximized.is_some_and(|active| active != id) {
                continue;
            }
            let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else {
                continue;
            };
            let rect = if maximized.is_some() {
                room
            } else {
                screen_rect(tile.rect)
            };
            if rect.width() < 1.0 || rect.height() < 1.0 {
                continue;
            }
            if ui.is_enabled()
                && ui.input(|i| {
                    i.pointer.any_pressed()
                        && i.pointer.interact_pos().is_some_and(|p| rect.contains(p))
                })
            {
                self.active_session = Some(id);
            }
            let active = self.active_session == Some(id);
            let color = if active {
                ui.visuals().selection.stroke.color
            } else {
                ui.visuals().widgets.noninteractive.bg_stroke.color
            };
            let clip = ui.clip_rect().intersect(rect);
            ui.scope_builder(
                egui::UiBuilder::new()
                    .id_salt(("grid-terminal", id))
                    .max_rect(rect),
                |ui| {
                    ui.set_clip_rect(clip);
                    egui::Frame::new()
                        .stroke(egui::Stroke::new(1.0_f32, color))
                        .inner_margin(4)
                        .show(ui, |ui| {
                            ui.set_min_size(ui.available_size());
                            ui.horizontal(|ui| {
                                let menu = crate::pane_header::MenuState {
                                    title: &session.title,
                                    maximized: maximized.is_some(),
                                    multiple_panes: ids.len() > 1,
                                    terminal_output: session.terminal.is_some(),
                                    language: if self.state.language
                                        == crate::i18n::Language::French
                                    {
                                        crate::pane_menu_model::PaneMenuLang::Fr
                                    } else {
                                        crate::pane_menu_model::PaneMenuLang::En
                                    },
                                };
                                let mut action = None;
                                ui.spacing_mut().item_spacing.x = 4.0;
                                let title_width = (ui.available_width() - 84.0).max(0.0);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(title_width, 22.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_min_width(title_width);
                                        ui.add(egui::Label::new(&session.title).truncate())
                                    },
                                )
                                .inner
                                .on_hover_text(&session.title)
                                .context_menu(|ui| {
                                    action = crate::pane_header::show(ui, &menu);
                                });
                                let more = crate::pane_header::button(
                                    ui,
                                    crate::icons::Icon::PaneMore,
                                    tr("More options"),
                                );
                                egui::Popup::menu(&more).show(|ui| {
                                    if let Some(selected) = crate::pane_header::show(ui, &menu) {
                                        action = Some(selected);
                                    }
                                    if session.launch.native_chat().is_none()
                                        && ui.button(tr("Sleep")).clicked()
                                    {
                                        session.terminal = None;
                                        session.loading = None;
                                        ui.close();
                                    }
                                });
                                if let Some(action) = action {
                                    use crate::pane_menu_model::PaneMenuAction;
                                    match action {
                                        PaneMenuAction::CopyOutput => {
                                            if let Some(terminal) = &session.terminal {
                                                ui.ctx().copy_text(terminal.output());
                                            }
                                        }
                                        PaneMenuAction::ToggleMaximize => maximize = Some(id),
                                        PaneMenuAction::ArrangeGrid => arrange = true,
                                        PaneMenuAction::SplitRight => {
                                            split = Some((id, SplitDir::Row))
                                        }
                                        PaneMenuAction::SplitDown => {
                                            split = Some((id, SplitDir::Col))
                                        }
                                        PaneMenuAction::ClosePane => close = Some(id),
                                        _ => {}
                                    }
                                }
                                if crate::pane_header::button(
                                    ui,
                                    if maximized.is_some() {
                                        crate::icons::Icon::PaneRestore
                                    } else {
                                        crate::icons::Icon::PaneMaximize
                                    },
                                    tr("Maximize / restore pane"),
                                )
                                .clicked()
                                {
                                    maximize = Some(id);
                                }
                                if crate::pane_header::button(
                                    ui,
                                    crate::icons::Icon::PaneClose,
                                    tr("Close"),
                                )
                                .clicked()
                                {
                                    close = Some(id);
                                }
                            });
                            if !session.error.is_empty() {
                                ui.colored_label(egui::Color32::LIGHT_RED, &session.error);
                            }
                            if session.launch.native_chat().is_some() {
                                let height = ui.available_height();
                                egui::ScrollArea::vertical()
                                    .id_salt(("native-chat", id))
                                    .show(ui, |ui| {
                                        ui.set_max_height(height);
                                        match session.launch {
                                            Launch::CodexChat => {
                                                if let Some(chat) = self
                                                    .chats
                                                    .iter_mut()
                                                    .find(|c| Some(c.project) == session.project)
                                                {
                                                    chat.show(ui);
                                                }
                                            }
                                            Launch::ClaudeChat => {
                                                if let Some(chat) = self
                                                    .claude_chats
                                                    .iter_mut()
                                                    .find(|c| Some(c.project) == session.project)
                                                {
                                                    chat.show(ui);
                                                }
                                            }
                                            _ => {}
                                        }
                                    });
                            } else if let Some(terminal) = &mut session.terminal {
                                terminal.show(ui, self.state.font_size);
                            } else if session.loading.is_some() {
                                ui.spinner();
                                ui.label(tr("Starting…"));
                            } else if ui.button(tr("Wake / reconnect")).clicked() {
                                session.start();
                            }
                        });
                },
            );
        }
        if maximized.is_none() {
            for divider in &geometry.dividers {
                let rect = screen_rect(divider.rect);
                let horizontal = divider.dir == SplitDir::Row;
                let response = ui
                    .interact(
                        rect,
                        ui.id().with(("grid-divider", &divider.key)),
                        egui::Sense::click_and_drag(),
                    )
                    .on_hover_cursor(if horizontal {
                        egui::CursorIcon::ResizeHorizontal
                    } else {
                        egui::CursorIcon::ResizeVertical
                    });
                if response.clicked() {
                    response.request_focus();
                }
                let focused = response.has_focus();
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    if response.hovered() || response.dragged() || focused {
                        ui.visuals().selection.stroke.color
                    } else {
                        ui.visuals().widgets.noninteractive.bg_stroke.color
                    },
                );
                let delta = if response.dragged() {
                    ui.input(|i| {
                        if horizontal {
                            i.pointer.delta().x
                        } else {
                            i.pointer.delta().y
                        }
                    }) as f64
                } else {
                    0.0
                };
                let direction = if focused && ui.is_enabled() {
                    let back = if horizontal {
                        egui::Key::ArrowLeft
                    } else {
                        egui::Key::ArrowUp
                    };
                    let forward = if horizontal {
                        egui::Key::ArrowRight
                    } else {
                        egui::Key::ArrowDown
                    };
                    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, back)) {
                        Some(false)
                    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, forward)) {
                        Some(true)
                    } else {
                        None
                    }
                } else {
                    None
                };
                if ui.is_enabled() && (delta != 0.0 || direction.is_some()) {
                    let bounds = screen_rect(divider.split_rect);
                    let pixels = if horizontal {
                        bounds.width()
                    } else {
                        bounds.height()
                    } as f64;
                    self.grid_dirty |= self
                        .state
                        .terminal_layouts
                        .get_mut(&key)
                        .expect("layout")
                        .resize(&divider.split_id, divider.index, pixels, delta, direction);
                }
            }
        }
        if arrange {
            if let Some(layout) = self.state.terminal_layouts.get_mut(&key) {
                layout.tile(self.grid_room, None);
            }
            self.grid_dirty = true;
        }
        if let Some(id) = maximize {
            self.state
                .terminal_layouts
                .get_mut(&key)
                .expect("layout")
                .maximized = if maximized == Some(id) {
                None
            } else {
                Some(id)
            };
            self.grid_dirty = true;
        }
        if let Some(id) = close {
            self.close_session(id);
        }
        if let Some(placement) = split {
            self.new_session_placed(Launch::Shell, Some(placement));
        }
        if self.grid_dirty && !ui.input(|i| i.pointer.any_down()) {
            self.grid_dirty = false;
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
        if document_toolbar(ui, d, readable) {
            close = Some(self.active_document);
        }
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
            self.request_document_close(i);
        }
    }
    fn tasks(&mut self, ui: &mut egui::Ui) {
        use crate::tasks_board_panel::{TaskCard, TasksBoardInput};
        let project = self.state.active_project.filter(|id| {
            self.state
                .projects
                .iter()
                .any(|candidate| candidate.id == *id)
        });
        if self.tasks_board_project != project {
            self.tasks_board.reset();
            self.tasks_board_project = project;
        }
        let cards: Vec<_> = self
            .state
            .tasks
            .iter()
            .filter(|task| Some(task.project) == project)
            .map(|task| TaskCard {
                id: task.id,
                title: task.title.clone(),
                column: task.column,
            })
            .collect();
        let action = self.tasks_board.show(
            ui,
            TasksBoardInput {
                tasks: &cards,
                french: self.state.language == crate::i18n::Language::French,
                project_selected: project.is_some(),
            },
        );
        let Some(project) = project else {
            return;
        };
        if let Some(action) = action {
            match crate::task_actions::apply(&mut self.state, project, action) {
                Ok(true) => self.save(),
                Ok(false) => {}
                Err(error) => self.error = tr(&error).into(),
            }
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
            && !self.git_result.busy()
            && !self.scm_state.busy()
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
        use crate::appearance_panel::{AppearanceAction, AppearanceInput};
        let action = self.appearance_panel.show(
            ui,
            &AppearanceInput {
                theme: self.state.theme,
                dark: self.state.dark,
                french: self.state.language == crate::i18n::Language::French,
                terminal_font_size: self.state.font_size,
            },
        );
        let changed = match action {
            Some(AppearanceAction::Theme(theme)) if self.state.theme != Some(theme) => {
                self.state.theme = Some(theme);
                crate::theme::apply(ui.ctx(), self.state.dark, self.state.theme);
                true
            }
            Some(AppearanceAction::Language { french }) => {
                let language = if french {
                    crate::i18n::Language::French
                } else {
                    crate::i18n::Language::English
                };
                let changed = self.state.language != language;
                self.state.language = language;
                crate::i18n::set_language(language);
                changed
            }
            Some(AppearanceAction::TerminalFontSize(size)) if size.is_finite() => {
                let size = size.clamp(10.0, 28.0);
                let changed = self.state.font_size != size;
                self.state.font_size = size;
                changed
            }
            _ => false,
        };
        if changed {
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
    fn open_help(&mut self) {
        self.help.reset();
        self.overlay = Some(Overlay::Help);
    }
    fn open_launcher(&mut self) {
        self.launcher.reset();
        self.overlay = Some(Overlay::Launcher);
    }
    fn launcher_ui(&mut self, ui: &mut egui::Ui) -> bool {
        use crate::finder_model::{
            LaunchMenuAction, LaunchMenuItem, LaunchPlacement, LauncherLang,
        };
        use crate::launcher_panel::{LauncherEntry, LauncherInput};
        let target = self
            .sessions
            .iter()
            .find(|s| Some(s.id) == self.active_session && s.project == self.state.active_project);
        let target_id = target.map(|s| s.id);
        let target_title = target.map(|s| s.title.clone());
        let mut entries = vec![LauncherEntry {
            item: LaunchMenuItem::Shell {
                id: "default".into(),
                name: if cfg!(windows) { "PowerShell" } else { "Shell" }.into(),
            },
            available: true,
            reason: None,
        }];
        for (id, name) in [
            ("codex", "Native Codex chat"),
            ("claude", "Native Claude chat"),
        ] {
            let available = self.state.active_project.is_some();
            entries.push(LauncherEntry {
                item: LaunchMenuItem::ChatAgent {
                    id: id.into(),
                    name: tr(name).into(),
                },
                available,
                reason: (!available).then(|| tr("Select a project for native chat.").into()),
            });
        }
        entries.extend(self.agents.iter().map(|(name, available)| LauncherEntry {
            item: LaunchMenuItem::Agent {
                id: name.clone(),
                name: name.clone(),
            },
            available: *available,
            reason: (!available).then(|| tr("not on PATH").into()),
        }));
        let placements = if target_id.is_some() {
            vec![
                LaunchPlacement::Auto,
                LaunchPlacement::Left,
                LaunchPlacement::Right,
                LaunchPlacement::Down,
            ]
        } else {
            vec![]
        };
        let action = self.launcher.show(
            ui,
            LauncherInput {
                entries: &entries,
                placements: &placements,
                target_title: target_title.as_deref(),
                lang: if self.state.language == crate::i18n::Language::French {
                    LauncherLang::Fr
                } else {
                    LauncherLang::En
                },
            },
        );
        match action {
            Some(LaunchMenuAction::Close) => true,
            Some(LaunchMenuAction::Launch {
                item,
                placement,
                in_worktree: false,
            }) => {
                if !entries
                    .iter()
                    .any(|entry| entry.available && entry.item == item)
                {
                    return false;
                }
                let launch = match item {
                    LaunchMenuItem::Shell { id, .. } if id == "default" => Launch::Shell,
                    LaunchMenuItem::Agent { id, .. } => Launch::Agent(id),
                    LaunchMenuItem::ChatAgent { id, .. } if id == "codex" => Launch::CodexChat,
                    LaunchMenuItem::ChatAgent { id, .. } if id == "claude" => Launch::ClaudeChat,
                    _ => return false,
                };
                let split = target_id.and_then(|id| match placement {
                    LaunchPlacement::Auto => None,
                    LaunchPlacement::Left | LaunchPlacement::Right => {
                        Some((id, crate::pane_grid::SplitDir::Row))
                    }
                    LaunchPlacement::Down => Some((id, crate::pane_grid::SplitDir::Col)),
                });
                self.new_session_positioned(launch, split, placement == LaunchPlacement::Left);
                true
            }
            _ => false,
        }
    }
    fn show_overlay(&mut self, ctx: &egui::Context) {
        let Some(overlay) = self.overlay else {
            return;
        };
        if overlay == Overlay::Launcher {
            let mut open = true;
            let bounds = ctx.content_rect();
            let position = egui::pos2(bounds.center().x - 170.0, bounds.top() + 80.0);
            egui::Popup::new(
                egui::Id::new("workspace-launcher"),
                ctx.clone(),
                position,
                egui::LayerId::background(),
            )
            .open_bool(&mut open)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(340.0)
            .show(|ui| {
                if self.launcher_ui(ui) {
                    ui.close();
                }
            });
            if !open {
                self.overlay = None;
            }
            return;
        }
        if overlay == Overlay::Settings {
            self.show_settings(ctx);
            return;
        }
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("workspace-overlay")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 50.0).clamp(260.0, 880.0));
            ui.horizontal(|ui| {
                ui.heading(tr(match overlay {
                    Overlay::Settings => "Settings",
                    Overlay::Share => "Share",
                    Overlay::Help => "Keyboard shortcuts",
                    Overlay::Launcher => "Open a terminal or agent",
                }));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::chrome::icon_button(
                        ui,
                        crate::icons::Icon::Close,
                        tr("Close"),
                        false,
                        false,
                    )
                    .clicked()
                    {
                        close = true;
                    }
                });
            });
            ui.separator();
            let height = (ctx.content_rect().height() * 0.86 - 65.0).clamp(150.0, 615.0);
            if overlay == Overlay::Launcher {
                close |= self.launcher_ui(ui);
            } else if overlay == Overlay::Help {
                close |= self.help.show(
                    ui,
                    self.state.language == crate::i18n::Language::French,
                    &[
                        "Ctrl+Shift+T",
                        "Ctrl+Shift+W",
                        "Ctrl+Shift+Space",
                        "Ctrl+Shift+P",
                        "Ctrl+Shift+J",
                        "Ctrl+Shift+E",
                        "Ctrl+Shift+O",
                        "Alt+Arrows",
                        "Esc",
                        "Ctrl+S",
                        "Ctrl+W",
                        "Ctrl+Shift+K",
                        "Ctrl+Shift+X",
                        "Ctrl+Shift+G",
                        "Ctrl+,",
                        "F1",
                    ],
                );
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
    fn show_settings(&mut self, ctx: &egui::Context) {
        use crate::icons::Icon;
        use crate::settings_model::{SettingsGroupId as Group, SettingsLang};
        use crate::settings_nav::{SettingsNavAction, SettingsNavEntry, SettingsNavInput};
        let sections = [
            (
                SettingsSection::Agents,
                "agents",
                "Agent CLIs",
                Group::Ai,
                Icon::Agent,
                "agent codex claude",
            ),
            (
                SettingsSection::Appearance,
                "appearance",
                "Appearance",
                Group::Workspace,
                Icon::Appearance,
                "theme language langue police font",
            ),
            #[cfg(windows)]
            (
                SettingsSection::Passwords,
                "passwords",
                "Passwords",
                Group::Workspace,
                Icon::Settings,
                "password mot passe vault coffre csv import",
            ),
            #[cfg(windows)]
            (
                SettingsSection::Account,
                "tessel-account",
                "Account",
                Group::Workspace,
                Icon::Account,
                "account compte login connexion tessel",
            ),
            (
                SettingsSection::Hosts,
                "ssh",
                "SSH hosts",
                Group::Workflows,
                Icon::Cable,
                "ssh remote distant server serveur",
            ),
            (
                SettingsSection::Backup,
                "backup",
                "Settings backup",
                Group::Workflows,
                Icon::Files,
                "backup export import sauvegarde restore restaurer",
            ),
        ];
        let entries: Vec<_> = sections
            .iter()
            .map(|(_, id, title, group, icon, keywords)| SettingsNavEntry {
                id: (*id).into(),
                title: tr(title).into(),
                group: *group,
                keywords: (*keywords).into(),
                icon: *icon,
            })
            .collect();
        let selected = sections
            .iter()
            .find(|(section, ..)| *section == self.settings_section)
            .map(|(_, id, ..)| *id)
            .unwrap_or("appearance");
        let mut close = false;
        let response = crate::settings_surface::show(ctx, |ui, nav_width| {
            let height = ui.available_height();
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal_top(|ui| {
                egui::Frame::NONE
                    .fill(crate::theme::chrome(ctx))
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 12,
                        top: 14,
                        bottom: 18,
                    })
                    .show(ui, |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_width(nav_width - 24.0);
                            ui.set_min_height((height - 32.0).max(0.0));
                            if self.restore_preview.is_some() {
                                ui.disable();
                            }
                            let lang = if self.state.language == crate::i18n::Language::French {
                                SettingsLang::Fr
                            } else {
                                SettingsLang::En
                            };
                            match self.settings_nav.show(
                                ui,
                                SettingsNavInput {
                                    entries: &entries,
                                    selected,
                                    lang,
                                },
                            ) {
                                Some(SettingsNavAction::Back) => close = true,
                                Some(SettingsNavAction::Select(id)) => {
                                    if let Some((section, ..)) =
                                        sections.iter().find(|(_, known, ..)| *known == id)
                                    {
                                        self.settings_section = *section;
                                    }
                                }
                                None => {}
                            }
                        });
                    });
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        crate::settings_surface::body(ui, |ui| match self.settings_section {
                            SettingsSection::Appearance => self.appearance_settings(ui),
                            SettingsSection::Hosts => self.host_settings(ui),
                            SettingsSection::Agents => self.agent_settings(ui),
                            SettingsSection::Backup => self.backup_controls(ui),
                            #[cfg(windows)]
                            SettingsSection::Passwords => self.vault.show(ui, &mut self.browser),
                            #[cfg(windows)]
                            SettingsSection::Account => self.account.show(ui),
                        })
                    },
                );
            });
        });
        if self.restore_preview.is_none() && (close || response.should_close()) {
            self.overlay = None;
            self.settings_nav.reset();
        }
    }

    fn confirmations(&mut self, ctx: &egui::Context) {
        if let Some(index) = self.pending_close {
            egui::Modal::new(egui::Id::new("unsaved-file")).show(ctx, |ui| {
                ui.label(tr("Save changes before closing this file?"));
                ui.label(self.documents[index].path.display().to_string());
                ui.horizontal(|ui| {
                    if ui.button(tr("Cancel")).clicked() {
                        self.pending_close = None;
                    }
                    if ui.button(tr("Save and close")).clicked() {
                        match self.documents[index].save() {
                            Ok(()) => {
                                self.remove_document(index);
                                self.pending_close = None;
                            }
                            Err(e) => self.error = e,
                        }
                    }
                    if ui.button(tr("Discard changes")).clicked() {
                        self.remove_document(index);
                        self.pending_close = None;
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
    fn sync_git_project(&mut self) {
        let owner = self.state.active().map(|project| crate::git_result::Owner {
            project: project.id,
            root: project.path.clone(),
        });
        if self.scm_state.select(owner.clone()) {
            self.scm_panel.reset();
        }
        if self.git_result.select(owner) {
            self.git_path.clear();
            self.commit_message.clear();
        }
    }
    fn git_start(&mut self, args: Vec<String>, input: Option<Vec<u8>>) {
        self.git_start_work(move |root| {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            crate::git::run(&root, &args, input.as_deref())
        });
    }
    fn git_start_work(
        &mut self,
        work: impl FnOnce(PathBuf) -> Result<String, String> + Send + 'static,
    ) {
        if self.git_result.busy() || self.scm_state.busy() {
            return;
        }
        let Some(root) = self.state.active().map(|p| p.path.clone()) else {
            self.error = "Select a project first.".into();
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let owner = crate::git_result::Owner {
            project: self.state.active_project.expect("active project checked"),
            root: root.clone(),
        };
        if !self.git_result.start(owner, receiver) {
            return;
        }
        self.scm_state.result = None;
        thread::spawn(move || {
            let _ = sender.send(work(root));
        });
    }
    fn git_view(&mut self, ui: &mut egui::Ui) {
        self.sync_git_project();
        ui.collapsing(tr("Git commands"), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("legacy-git-commands")
                .max_height(240.0)
                .show(ui, |ui| self.git_commands(ui));
        });
        use crate::scm_changes_panel::{
            ScmCapabilities, ScmChangesAction, ScmChangesInput, ScmStatusSnapshot,
        };
        let status = self
            .scm_state
            .result
            .as_ref()
            .and_then(|result| result.as_ref().ok());
        let error = self
            .scm_state
            .result
            .as_ref()
            .and_then(|result| result.as_ref().err());
        let action = self.scm_panel.show(
            ui,
            ScmChangesInput {
                project_selected: self.state.active().is_some(),
                loaded: self.scm_state.result.is_some(),
                busy: self.scm_state.busy() || self.git_result.busy(),
                read_only: false,
                french: self.state.language == crate::i18n::Language::French,
                error: error.map(String::as_str),
                status: status.map(|parsed| ScmStatusSnapshot::from_parsed(true, parsed)),
                active_entry: None,
                capabilities: ScmCapabilities {
                    can_refresh: true,
                    can_open_file: true,
                    can_stage: true,
                    can_unstage: true,
                    ..Default::default()
                },
            },
        );
        match action {
            Some(ScmChangesAction::Refresh) if !self.git_result.busy() => self.scm_state.refresh(),
            Some(ScmChangesAction::Unstage(paths)) if !self.git_result.busy() => {
                match self.scm_state.unstage_paths(&paths) {
                    Ok(paths) => {
                        self.git_start_work(move |root| crate::git::unstage_files(&root, &paths))
                    }
                    Err(error) => self.error = error,
                }
            }
            Some(ScmChangesAction::Stage(paths)) if !self.git_result.busy() => {
                match self.scm_state.stage_paths(&paths) {
                    Ok(paths) => {
                        self.git_start_work(move |root| crate::git::stage_files(&root, &paths))
                    }
                    Err(error) => self.error = error,
                }
            }
            Some(ScmChangesAction::OpenFile(path)) => {
                if let Some(path) = self.scm_state.file_path(&path) {
                    self.open_document(path);
                }
            }
            _ => {}
        }
    }

    fn git_commands(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(tr("Source control"))
                .strong()
                .size(13.0),
        );
        ui.small(tr("On-demand Git operations. Output is capped at 2 MiB; operations time out after 20 seconds."));
        ui.add_enabled_ui(!self.git_result.busy() && !self.scm_state.busy(), |ui| {
            ui.horizontal_wrapped(|ui| {
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
            ui.vertical(|ui| {
                ui.label(tr("Relative file path"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.git_path)
                        .desired_width(ui.available_width())
                        .char_limit(4096),
                );
                if ui.button(tr("Stage file")).clicked() && !self.git_path.trim().is_empty() {
                    let path = self.git_path.clone();
                    self.git_start_work(move |root| crate::git::stage_file(&root, &path));
                }
            });
            ui.vertical(|ui| {
                ui.label(tr("Commit message"));
                ui.add(
                    egui::TextEdit::multiline(&mut self.commit_message)
                        .desired_width(ui.available_width())
                        .desired_rows(3)
                        .char_limit(16384),
                );
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
        if let Some(owner) = self.git_result.pending_owner() {
            ui.horizontal_wrapped(|ui| {
                ui.spinner();
                let name = self
                    .state
                    .projects
                    .iter()
                    .find(|project| project.id == owner.project)
                    .map(|project| project.name.as_str())
                    .unwrap_or_else(|| tr("Project"));
                ui.small(format!("{}: {name}", tr("Git in progress")))
                    .on_hover_text(owner.root.display().to_string());
            });
        }
        egui::ScrollArea::both().show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.git_result.output)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
        });
    }
}
impl eframe::App for Tessel {
    #[cfg(feature = "visual-test")]
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        if std::env::var_os("TESSEL_SETTLED_SCREENSHOT").is_some() {
            // Automated captures must never act on typing/clicks intended for another window.
            input
                .events
                .retain(|event| matches!(event, egui::Event::Screenshot { .. }));
            input.modifiers = egui::Modifiers::NONE;
            input.dropped_files.clear();
            input.hovered_files.clear();
        }
    }

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
        if self.finder.is_open()
            && ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::J)
            })
        {
            self.finder.close();
        }
        let confirmation_open = self.finder.is_open()
            || self.toolbar_menu_open
            || self.pending_close.is_some()
            || self.pending_exit
            || self.file_action.is_some()
            || self.restore_preview.is_some();
        let keyboard_free = !ctx.wants_keyboard_input() || terminal_focused;
        if !confirmation_open
            && !self.command_palette.open
            && matches!(self.overlay, None | Some(Overlay::Help))
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F1))
        {
            if self.overlay == Some(Overlay::Help) {
                self.overlay = None;
            } else {
                self.open_help();
            }
        }
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
                if ctx.input_mut(|i| {
                    i.consume_key(
                        egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                        egui::Key::Space,
                    )
                }) {
                    self.open_launcher();
                }
                if ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::J)
                }) {
                    self.open_finder();
                }
                for (key, dir) in [
                    (egui::Key::E, crate::pane_grid::SplitDir::Row),
                    (egui::Key::O, crate::pane_grid::SplitDir::Col),
                ] {
                    if ctx.input_mut(|i| {
                        i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, key)
                    }) && let Some(target) = self.active_session
                    {
                        self.new_session_placed(Launch::Shell, Some((target, dir)));
                    }
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    let key = crate::terminal_layout::project_key(self.state.active_project);
                    if let Some(layout) = self.state.terminal_layouts.get_mut(&key)
                        && layout.maximized.take().is_some()
                    {
                        self.grid_dirty = true;
                    }
                }
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
        if take_close_pane_shortcut(
            ctx,
            keyboard_free
                && !confirmation_open
                && !self.command_palette.open
                && self.overlay.is_none()
                && self.view == View::Work,
        ) && let Some(id) = self.active_session.filter(|id| {
            self.sessions
                .iter()
                .any(|s| s.id == *id && s.project == self.state.active_project)
        }) {
            self.close_session(id);
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
        self.toolbar_menu_open = false;
        egui::TopBottomPanel::top("toolbar")
            .exact_height(40.0)
            .frame(
                egui::Frame::default()
                    .fill(crate::theme::chrome(ctx))
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ctx, |ui| {
                use crate::{chrome, icons::Icon};
                #[allow(unused_mut)]
                let mut toolbar_room = ui.max_rect();
                #[cfg(windows)]
                if self.window_frame.is_some() {
                    toolbar_room.max.x -= crate::window_frame::CONTROL_WIDTH + 2.0;
                }
                let regions = chrome::regions(toolbar_room);
                #[cfg(windows)]
                if let Some(frame) = &self.window_frame {
                    frame.show(ui, [regions.search, regions.actions]);
                }
                chrome::brand(ui, regions.brand);
                if regions.search.width() >= 40.0
                    && chrome::command_button(ui, regions.search).clicked()
                {
                    self.command_palette.toggle();
                }
                let mut actions = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(regions.actions)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                let ui = &mut actions;
                ui.spacing_mut().item_spacing.x = 2.0;
                if chrome::icon_button(ui, Icon::Plus, tr("Open terminal"), false, false).clicked()
                {
                    self.new_session(Launch::Shell);
                }
                let launcher = chrome::icon_button(
                    ui,
                    Icon::ChevronDown,
                    tr("Open a terminal or agent"),
                    false,
                    true,
                );
                if launcher.clicked() {
                    self.launcher.reset();
                }
                let popup = egui::Popup::menu(&launcher)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
                let shown = popup.show(|ui| {
                    if self.launcher_ui(ui) {
                        ui.close();
                    }
                });
                self.toolbar_menu_open |= shown.is_some();
                ui.separator();
                if chrome::icon_button(ui, Icon::Share, tr("Share"), self.share.hosting(), false)
                    .clicked()
                {
                    self.overlay = Some(Overlay::Share);
                }
                let layout = chrome::icon_button(ui, Icon::Layout, tr("Layout"), false, false);
                let popup = egui::Popup::menu(&layout);
                let shown = popup.show(|ui| {
                    ui.checkbox(&mut self.state.panels.left_open, tr("Projects"));
                    ui.checkbox(&mut self.state.panels.right_open, tr("Side panel"));
                    if ui.button(tr("Arrange panes automatically")).clicked() {
                        self.tile_sessions(None);
                        ui.close();
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
                            self.tile_sessions(Some(columns));
                        }
                    }
                });
                self.toolbar_menu_open |= shown.is_some();
                let left_icon = if self.state.panels.left_open {
                    Icon::PanelLeftOpen
                } else {
                    Icon::PanelLeftClosed
                };
                if chrome::icon_button(ui, left_icon, tr("Toggle left sidebar"), false, false)
                    .clicked()
                {
                    self.state.panels.left_open = !self.state.panels.left_open;
                }
                let right_icon = if self.state.panels.right_open {
                    Icon::PanelRightOpen
                } else {
                    Icon::PanelRightClosed
                };
                if chrome::icon_button(ui, right_icon, tr("Toggle right sidebar"), false, false)
                    .clicked()
                {
                    self.state.panels.right_open = !self.state.panels.right_open;
                }
                if chrome::icon_button(ui, Icon::Settings, tr("Settings"), false, false).clicked() {
                    self.overlay = Some(Overlay::Settings);
                }
                if chrome::icon_button(ui, Icon::Help, tr("Keyboard shortcuts"), false, false)
                    .clicked()
                {
                    self.open_help();
                }
            });
        egui::TopBottomPanel::bottom("status")
            .exact_height(24.0)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::chrome(ctx))
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(ctx, |ui| {
                let room = ui.available_rect_before_wrap();
                ui.painter()
                    .hline(room.x_range(), room.top(), ui.visuals().window_stroke);
                if self.store.is_some() && self.error.is_empty() {
                    let target = if self.view == View::Work {
                        self.sessions
                            .iter()
                            .find(|s| {
                                Some(s.id) == self.active_session
                                    && s.project == self.state.active_project
                            })
                            .map_or("", |s| s.title.as_str())
                    } else {
                        ""
                    };
                    let count = self
                        .sessions
                        .iter()
                        .filter(|s| s.project == self.state.active_project)
                        .count();
                    let path = self
                        .state
                        .active()
                        .map(|p| p.path.display().to_string())
                        .unwrap_or_default();
                    let build = egui::Rect::from_min_max(
                        egui::pos2((room.right() - 120.0).max(room.left()), room.top()),
                        room.max,
                    );
                    ui.scope_builder(egui::UiBuilder::new().max_rect(build), |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.small(env!("TESSEL_BUILD"))
                                .on_hover_text(tr("Rust native · development"));
                        });
                    });
                    let left = egui::Rect::from_min_max(
                        room.min,
                        egui::pos2((build.left() - 12.0).max(room.left()), room.bottom()),
                    );
                    ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
                        crate::status_strip::show(
                            ui,
                            target,
                            count,
                            &path,
                            self.state.language == crate::i18n::Language::French,
                        );
                    });
                    return;
                }
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
                && self.overlay.is_none()
                && !self.toolbar_menu_open
                && !self.finder.is_open(),
        );
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.finder.is_open()
                || self.toolbar_menu_open
                || self.command_palette.open
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
                View::Remote => self.remote.show(ui, &self.state.hosts),
                #[cfg(windows)]
                View::Browser => {
                    if !self.pending_exit
                        && self.pending_close.is_none()
                        && self.file_action.is_none()
                        && !self.command_palette.open
                        && self.overlay.is_none()
                        && !self.toolbar_menu_open
                        && !self.finder.is_open()
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
        if let Some(path) = self
            .finder
            .show(ctx, self.state.active().map(|p| p.path.as_path()))
        {
            self.open_document(path);
        }
        self.show_overlay(ctx);
        self.show_command_palette(ctx);
        #[cfg(feature = "visual-test")]
        if std::env::args().any(|a| a == "--pane-menu-fixture") {
            egui::Window::new("Pane menu fixture").show(ctx, |ui| {
                let _ = crate::pane_header::show(
                    ui,
                    &crate::pane_header::MenuState {
                        title: "Terminal",
                        maximized: false,
                        multiple_panes: true,
                        terminal_output: true,
                        language: if self.state.language == crate::i18n::Language::French {
                            crate::pane_menu_model::PaneMenuLang::Fr
                        } else {
                            crate::pane_menu_model::PaneMenuLang::En
                        },
                    },
                );
            });
        }
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

fn take_close_pane_shortcut(ctx: &egui::Context, allowed: bool) -> bool {
    take_close_shortcut(ctx, allowed, egui::Modifiers::CTRL | egui::Modifiers::SHIFT)
}

fn take_close_shortcut(ctx: &egui::Context, allowed: bool, modifiers: egui::Modifiers) -> bool {
    if !allowed {
        return false;
    }
    ctx.input_mut(|input| {
        let fresh = input.events.iter().any(|event| matches!(event,
            egui::Event::Key { key: egui::Key::W, pressed: true, repeat: false, modifiers: actual, .. }
                if actual.matches_exact(modifiers)));
        input.events.retain(|event| !matches!(event,
            egui::Event::Key { key: egui::Key::W, pressed: true, modifiers: actual, .. }
                if actual.matches_exact(modifiers)));
        fresh
    })
}

fn document_toolbar(ui: &mut egui::Ui, d: &mut Document, readable: bool) -> bool {
    ui.horizontal(|ui| {
        if !d.media {
            let save_clicked = ui.button(tr("Save (Ctrl+S)")).clicked();
            if ui.is_enabled()
                && (save_clicked
                    || ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S)))
            {
                d.error = d.save().err().unwrap_or_default();
            }
        }
        if readable {
            ui.selectable_value(&mut d.preview_mode, false, tr("Source"));
            ui.selectable_value(&mut d.preview_mode, true, tr("Readable preview"));
        }
        let close = ui.button(tr("Close")).clicked()
            || take_close_shortcut(ui.ctx(), ui.is_enabled(), egui::Modifiers::CTRL);
        ui.small(d.path.display().to_string());
        close
    })
    .inner
}

fn active_after_close(active: usize, removed: usize, remaining: usize) -> usize {
    let next = if removed < active {
        active.saturating_sub(1)
    } else {
        active
    };
    next.min(remaining.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_documents_ignores_held_keys_and_pane_close_modifiers() {
        let ctx = egui::Context::default();
        let frame = |pressed, modifiers, allowed| {
            let mut close = false;
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::W,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    close = take_close_shortcut(ctx, allowed, egui::Modifiers::CTRL);
                },
            );
            close
        };
        assert!(frame(true, egui::Modifiers::CTRL, true));
        assert!(!frame(true, egui::Modifiers::CTRL, true));
        assert!(!frame(false, egui::Modifiers::CTRL, true));
        assert!(!frame(
            true,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
            true
        ));
        assert!(!frame(false, egui::Modifiers::CTRL, true));
        assert!(!frame(true, egui::Modifiers::CTRL, false));
        assert!(!frame(true, egui::Modifiers::CTRL, true));
        assert!(!frame(false, egui::Modifiers::CTRL, true));
        assert!(frame(true, egui::Modifiers::CTRL, true));
    }

    #[test]
    fn closing_a_pane_requires_a_fresh_press_and_an_unblocked_workspace() {
        let ctx = egui::Context::default();
        let frame = |pressed: bool, allowed: bool| {
            let mut close = false;
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::W,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    close = take_close_pane_shortcut(ctx, allowed);
                },
            );
            close
        };
        assert!(!frame(true, false));
        assert!(!frame(true, true)); // Holding the key after a modal closes is not a new command.
        assert!(!frame(false, true));
        assert!(frame(true, true));
        assert!(!frame(true, true)); // Holding W must not close successive panes.
        assert!(!frame(false, true));
        assert!(frame(true, true));
    }

    #[test]
    fn document_shortcuts_cannot_save_or_close_behind_a_modal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("example.txt");
        std::fs::write(&path, "original").unwrap();
        let mut document = Document::open(dir.path(), &path).unwrap();
        document.text = "edited".into();
        let ctx = egui::Context::default();
        let mut frame = |key, pressed, enabled| {
            let mut close = false;
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL,
                }],
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        close = document_toolbar(ui, &mut document, false);
                    });
                });
            });
            close
        };
        assert!(!frame(egui::Key::S, true, false));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert!(!frame(egui::Key::W, true, false));
        assert!(!frame(egui::Key::W, true, true));
        assert!(!frame(egui::Key::W, false, true));
        assert!(frame(egui::Key::W, true, true));
        assert!(!frame(egui::Key::S, true, true));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited");
    }

    #[test]
    fn closing_tabs_preserves_the_selected_document_or_its_nearest_neighbor() {
        assert_eq!(active_after_close(2, 0, 3), 1);
        assert_eq!(active_after_close(0, 2, 3), 0);
        assert_eq!(active_after_close(1, 1, 3), 1);
        assert_eq!(active_after_close(3, 3, 3), 2);
        assert_eq!(active_after_close(0, 0, 0), 0);
    }
    #[test]
    fn native_panes_cannot_start_pty_workers() {
        for launch in [Launch::CodexChat, Launch::ClaudeChat] {
            let mut session = Session {
                id: 1,
                project: Some(1),
                title: "Native".into(),
                launch,
                cwd: PathBuf::from("unused"),
                terminal: None,
                loading: None,
                error: String::new(),
            };
            session.start();
            assert!(session.loading.is_none());
            assert!(session.terminal.is_none());
            assert!(session.error.is_empty());
        }
    }
    #[test]
    fn disconnected_terminal_startup_releases_loading_and_can_receive_a_retry_result() {
        let (sender, receiver) = mpsc::channel();
        let mut session = Session {
            id: 1,
            project: Some(1),
            title: "Fixture".into(),
            launch: Launch::Shell,
            cwd: PathBuf::from("unused"),
            terminal: None,
            loading: Some(receiver),
            error: String::new(),
        };
        session.poll();
        assert!(session.loading.is_some());
        assert!(session.error.is_empty());
        drop(sender);
        session.poll();
        assert!(session.loading.is_none());
        assert!(session.error.contains("stopped without a result"));
        let (sender, receiver) = mpsc::channel();
        session.loading = Some(receiver);
        sender.send(Err("explicit startup error".into())).unwrap();
        session.poll();
        assert!(session.loading.is_none());
        assert_eq!(session.error, "explicit startup error");
        assert!(session.terminal.is_none());
    }
}
