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

struct Restart {
    result: Receiver<Result<(), (Terminal, String)>>,
    preferences: Option<crate::agent_preferences::Preferences>,
}
struct Session {
    last_model: Option<crate::pane_session_model::SavedPaneModel>,
    model_save_attempt: Option<std::time::Instant>,
    model_picker: crate::session_option_picker_ui::SessionOptionPickerUi,
    codex_seed: Option<crate::codex_terminal::Seed>,
    codex_restore: Option<crate::agent_preferences::Preferences>,
    codex_discovery: Option<crate::codex_terminal::Discovery>,
    restarting: Option<Restart>,
    conversation_id: Option<String>,
    settings_changed: bool,
    launch_fingerprint: Option<String>,
    session_options: Option<serde_json::Value>,
    pane_name: Option<String>,
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
    fn model_agent(&self) -> Option<&str> {
        match &self.launch {
            Launch::Agent(agent) if crate::agent_model_catalog::find(agent).is_some() => {
                Some(agent)
            }
            Launch::CodexChat => Some("codex"),
            Launch::ClaudeChat => Some("claude"),
            _ => None,
        }
    }
    fn has_model_choice(&self) -> bool {
        self.model_agent().is_some()
    }
    fn can_pick_model(&self) -> bool {
        self.loading.is_none()
            && self.restarting.is_none()
            && self.codex_restore.is_none()
            && self.has_model_choice()
    }
    fn poll_conversation(&mut self, claimed: &mut std::collections::HashSet<String>) -> bool {
        if (self.terminal.is_none() && self.codex_restore.is_none()) || self.restarting.is_some() {
            return false;
        }
        let Some(discovery) = &mut self.codex_discovery else {
            return false;
        };
        let Some(result) = discovery.poll(claimed) else {
            return false;
        };
        match result {
            Ok(Some(id)) if claimed.insert(id.clone()) => {
                self.conversation_id = Some(id);
                self.codex_seed = None;
                self.codex_discovery = None;
                if let Some(prefs) = self.codex_restore.take() {
                    self.start(Some(prefs));
                }
                true
            }
            Err(error) => {
                self.error = error;
                self.codex_discovery = None;
                self.codex_restore = None;
                false
            }
            Ok(None) if self.codex_restore.is_some() => {
                self.error = "The previous Codex conversation could not be found; no new conversation was started.".into();
                self.codex_restore = None;
                self.codex_discovery = None;
                false
            }
            Ok(None) if self.terminal.as_ref().is_some_and(Terminal::exited) => {
                self.codex_discovery = None;
                false
            }
            _ => false,
        }
    }
    fn can_restart(&self, prefs: Option<&crate::agent_preferences::Preferences>) -> bool {
        cfg!(windows)
            && self.loading.is_none()
            && self.restarting.is_none()
            && self.terminal.is_some()
            && match (&self.launch, prefs) {
                (Launch::Shell, _) => true,
                (Launch::Agent(agent), Some(prefs)) => {
                    self.conversation_id.is_some()
                        && (crate::claude_terminal::managed(agent, prefs)
                            || crate::codex_terminal::managed(agent, prefs))
                }
                _ => false,
            }
    }
    fn restart(&mut self, prefs: Option<crate::agent_preferences::Preferences>) {
        if !self.can_restart(prefs.as_ref()) {
            return;
        }
        let Some(mut terminal) = self.terminal.take() else {
            return;
        };
        let (sender, result) = mpsc::channel();
        self.restarting = Some(Restart {
            result,
            preferences: prefs,
        });
        self.error.clear();
        thread::spawn(move || {
            let result = match terminal.stop_for_restart() {
                Ok(()) => {
                    drop(terminal);
                    Ok(())
                }
                Err(error) => Err((terminal, error)),
            };
            let _ = sender.send(result);
        });
    }
    fn needs_restart(&self) -> bool {
        self.settings_changed
            && self.restarting.is_none()
            && self.loading.is_none()
            && self
                .terminal
                .as_ref()
                .is_some_and(|terminal| !terminal.exited())
    }
    fn launch_stale(&self, current: Option<&crate::agent_preferences::Preferences>) -> bool {
        if self.restarting.is_some()
            || self.loading.is_some()
            || self.terminal.as_ref().is_none_or(Terminal::exited)
        {
            return false;
        }
        let (Launch::Agent(id), Some(before), Some(current)) =
            (&self.launch, &self.launch_fingerprint, current)
        else {
            return false;
        };
        crate::launch_fingerprint::for_agent(id, current).is_some_and(|now| now != *before)
    }
    fn label(&self) -> &str {
        self.pane_name.as_deref().unwrap_or(&self.title)
    }
    fn start(&mut self, prefs: Option<crate::agent_preferences::Preferences>) {
        if self.codex_restore.is_some()
            || self.restarting.is_some()
            || self.launch.native_chat().is_some()
            || self.terminal.is_some()
            || self.loading.is_some()
        {
            return;
        }
        let launch = self.launch.clone();
        let managed = match (&launch, &prefs) {
            (Launch::Agent(agent), Some(prefs)) => crate::claude_terminal::managed(agent, prefs),
            _ => false,
        };
        if managed && self.conversation_id.is_none() {
            match crate::claude::uuid() {
                Ok(id) => self.conversation_id = Some(id),
                Err(error) => {
                    self.error = error;
                    return;
                }
            }
        }
        let codex_managed = match (&launch, &prefs) {
            (Launch::Agent(agent), Some(prefs)) => crate::codex_terminal::managed(agent, prefs),
            _ => false,
        };
        if !managed && !codex_managed && matches!(launch, Launch::Agent(_)) {
            // Explicit session/mode arguments may select a different thread.
            // The previous automatic ID no longer describes this process.
            self.conversation_id = None;
            self.codex_seed = None;
        }
        self.codex_discovery = None;
        if codex_managed && self.conversation_id.is_none() {
            if let Some(seed) = &self.codex_seed {
                match crate::codex_terminal::Discovery::restore(
                    prefs.as_ref().expect("Codex preferences"),
                    &self.cwd,
                    seed,
                ) {
                    Ok(discovery) => {
                        self.codex_discovery = Some(discovery);
                        self.codex_restore = prefs;
                        self.error.clear();
                    }
                    Err(error) => self.error = error,
                }
                return;
            }
            match crate::codex_terminal::Discovery::new(
                prefs.as_ref().expect("Codex preferences"),
                &self.cwd,
            ) {
                Ok(discovery) => {
                    self.codex_seed = Some(discovery.seed());
                    self.codex_discovery = Some(discovery);
                }
                Err(error) => {
                    self.error = error;
                    return;
                }
            }
        }
        let conversation = (managed || codex_managed)
            .then(|| self.conversation_id.clone())
            .flatten();
        self.settings_changed = false;
        self.launch_fingerprint = match (&self.launch, &prefs) {
            (Launch::Agent(id), Some(prefs)) => crate::launch_fingerprint::for_agent(id, prefs),
            _ => None,
        };
        let cwd = self.cwd.clone();
        let (sender, receiver) = mpsc::channel();
        self.loading = Some(receiver);
        self.error.clear();
        thread::spawn(move || {
            let effective = match (&launch, &prefs, conversation) {
                (Launch::Agent(agent), Some(prefs), Some(id)) if agent == "codex" => {
                    crate::codex_terminal::prepare(prefs, &cwd, &id).map(Some)
                }
                (Launch::Agent(agent), Some(prefs), Some(id)) => {
                    crate::claude_terminal::prepare(agent, prefs, &cwd, &id).map(Some)
                }
                _ => Ok(prefs),
            };
            let result = effective.and_then(|prefs| {
                let command = launch::command_with_preferences(&launch, &cwd, prefs.as_ref())?;
                let custom = prefs
                    .as_ref()
                    .is_some_and(|p| !crate::agent_env::trim(&p.command).is_empty());
                match &launch {
                    Launch::Agent(agent) if !custom => {
                        Terminal::spawn_agent_command(agent, command)
                    }
                    _ => Terminal::spawn_command(command),
                }
                .map_err(|e| e.to_string())
            });
            let _ = sender.send(result);
        });
    }
    fn poll(&mut self) -> bool {
        let mut changed = false;
        let stopped = match self
            .restarting
            .as_ref()
            .map(|pending| pending.result.try_recv())
        {
            Some(Ok(result)) => Some(Ok(result)),
            Some(Err(mpsc::TryRecvError::Disconnected)) => Some(Err(
                "Terminal restart stopped without confirmation; no replacement was started."
                    .to_string(),
            )),
            _ => None,
        };
        if let Some(result) = stopped {
            changed = true;
            let pending = self.restarting.take().expect("pending restart");
            match result {
                Ok(Ok(())) => {
                    self.start(pending.preferences);
                }
                Ok(Err((terminal, error))) => {
                    self.terminal = Some(terminal);
                    self.error = error;
                }
                Err(error) => self.error = error,
            }
        }
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
            changed = true;
            self.loading = None;
            match result {
                Ok(t) => {
                    self.error.clear();
                    self.terminal = Some(t);
                }
                Err(e) => {
                    self.error = e;
                    self.codex_discovery = None;
                    self.codex_seed = None;
                }
            }
        }
        if let Some(t) = &mut self.terminal {
            let was_exited = t.exited();
            t.poll();
            changed |= was_exited != t.exited();
        }
        changed
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
    RenameAgent,
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
struct RenameAgent {
    pane: u64,
    expected: String,
    draft: String,
    error: String,
    focused: bool,
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
    rename_agent: Option<RenameAgent>,
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
    agents: crate::agent_detection::Detection,
    model_discovery: Option<crate::agent_model_discovery::Discovery>,
    agents_panel: crate::agents_panel::AgentsPanel,
    agents_notice: Option<String>,
    host_panel: crate::ssh_hosts_panel::SshHostsPanel,
    host_notice: Option<(String, bool)>,
    host_import: Option<Receiver<crate::ssh_config::Discovery>>,
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
        let next_session = state.next_pane_id();
        let model_discovery = store
            .as_ref()
            .map(|s| crate::agent_model_discovery::Discovery::open(s.directory()));
        let mut app = Self {
            #[cfg(windows)]
            window_frame,
            state,
            store,
            error,
            sessions: vec![],
            next_session,
            active_session: None,
            documents: vec![],
            document_pending: Default::default(),
            active_document: 0,
            view: View::Work,
            overlay: None,
            rename_agent: None,
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
            agents: Default::default(),
            model_discovery,
            agents_panel: Default::default(),
            agents_notice: None,
            host_panel: Default::default(),
            host_notice: None,
            host_import: None,
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
                restarting: None,
                codex_discovery: None,
                codex_restore: None,
                model_picker: Default::default(),
                last_model: saved.last_model.clone(),
                model_save_attempt: None,
                settings_changed: false,
                codex_seed: saved.codex_seed.clone(),
                conversation_id: saved.conversation_id.clone(),
                launch_fingerprint: saved.launch_fingerprint.clone(),
                session_options: if saved.launch.native_chat().is_some() {
                    saved.native_options.clone()
                } else {
                    saved.session_options.clone()
                },
                pane_name: saved.pane_name.clone(),
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
                app.claude_chats.last_mut().unwrap().pane = app.active_session.unwrap();
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
                app.chats.last_mut().unwrap().pane = app.active_session.unwrap();
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
        #[cfg(feature = "visual-test")]
        let models_fixture = std::env::args().any(|arg| arg == "--models-fixture");
        #[cfg(not(feature = "visual-test"))]
        let models_fixture = false;
        if !models_fixture {
            app.detect_agents();
        }
        #[cfg(feature = "visual-test")]
        if models_fixture {
            let cursor = std::env::args().any(|a| a == "--cursor-models-fixture");
            let agent = if cursor { "cursor" } else { "codex" };
            app.agents = crate::agent_detection::Detection::model_picker_fixture(agent);
            app.agents_panel = crate::agents_panel::AgentsPanel::model_picker_fixture(agent);
            app.overlay = Some(Overlay::Settings);
            app.settings_section = SettingsSection::Agents;
            app.state.language = crate::i18n::Language::French;
            crate::i18n::set_language(app.state.language);
            let mut defaults = crate::agent_model_defaults::Defaults {
                model: Some(
                    if cursor {
                        "gpt-5.3-codex"
                    } else {
                        "gpt-5.6-sol"
                    }
                    .into(),
                ),
                ..Default::default()
            };
            defaults.set_option("effort", Some("high".into()));
            if cursor {
                defaults.set_option("fastMode", Some(true.into()));
            }
            app.state
                .agent_model_defaults
                .insert(agent.into(), defaults);
        }
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
                        restarting: None,
                        codex_discovery: None,
                        codex_restore: None,
                        model_picker: Default::default(),
                        last_model: None,
                        model_save_attempt: None,
                        settings_changed: false,
                        codex_seed: None,
                        conversation_id: None,
                        launch_fingerprint: None,
                        session_options: None,
                        pane_name: None,
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
                    if std::env::args().any(|arg| arg == "--restart-fixture") {
                        app.overlay = None;
                        let session = app.sessions.last_mut().expect("synthetic terminal");
                        session.launch = Launch::Agent("claude".into());
                        session.conversation_id =
                            Some("12345678-1234-4234-8234-123456789abc".into());
                        session.title = "Synthetic agent".into();
                        session.launch_fingerprint = crate::launch_fingerprint::for_agent(
                            "claude",
                            &crate::agent_preferences::Preferences {
                                args: "--model synthetic-previous-model".into(),
                                ..Default::default()
                            },
                        );
                        session.settings_changed = true;
                    }
                    if std::env::args().any(|arg| arg == "--model-observation-fixture") {
                        app.overlay = None;
                        let session = app.sessions.last_mut().expect("synthetic terminal");
                        session.launch = Launch::Agent("codex".into());
                        if let Some(terminal) = &mut session.terminal {
                            terminal.enable_model_fixture();
                        }
                    }
                    if let Some(root) = std::env::args()
                        .collect::<Vec<_>>()
                        .windows(2)
                        .find(|args| args[0] == "--model-file-fixture")
                        .map(|args| PathBuf::from(&args[1]))
                    {
                        app.overlay = None;
                        let session = app.sessions.last_mut().expect("synthetic terminal");
                        session.launch = Launch::Agent("codex".into());
                        session.conversation_id =
                            Some("12345678-1234-4234-8234-123456789abc".into());
                        if let Some(cwd) = root.parent() {
                            session.cwd = cwd.to_owned();
                            if let Some(terminal) = &mut session.terminal {
                                terminal.enable_model_file_fixture(&root, cwd);
                            }
                        }
                    }
                    if std::env::args().any(|arg| arg == "--pane-model-fixture") {
                        app.overlay = None;
                        let session = app.sessions.last_mut().expect("synthetic terminal");
                        session.launch = Launch::Agent("codex".into());
                        session.title = "Synthetic Codex".into();
                        session.model_picker.open_at_point(
                            &cc.egui_ctx,
                            egui::pos2(430.0, 110.0),
                            None,
                        );
                    }
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
                    restarting: None,
                    codex_discovery: None,
                    codex_restore: None,
                    model_picker: Default::default(),
                    last_model: None,
                    model_save_attempt: None,
                    settings_changed: false,
                    codex_seed: None,
                    conversation_id: None,
                    launch_fingerprint: None,
                    session_options: None,
                    pane_name: None,
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
        self.refresh_launch_status();
        normalize_session_names(&mut self.sessions);
        self.state.next_pane = self.next_session;
        self.state.sessions = self
            .sessions
            .iter()
            .map(|s| workspace::SavedSession {
                last_model: s.last_model.clone(),
                codex_seed: s.codex_seed.clone(),
                conversation_id: s.conversation_id.clone(),
                launch_fingerprint: s.launch_fingerprint.clone(),
                session_options: if s.launch.native_chat().is_some() {
                    None
                } else {
                    s.session_options.clone()
                },
                native_options: if s.launch.native_chat().is_some() {
                    s.session_options.clone()
                } else {
                    None
                },
                pane_name: s.pane_name.clone(),
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
    fn save_pane_model_choice(&mut self, id: u64, option: &str, value: Option<&serde_json::Value>) {
        let Some(session) = self
            .sessions
            .iter()
            .find(|s| s.id == id && s.can_pick_model())
        else {
            return;
        };
        let Some(store) = &self.store else {
            self.error = "Workspace storage is unavailable.".into();
            return;
        };
        if session.launch.native_chat().is_some() {
            match crate::native_chat_options::save_choice(&mut self.state, store, id, option, value)
            {
                Ok(values) => {
                    if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
                        session.session_options = values.clone();
                    }
                    if let Some(chat) = self.chats.iter_mut().find(|c| c.pane == id) {
                        chat.options =
                            crate::native_chat_options::Options::from_value(values.as_ref());
                        chat.options_commit = false;
                    }
                    if let Some(chat) = self.claude_chats.iter_mut().find(|c| c.pane == id) {
                        chat.options =
                            crate::native_chat_options::Options::from_value(values.as_ref());
                        chat.options_commit = false;
                    }
                }
                Err(error) => self.error = error,
            }
            return;
        }
        let Launch::Agent(agent) = &session.launch else {
            return;
        };
        let discovered = self.model_discovery.as_ref().and_then(|d| d.models(agent));
        match crate::pane_model_settings::save(
            &mut self.state,
            store,
            id,
            discovered,
            option,
            value,
        ) {
            Ok(values) => {
                if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
                    session.session_options = values;
                }
                self.refresh_launch_status();
            }
            Err(error) => self.error = error,
        }
    }
    fn detect_agents(&mut self) {
        self.agents.start(|| {
            let search = crate::agent_paths::SearchPath::current()?;
            launch::discover(&search)
        });
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
    fn refresh_launch_status(&mut self) {
        for session in &mut self.sessions {
            let prefs = match &session.launch {
                Launch::Agent(id) => Some(crate::agent_preferences::for_pane_launch(
                    &self.state,
                    id,
                    self.model_discovery.as_ref().and_then(|d| d.models(id)),
                    session.session_options.as_ref(),
                )),
                _ => None,
            };
            session.settings_changed = session.launch_stale(prefs.as_ref());
        }
    }
    fn poll(&mut self) {
        self.agents.poll();
        let mut launch_changed = self
            .model_discovery
            .as_mut()
            .is_some_and(|discovery| discovery.poll());
        self.poll_host_import();
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
            if let Some(id) = &chat.thread_id {
                match self
                    .state
                    .remember_native_thread(chat.pane, chat.project, "Codex", Some(id))
                {
                    Ok(changed) => chat_metadata_changed |= changed,
                    Err(error) => self.error = error,
                }
            }
        }
        for chat in &mut self.claude_chats {
            chat.poll();
            match self.state.remember_native_thread(
                chat.pane,
                chat.project,
                "Claude",
                chat.saved_session.as_deref(),
            ) {
                Ok(changed) => chat_metadata_changed |= changed,
                Err(error) => self.error = error,
            }
        }
        if chat_metadata_changed {
            self.save();
        }
        let mut terminal_changed = false;
        let mut claimed: std::collections::HashSet<String> = self
            .sessions
            .iter()
            .filter_map(|session| session.conversation_id.clone())
            .collect();
        for s in &mut self.sessions {
            terminal_changed |= s.poll();
            terminal_changed |= s.poll_conversation(&mut claimed);
        }
        if terminal_changed {
            self.save();
        }
        launch_changed |= terminal_changed;
        if launch_changed {
            self.refresh_launch_status();
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
        if launch.native_chat().is_some() && self.state.active_project.is_none() {
            self.error = tr("Select a project for native chat.").into();
            return;
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
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            settings_changed: false,
            codex_seed: None,
            conversation_id: None,
            launch_fingerprint: None,
            session_options: None,
            pane_name: None,
            id,
            project: self.state.active_project,
            title,
            launch,
            cwd,
            terminal: None,
            loading: None,
            error: String::new(),
        };
        let prefs = match &s.launch {
            Launch::Agent(id) => Some(crate::agent_preferences::for_pane_launch(
                &self.state,
                id,
                self.model_discovery.as_ref().and_then(|d| d.models(id)),
                s.session_options.as_ref(),
            )),
            _ => None,
        };
        s.start(prefs);
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
        #[cfg(windows)]
        self.agent_browsers.cancel_pane(id);
        match session.launch {
            Launch::CodexChat => self.chats.retain(|c| c.pane != id),
            Launch::ClaudeChat => self.claude_chats.retain(|c| c.pane != id),
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
        .filter(|(id, _)| match *id {
            "chat" => crate::agent_preferences::enabled(&self.state, "codex"),
            "claude-chat" => crate::agent_preferences::enabled(&self.state, "claude"),
            _ => true,
        })
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
                session.label(),
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
            title: session.label().into(),
            kind: match session.launch {
                Launch::Shell | Launch::Ssh(_) => SessionKind::Shell,
                Launch::Agent(_) => SessionKind::Agent,
                Launch::CodexChat | Launch::ClaudeChat => SessionKind::Chat,
            },
            sleeping: if let Some(provider) = session.launch.native_chat() {
                match provider {
                    "Codex" => !self.chats.iter().any(|c| c.pane == session.id && c.busy()),
                    "Claude" => !self
                        .claude_chats
                        .iter()
                        .any(|c| c.pane == session.id && c.busy()),
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
            for (name, installed) in self.agents.entries() {
                if *installed && crate::agent_preferences::enabled(&self.state, name) {
                    entries.push(Entry {
                        id: format!("agent:{name}"),
                        label: crate::agent_catalog::find(name)
                            .map_or(name.as_str(), |p| p.name)
                            .into(),
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
                    if !crate::agent_preferences::enabled(&self.state, id) {
                        continue;
                    }
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
                            .entries()
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
        // Each native pane owns its provider process and resume ID. No connection on restore.
        if let Some(project) = self.state.active() {
            for session in self
                .sessions
                .iter()
                .filter(|s| s.project == Some(project.id))
            {
                match session.launch {
                    Launch::CodexChat if !self.chats.iter().any(|c| c.pane == session.id) => {
                        let mut chat = crate::chat::Chat::new(project.id, project.path.clone());
                        chat.pane = session.id;
                        chat.options = crate::native_chat_options::Options::from_value(
                            session.session_options.as_ref(),
                        );
                        if let Some(thread) =
                            self.state.native_resume(session.id, project.id, "Codex")
                        {
                            chat.set_resume(thread.into());
                        }
                        self.chats.push(chat);
                    }
                    Launch::ClaudeChat
                        if !self.claude_chats.iter().any(|c| c.pane == session.id) =>
                    {
                        let mut chat = crate::claude::Chat::new(
                            project.id,
                            project.path.clone(),
                            self.state
                                .native_resume(session.id, project.id, "Claude")
                                .map(str::to_owned),
                        );
                        chat.pane = session.id;
                        chat.options = crate::native_chat_options::Options::from_value(
                            session.session_options.as_ref(),
                        );
                        self.claude_chats.push(chat);
                    }
                    _ => {}
                }
            }
        }
        let mut rename = None;
        let mut close = None;
        let mut split = None;
        let mut maximize = None;
        let mut arrange = false;
        let mut model_choice = None;
        let mut observed_updates = Vec::new();
        let mut native_options_changed = false;
        let picker_was_open = self.sessions.iter().any(|s| s.model_picker.is_open());
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
                                let pane_header_width = ui.available_width();
                                let pane_models = match &session.launch {
                                    Launch::Agent(agent) => crate::agent_model_catalog::models_for(agent, self.model_discovery.as_ref().and_then(|d| d.models(agent))),
                                    _ => Vec::new(),
                                };
                                let observed_model = match &session.launch {
                                    Launch::Agent(agent) => {
                                        session.terminal.as_mut().and_then(|terminal| terminal.header_model(agent, &pane_models, session.conversation_id.as_deref(), ui.ctx()))
                                    }
                                    _ => None,
                                };
                                if let Some(snapshot) = observed_model.as_ref().and_then(crate::pane_model_history::snapshot)
                                    && session.last_model.as_ref() != Some(&snapshot)
                                    && session.model_save_attempt.is_none_or(|time| time.elapsed() >= Duration::from_secs(5))
                                {
                                    session.model_save_attempt = Some(std::time::Instant::now());
                                    observed_updates.push((session.id, snapshot));
                                }
                                let observed_model = observed_model.or_else(|| session.last_model.as_ref().map(crate::pane_model_history::restored));
                                let conversation_id = session
                                    .launch
                                    .native_chat()
                                    .and_then(|provider| {
                                        session.project.and_then(|project| {
                                            self.state.native_resume(session.id, project, provider)
                                        })
                                    })
                                    .or(session.conversation_id.as_deref())
                                    .map(str::to_owned);
                                let pane_prefs = match &session.launch {
                                    Launch::Agent(agent) => {
                                        Some(crate::agent_preferences::for_pane_launch(
                                            &self.state,
                                            agent,
                                            self.model_discovery
                                                .as_ref()
                                                .and_then(|d| d.models(agent)),
                                            session.session_options.as_ref(),
                                        ))
                                    }
                                    _ => None,
                                };
                                let menu = crate::pane_header::MenuState {
                                    model_choice: session.has_model_choice(),
                                    model_pending: !session.can_pick_model(),
                                    model_manual: session.session_options.is_some(),
                                    model_text: match &session.launch {
                                        Launch::Agent(agent) => {
                                            let values = crate::pane_session_model::launch_session_values(session.session_options.as_ref(), &self.state.agent_model_defaults, agent);
                                            Some(crate::agent_model_defaults_ui::session_pill_label(&pane_models, values.as_ref(), self.state.language == crate::i18n::Language::French))
                                        }
                                        Launch::CodexChat | Launch::ClaudeChat => {
                                            let agent = session.model_agent().expect("native provider");
                                            let defaults = self.state.agent_model_defaults.get(agent).and_then(crate::agent_model_defaults::Defaults::values);
                                            let values = crate::native_chat_options::Options::from_value(session.session_options.as_ref()).effective(defaults.as_ref()).value();
                                            let models = crate::agent_model_catalog::models_for(agent, self.model_discovery.as_ref().and_then(|d| d.models(agent)));
                                            Some(crate::agent_model_defaults_ui::session_pill_label(&models, values.as_ref(), self.state.language == crate::i18n::Language::French))
                                        }
                                        _ => None,
                                    },
                                    can_restart: session.can_restart(pane_prefs.as_ref()),
                                    launch_stale: session.needs_restart(),
                                    session_id: conversation_id.as_deref(),
                                    title: session.pane_name.as_deref().unwrap_or(&session.title),
                                    maximized: maximized.is_some(),
                                    multiple_panes: ids.len() > 1,
                                    terminal_output: session.terminal.is_some(),
                                    can_rename: session.pane_name.is_some(),
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
                                let chip_width = if observed_model.is_some() && pane_header_width > crate::session_option_picker_ui::HEADER_MODEL_CHIP_HIDE_MAX_WIDTH {
                                    let text = crate::pane_session_model::model_text(observed_model.as_ref(), self.state.language == crate::i18n::Language::French);
                                    (ui.painter().layout_no_wrap(text, egui::FontId::proportional(10.5), ui.visuals().text_color()).size().x + 12.0).ceil().clamp(24.0, 140.0) + 4.0
                                } else { 0.0 };
                                let title_width = (ui.available_width() - 84.0 - chip_width).max(0.0);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(title_width, 22.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_min_width(title_width);
                                        let badge = crate::sidebar_model::PaneHeaderBadge::Apply
                                            .label(
                                                if self.state.language
                                                    == crate::i18n::Language::French
                                                {
                                                    crate::sidebar_model::SidebarLang::Fr
                                                } else {
                                                    crate::sidebar_model::SidebarLang::En
                                                },
                                            );
                                        let font = egui::FontId::proportional(11.0);
                                        let badge_width = ui
                                            .painter()
                                            .layout_no_wrap(
                                                badge.into(),
                                                font.clone(),
                                                ui.visuals().warn_fg_color,
                                            )
                                            .size()
                                            .x
                                            + 8.0;
                                        let show_badge = session.needs_restart()
                                            && title_width > badge_width + 80.0;
                                        let response = ui
                                            .allocate_ui_with_layout(
                                                [
                                                    if show_badge {
                                                        title_width - badge_width - 4.0
                                                    } else {
                                                        title_width
                                                    },
                                                    22.0,
                                                ]
                                                .into(),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.add(
                                                        egui::Label::new(session.label())
                                                            .truncate(),
                                                    )
                                                },
                                            )
                                            .inner;
                                        if show_badge {
                                            let text = egui::RichText::new(badge).font(font).color(ui.visuals().warn_fg_color);
                                            if menu.can_restart {
                                                if ui.add(egui::Button::new(text).frame(false)).clicked() {
                                                    action = Some(crate::pane_menu_model::PaneMenuAction::RestartApplySettings);
                                                }
                                            } else { ui.label(text); }
                                        }
                                        response
                                    },
                                )
                                .inner
                                .on_hover_text(session.label())
                                .context_menu(|ui| {
                                    action = crate::pane_header::show(ui, &menu);
                                });
                                if let Launch::Agent(agent) = &session.launch {
                                    let chip = crate::session_option_picker_ui::show_header_model_chip(ui, egui::Id::new(("terminal-model", id)), &crate::session_option_picker_ui::HeaderModelChipInput {
                                        pane_header_width, is_agent: true, detected: false, agent_id: agent,
                                        guest_runs: false, guest_model: None, agent_model: observed_model.as_ref(),
                                        capabilities: crate::session_option_picker_ui::SessionPickerCapabilities { can_open_picker: session.can_pick_model(), ..Default::default() },
                                        french: self.state.language == crate::i18n::Language::French,
                                    });
                                    if chip.clicked && let Some(rect) = chip.rect {
                                        session.model_picker.toggle_at_chip(ui.ctx(), rect, chip.id);
                                    }
                                }
                                let more = crate::pane_header::button(
                                    ui,
                                    crate::icons::Icon::PaneMore,
                                    tr("More options"),
                                );
                                egui::Popup::menu(&more).show(|ui| {
                                    if let Some(selected) = crate::pane_header::show(ui, &menu) {
                                        action = Some(selected);
                                    }
                                    if session.restarting.is_none()
                                        && session.loading.is_none()
                                        && session.launch.native_chat().is_none()
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
                                        PaneMenuAction::ChooseModel if session.can_pick_model() => {
                                            session.model_picker.open_at_point(ui.ctx(), more.rect.left_bottom(), Some(more.id));
                                        }
                                        PaneMenuAction::Restart
                                        | PaneMenuAction::RestartApplySettings => {
                                            session.restart(pane_prefs);
                                        }
                                        PaneMenuAction::CopySessionId => {
                                            if let Some(id) = &conversation_id {
                                                ui.ctx().copy_text(id.clone());
                                            }
                                        }
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
                                        PaneMenuAction::Rename if session.pane_name.is_some() => {
                                            rename = Some((id, session.label().to_owned()))
                                        }
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
                            if !ui.is_enabled() || !session.can_pick_model() {
                                session.model_picker.close();
                            } else if session.model_picker.is_open()
                                && let Some(agent) = session.model_agent().map(str::to_owned)
                            {
                                let agent = agent.as_str();
                                let native_chat = session.launch.native_chat().is_some();
                                let mut input = crate::session_option_picker_ui::SessionPickerInput::new(agent, &self.state.agent_model_defaults);
                                // This surface currently edits the next launch. Its option
                                // rows must follow that launch's inherited model, not an
                                // unrelated first catalogue row when no pane override exists.
                                let configured = if native_chat {
                                    let defaults = self.state.agent_model_defaults.get(agent).and_then(crate::agent_model_defaults::Defaults::values);
                                    crate::native_chat_options::Options::from_value(session.session_options.as_ref()).effective(defaults.as_ref()).value()
                                } else { crate::pane_session_model::launch_session_values(session.session_options.as_ref(), &self.state.agent_model_defaults, agent) };
                                let settings_model = configured.as_ref().and_then(|values| values.get("model")).and_then(serde_json::Value::as_str).map(|model| crate::pane_session_model::ShownAgentModel {
                                    model: model.into(), name: None, effort: None, chosen_effort: None,
                                    source: crate::pane_session_model::ModelSource::Settings, at: None,
                                });
                                input.native_chat = native_chat;
                                input.agent_model = settings_model.as_ref();
                                input.pane_running = session.terminal.as_ref().is_some_and(|terminal| !terminal.exited());
                                input.session_options = session.session_options.as_ref();
                                input.discovered_models = self.model_discovery.as_ref().and_then(|d| d.models(agent));
                                input.french = self.state.language == crate::i18n::Language::French;
                                input.capabilities = crate::session_option_picker_ui::SessionPickerCapabilities {
                                    can_open_picker: true,
                                    ..Default::default()
                                };
                                let result = session.model_picker.show_popover(ui.ctx(), egui::Id::new(("pane-model-picker", id)), &input);
                                if let Some(crate::session_option_picker_ui::SessionPickerEvent::Set { option_id, value }) = result.event {
                                    model_choice = Some((id, option_id, value));
                                }
                            }
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
                                                if let Some(chat) =
                                                    self.chats.iter_mut().find(|c| c.pane == id)
                                                {
                                                    chat.defaults = self.state.agent_model_defaults.get("codex").and_then(crate::agent_model_defaults::Defaults::values);
                                                    chat.show(ui);
                                                    if std::mem::take(&mut chat.options_commit) && chat.options.value() != session.session_options {
                                                        session.session_options = chat.options.value();
                                                        native_options_changed = true;
                                                    }
                                                }
                                            }
                                            Launch::ClaudeChat => {
                                                if let Some(chat) = self
                                                    .claude_chats
                                                    .iter_mut()
                                                    .find(|c| c.pane == id)
                                                {
                                                    chat.defaults = self.state.agent_model_defaults.get("claude").and_then(crate::agent_model_defaults::Defaults::values);
                                                    chat.show(ui);
                                                    if std::mem::take(&mut chat.options_commit) && chat.options.value() != session.session_options {
                                                        session.session_options = chat.options.value();
                                                        native_options_changed = true;
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    });
                            } else if let Some(terminal) = &mut session.terminal {
                                ui.add_enabled_ui(!picker_was_open && !session.model_picker.is_open(), |ui| terminal.show(ui, self.state.font_size));
                            } else if session.restarting.is_some() {
                                ui.spinner();
                                ui.label(tr("Stopping terminal processes…"));
                            } else if session.codex_restore.is_some() {
                                ui.spinner();
                                ui.label(tr("Finding the previous conversation…"));
                            } else if session.loading.is_some() {
                                ui.spinner();
                                ui.label(tr("Starting…"));
                            } else {
                                let wake = ui.button(tr("Wake / reconnect")).clicked();
                                let fresh = session.codex_seed.is_some() && !session.error.is_empty()
                                    && ui.button(tr("Start a new conversation")).clicked();
                                if fresh { session.codex_seed = None; }
                                if wake || fresh {
                                let prefs = match &session.launch {
                                    Launch::Agent(id) => {
                                        Some(crate::agent_preferences::for_pane_launch(
                                            &self.state,
                                            id,
                                            self.model_discovery
                                                .as_ref()
                                                .and_then(|d| d.models(id)),
                                            session.session_options.as_ref(),
                                        ))
                                    }
                                    _ => None,
                                };
                                session.start(prefs);
                                }
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
        if let Some((pane, name)) = rename {
            self.rename_agent = Some(RenameAgent {
                pane,
                expected: name.clone(),
                draft: name,
                error: String::new(),
                focused: false,
            });
            self.overlay = Some(Overlay::RenameAgent);
        }
        if let Some(id) = close {
            self.close_session(id);
        }
        if native_options_changed {
            self.save();
        }
        if let Some((id, option, value)) = model_choice {
            self.save_pane_model_choice(id, &option, value.as_ref());
        }
        if !observed_updates.is_empty()
            && let Some(store) = &self.store
        {
            match crate::pane_model_history::persist(&mut self.state, store, &observed_updates) {
                Ok(_) => {
                    for (id, model) in observed_updates {
                        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
                            session.last_model = Some(model);
                            session.model_save_attempt = None;
                        }
                    }
                }
                Err(error) => self.error = error,
            }
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
        self.host_import.is_none()
            && self.store.is_some()
            && !self.file_action_pending
            && !self.sessions.iter().any(|s| {
                s.terminal.is_some()
                    || s.loading.is_some()
                    || s.restarting.is_some()
                    || s.codex_restore.is_some()
            })
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
        state.migrate_native_threads();
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
        self.agents_panel = Default::default();
        self.agents_notice = None;
        self.host_panel.reset();
        self.host_notice = None;
        self.rename_agent = None;
        self.next_session = self.state.next_pane_id();
        for saved in &self.state.sessions {
            self.next_session = self.next_session.max(saved.id + 1);
            self.sessions.push(Session {
                restarting: None,
                codex_discovery: None,
                codex_restore: None,
                model_picker: Default::default(),
                last_model: saved.last_model.clone(),
                model_save_attempt: None,
                settings_changed: false,
                codex_seed: saved.codex_seed.clone(),
                conversation_id: saved.conversation_id.clone(),
                launch_fingerprint: saved.launch_fingerprint.clone(),
                session_options: if saved.launch.native_chat().is_some() {
                    saved.native_options.clone()
                } else {
                    saved.session_options.clone()
                },
                pane_name: saved.pane_name.clone(),
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
        use crate::ssh_hosts_panel::{
            SshHostRow, SshHostSpec, SshHostsAction, SshHostsCapabilities, SshHostsInput,
            SshHostsNotice, SshHostsNoticeTone,
        };
        let rows: Vec<_> = self
            .state
            .hosts
            .iter()
            .map(|host| SshHostRow {
                host: SshHostSpec {
                    alias: host.alias.clone(),
                    folder: host.folder.clone(),
                },
                // A running SSH process alone does not prove authenticated connectivity.
                status: None,
                error: None,
                busy: false,
            })
            .collect();
        let mut capabilities = SshHostsCapabilities::native_openssh();
        capabilities.can_import_config = self.store.is_some() && self.host_import.is_none();
        if self.store.is_none() {
            capabilities.can_add = false;
            capabilities.can_edit = false;
            capabilities.can_remove = false;
        }
        let action = self.host_panel.show(
            ui,
            SshHostsInput {
                hosts: &rows,
                loaded: true,
                loading: self.host_import.is_some(),
                load_error: None,
                notice: self.host_notice.as_ref().map(|(text, bad)| SshHostsNotice {
                    text,
                    tone: if *bad {
                        SshHostsNoticeTone::Bad
                    } else {
                        SshHostsNoticeTone::Ok
                    },
                }),
                french: self.state.language == crate::i18n::Language::French,
                capabilities,
            },
        );
        let to_host = |host: SshHostSpec| Host {
            alias: host.alias,
            folder: host.folder,
        };
        match action {
            Some(SshHostsAction::ImportConfig) if self.host_import.is_none() => {
                if let Some(base) = directories::BaseDirs::new() {
                    let home = base.home_dir().to_owned();
                    let (sender, receiver) = mpsc::channel();
                    self.host_import = Some(receiver);
                    self.host_notice = None;
                    thread::spawn(move || {
                        let _ = sender.send(crate::ssh_config::discover(&home));
                    });
                } else {
                    self.host_notice = Some((tr("User home directory unavailable").into(), true));
                }
            }
            Some(SshHostsAction::Save {
                original,
                replacement,
            }) => {
                let original = original.map(|(index, host)| (index, to_host(host)));
                let result = self.change_hosts(|hosts| {
                    crate::host_actions::save(
                        hosts,
                        original.as_ref().map(|(i, h)| (*i, h)),
                        to_host(replacement),
                    )
                });
                match result {
                    Ok(_) => {
                        self.host_panel.cancel_dialogs();
                        self.host_notice = Some((tr("SSH target saved").into(), false));
                    }
                    Err(error) => {
                        let error = tr(&error).to_owned();
                        self.host_panel.set_form_error(error.clone());
                        self.host_notice = Some((error, true));
                    }
                }
            }
            Some(SshHostsAction::Remove { index, expected }) => {
                match self.change_hosts(|hosts| {
                    crate::host_actions::remove(hosts, index, &to_host(expected)).map(|()| true)
                }) {
                    Ok(_) => self.host_notice = Some((tr("SSH target removed").into(), false)),
                    Err(error) => self.host_notice = Some((tr(&error).into(), true)),
                }
            }
            Some(SshHostsAction::Connect { index, host }) => {
                let host = to_host(host);
                if self.state.hosts.get(index) == Some(&host) {
                    self.new_session(Launch::Ssh(host));
                    self.overlay = None;
                } else {
                    self.host_notice =
                        Some((tr("SSH target changed; reopen its settings.").into(), true));
                }
            }
            _ => {}
        }
    }

    fn poll_host_import(&mut self) {
        let result = self
            .host_import
            .as_ref()
            .and_then(|receiver| match receiver.try_recv() {
                Ok(discovery) => Some(Ok(discovery)),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("SSH configuration import stopped without a result."))
                }
            });
        let Some(result) = result else {
            return;
        };
        self.host_import = None;
        match result {
            Ok(discovery) => {
                let mut added = 0;
                let saved = self.change_hosts(|hosts| {
                    added = crate::host_actions::import_aliases(hosts, &discovery.aliases)?;
                    Ok(added != 0)
                });
                match saved {
                    Ok(_) => {
                        let incomplete = discovery.truncated || discovery.skipped_files != 0;
                        let mut message = format!("{} {added}", tr("SSH targets imported:"));
                        if incomplete {
                            message.push(' ');
                            message.push_str(tr(
                                "Some SSH configuration files were skipped or exceeded limits.",
                            ));
                        }
                        self.host_notice = Some((message, incomplete));
                    }
                    Err(error) => self.host_notice = Some((tr(&error).into(), true)),
                }
            }
            Err(error) => self.host_notice = Some((tr(error).into(), true)),
        }
    }

    fn change_hosts(
        &mut self,
        change: impl FnOnce(&mut Vec<Host>) -> Result<bool, String>,
    ) -> Result<bool, String> {
        let store = self.store.as_ref().ok_or("Settings store unavailable")?;
        crate::host_actions::persist(&mut self.state, store, change)
    }

    fn agent_settings(&mut self, ui: &mut egui::Ui) {
        use crate::agents_panel::{Action, Input, Row};
        let rows: Vec<_> = self
            .agents
            .entries()
            .iter()
            .filter_map(|(id, available)| {
                Some(Row {
                    preset: crate::agent_catalog::find(id)?,
                    available: *available,
                    enabled: crate::agent_preferences::enabled(&self.state, id),
                    prefs: crate::agent_preferences::get(&self.state, id)
                        .cloned()
                        .unwrap_or_default(),
                    model_defaults: self
                        .state
                        .agent_model_defaults
                        .get(id)
                        .cloned()
                        .unwrap_or_default(),
                })
            })
            .collect();
        let action = self.agents_panel.show(
            ui,
            Input {
                rows: &rows,
                discovery: self.model_discovery.as_ref(),
                loaded: self.agents.loaded(),
                detecting: self.agents.detecting(),
                error: self.agents.error(),
                notice: self.agents_notice.as_deref(),
                has_project: self.state.active_project.is_some(),
                french: self.state.language == crate::i18n::Language::French,
                search: self.settings_nav.query(),
            },
        );
        if !self.save_agent_draft() {
            return;
        }
        match action {
            Some(Action::RefreshModels(id)) => {
                let command = crate::agent_preferences::get(&self.state, &id)
                    .map(|p| p.command.clone())
                    .unwrap_or_default();
                if let Some(discovery) = &mut self.model_discovery {
                    let ctx = ui.ctx().clone();
                    discovery.refresh(&id, &command, move || ctx.request_repaint());
                }
            }
            Some(Action::Detect) => {
                self.agents_notice = None;
                self.detect_agents();
            }
            Some(Action::SetEnabled(id, enabled)) => self.change_agent_visibility(&id, enabled),
            Some(Action::SetModelDefaults(id, defaults)) => {
                self.agents_notice = self
                    .store
                    .as_ref()
                    .ok_or_else(|| "Settings store unavailable".to_owned())
                    .and_then(|store| {
                        crate::agent_model_defaults::save(&mut self.state, store, &id, defaults)
                    })
                    .err();
                self.refresh_launch_status();
            }
            Some(Action::Terminal(id)) => {
                self.new_session(Launch::Agent(id));
                self.overlay = None;
            }
            Some(Action::NativeChat(id)) if self.state.active_project.is_some() => {
                match id.as_str() {
                    "codex" => {
                        self.new_session(Launch::CodexChat);
                        self.overlay = None;
                    }
                    "claude" => {
                        self.new_session(Launch::ClaudeChat);
                        self.overlay = None;
                    }
                    _ => {}
                }
            }
            Some(Action::Docs(id)) => {
                let available = rows.iter().any(|row| row.preset.id == id && row.available);
                let url = if available {
                    crate::agent_catalog::find(&id).and_then(|entry| entry.docs)
                } else {
                    crate::agent_installs::find(&id)
                        .and_then(|entry| {
                            entry
                                .docs_url
                                .as_deref()
                                .or(entry.install_source.as_deref())
                        })
                        .or_else(|| crate::agent_catalog::find(&id).and_then(|entry| entry.docs))
                };
                if let Some(url) = url {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                }
            }
            Some(Action::CopyInstall(id)) => {
                if let Some(text) =
                    crate::agent_installs::find(&id).and_then(|entry| entry.copy_text())
                {
                    ui.ctx().copy_text(text);
                    self.agents_notice = Some(tr("Install command copied.").into());
                }
            }
            _ => {}
        }
    }
    fn save_agent_draft(&mut self) -> bool {
        if let Some((id, prefs)) = self.agents_panel.take_save() {
            if crate::agent_preferences::get(&self.state, &id)
                .cloned()
                .unwrap_or_default()
                == prefs
            {
                return true;
            }
            self.agents_notice = self
                .store
                .as_ref()
                .ok_or_else(|| "Settings store unavailable".to_owned())
                .and_then(|store| {
                    crate::agent_preferences::save(&mut self.state, store, &id, prefs)
                })
                .err();
            self.refresh_launch_status();
            return self.agents_notice.is_none();
        }
        true
    }
    fn change_agent_visibility(&mut self, id: &str, enabled: bool) {
        let result = self
            .store
            .as_ref()
            .ok_or_else(|| "Settings store unavailable".to_owned())
            .and_then(|store| {
                crate::agent_preferences::set_enabled(&mut self.state, store, id, enabled)
            });
        self.agents_notice = result.err().map(|error| tr(&error).to_owned());
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
        let target_title = target.map(|s| s.label().to_owned());
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
            if !crate::agent_preferences::enabled(&self.state, id) {
                continue;
            }
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
        entries.extend(
            self.agents
                .entries()
                .iter()
                .filter(|(name, _)| crate::agent_preferences::enabled(&self.state, name))
                .map(|(name, available)| LauncherEntry {
                    item: LaunchMenuItem::Agent {
                        id: name.clone(),
                        name: crate::agent_catalog::find(name)
                            .map_or(name.as_str(), |p| p.name)
                            .into(),
                    },
                    available: *available,
                    reason: (!available).then(|| tr("not on PATH").into()),
                }),
        );
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
    fn rename_agent_dialog(&mut self, ctx: &egui::Context) {
        let Some(draft) = &mut self.rename_agent else {
            self.overlay = None;
            return;
        };
        let mut submit = false;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("rename-agent")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(220.0, 420.0));
            ui.heading(tr("Rename agent"));
            let field = ui.add(egui::TextEdit::singleline(&mut draft.draft).char_limit(60).desired_width(ui.available_width()));
            if !draft.focused { field.request_focus(); draft.focused = true; }
            if field.changed() { draft.error.clear(); }
            if !draft.error.is_empty() { ui.colored_label(ui.visuals().error_fg_color, &draft.error); }
            ui.horizontal(|ui| {
                if ui.button(tr("Cancel")).clicked() { cancel = true; }
                if ui.button(tr("Save")).clicked() { submit = true; }
            });
            let fresh_enter = ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Enter, pressed: true, repeat: false, modifiers, .. } if *modifiers == egui::Modifiers::NONE)));
            if (field.has_focus() || field.lost_focus()) && fresh_enter {
                ui.input_mut(|input| { input.consume_key(egui::Modifiers::NONE, egui::Key::Enter); });
                submit = true;
            }
        });
        if cancel || response.should_close() {
            self.rename_agent = None;
            self.overlay = None;
            return;
        }
        if submit {
            let pane = draft.pane;
            let expected = draft.expected.clone();
            let name = draft.draft.clone();
            match self.rename_agent_saved(pane, &expected, &name) {
                Ok(()) => {
                    self.rename_agent = None;
                    self.overlay = None;
                }
                Err(error) => {
                    if let Some(draft) = &mut self.rename_agent {
                        draft.error = tr(&error).into();
                    }
                }
            }
        }
    }

    fn rename_agent_saved(&mut self, pane: u64, expected: &str, name: &str) -> Result<(), String> {
        if !self
            .sessions
            .iter()
            .any(|s| s.id == pane && s.pane_name.as_deref() == Some(expected))
        {
            return Err(tr("Agent changed; reopen its menu.").into());
        }
        let mut names: Vec<_> = self
            .sessions
            .iter()
            .map(|s| crate::agent_names::NamedPane {
                id: s.id,
                number: s.id,
                named: s.pane_name.is_some(),
                name: s.pane_name.clone(),
            })
            .collect();
        crate::agent_names::rename_agent_name(&mut names, pane, name).map_err(|error| {
            match error {
                crate::agent_names::RenameError::Invalid => {
                    tr("Choose a valid agent name (1–60 characters).")
                }
                crate::agent_names::RenameError::Taken => {
                    tr("Another agent already uses this name.")
                }
            }
            .to_owned()
        })?;
        let name = names
            .into_iter()
            .find(|p| p.id == pane)
            .and_then(|p| p.name)
            .ok_or("Agent no longer exists")?;
        let mut next = self.state.clone();
        let saved = next
            .sessions
            .iter_mut()
            .find(|s| s.id == pane)
            .ok_or("Agent no longer exists")?;
        saved.pane_name = Some(name.clone());
        self.store
            .as_ref()
            .ok_or("Settings store unavailable")?
            .save(&next)?;
        self.state.sessions = next.sessions;
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == pane) {
            session.pane_name = Some(name);
        }
        Ok(())
    }

    fn show_overlay(&mut self, ctx: &egui::Context) {
        let Some(overlay) = self.overlay else {
            return;
        };
        if overlay == Overlay::RenameAgent {
            self.rename_agent_dialog(ctx);
            return;
        }
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
                    Overlay::RenameAgent => "Rename agent",
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
                "Agents",
                Group::Ai,
                Icon::Agent,
                "agent codex claude install detection visibility command",
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
        let host_dialog_open = self.settings_section == SettingsSection::Hosts
            && (self.host_panel.is_form_open() || self.host_panel.is_confirm_remove_open());
        let agents_escape_consumed = self.settings_section == SettingsSection::Agents
            && self.restore_preview.is_none()
            && self.agents_panel.handle_escape(ctx);
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
                            if self.restore_preview.is_some() || host_dialog_open {
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
                                        if self.settings_section == SettingsSection::Agents {
                                            self.agents_panel.save_open_draft();
                                        }
                                        if self.save_agent_draft() {
                                            self.settings_section = *section;
                                        }
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
        if self.restore_preview.is_none()
            && !host_dialog_open
            && !agents_escape_consumed
            && (close || response.should_close())
        {
            if self.settings_section == SettingsSection::Agents {
                self.agents_panel.save_open_draft();
            }
            if self.save_agent_draft() {
                self.overlay = None;
                self.settings_nav.reset();
            }
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
            let model_busy = self.model_discovery.as_ref().is_some_and(|d| d.active());
            egui::Modal::new(egui::Id::new("close-app")).show(ctx, |ui| {
                ui.label(tr("There are unsaved files or sessions. Closing ends sessions."));
                if ui.button(tr("Keep working")).clicked() {
                    self.pending_exit = false;
                }
                if self.file_action_pending { ui.label(tr("File operation in progress…")); }
                if self.account_busy() { ui.label(tr("Wait for the account request to finish before closing.")); }
                if model_busy { ui.label(if self.state.language == crate::i18n::Language::French { "Attendez la fin de la recherche et de la sauvegarde des modèles avant de fermer." } else { "Wait for model discovery and saving to finish before closing." }); }
                if self.remote.needs_close_confirmation() {ui.label(tr("Finish remote requests and save remote edits in Remote files before saving and closing."));}
                if ui.add_enabled(!model_busy && !self.remote.needs_close_confirmation() && !self.account_busy() && !self.file_action_pending,egui::Button::new(tr("Save all and close"))).clicked() {
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
                if ui.add_enabled(!model_busy && !self.remote.busy() && !self.account_busy() && !self.file_action_pending,egui::Button::new(tr("Discard edits and close"))).clicked() {
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
        for session in &mut self.sessions {
            if self.overlay.is_some()
                || self.command_palette.open
                || confirmation_open
                || self.view != View::Work
                || session.project != self.state.active_project
            {
                session.model_picker.close();
            }
        }
        let picker_escape = self.sessions.iter_mut().fold(false, |consumed, session| {
            session.model_picker.handle_escape(ctx) | consumed
        });
        let keyboard_free = !picker_escape
            && (!ctx.wants_keyboard_input() || terminal_focused)
            && !self.sessions.iter().any(|s| s.model_picker.is_open());
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
        if take_fresh_shortcut(
            ctx,
            keyboard_free
                && !confirmation_open
                && !self.command_palette.open
                && self.overlay.is_none()
                && self.view == View::Work,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
            egui::Key::R,
        ) && let Some(session) = self
            .sessions
            .iter_mut()
            .find(|s| Some(s.id) == self.active_session && s.project == self.state.active_project)
        {
            let prefs = match &session.launch {
                Launch::Agent(agent) => Some(crate::agent_preferences::for_pane_launch(
                    &self.state,
                    agent,
                    self.model_discovery.as_ref().and_then(|d| d.models(agent)),
                    session.session_options.as_ref(),
                )),
                _ => None,
            };
            session.restart(prefs);
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
                || self.model_discovery.as_ref().is_some_and(|d| d.active())
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
                            .map_or("", Session::label)
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
                        model_choice: true,
                        model_pending: false,
                        model_manual: true,
                        model_text: Some("synthetic-model".into()),
                        session_id: Some("12345678-1234-4234-8234-123456789abc"),
                        can_restart: true,
                        launch_stale: true,
                        maximized: false,
                        multiple_panes: true,
                        terminal_output: true,
                        can_rename: false,
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

fn normalize_session_names(sessions: &mut [Session]) {
    let mut panes: Vec<_> = sessions
        .iter()
        .map(|session| crate::agent_names::NamedPane {
            id: session.id,
            number: session.id,
            named: matches!(
                session.launch,
                Launch::Agent(_) | Launch::CodexChat | Launch::ClaudeChat
            ),
            name: session.pane_name.clone(),
        })
        .collect();
    crate::agent_names::ensure_agent_names(&mut panes);
    for (session, pane) in sessions.iter_mut().zip(panes) {
        session.pane_name = pane.name;
    }
}

fn take_close_pane_shortcut(ctx: &egui::Context, allowed: bool) -> bool {
    take_close_shortcut(ctx, allowed, egui::Modifiers::CTRL | egui::Modifiers::SHIFT)
}

fn take_close_shortcut(ctx: &egui::Context, allowed: bool, modifiers: egui::Modifiers) -> bool {
    take_fresh_shortcut(ctx, allowed, modifiers, egui::Key::W)
}
fn take_fresh_shortcut(
    ctx: &egui::Context,
    allowed: bool,
    modifiers: egui::Modifiers,
    key: egui::Key,
) -> bool {
    if !allowed {
        return false;
    }
    ctx.input_mut(|input| {
        let fresh = input.events.iter().any(|event| matches!(event,
            egui::Event::Key { key: actual_key, pressed: true, repeat: false, modifiers: actual, .. }
                if *actual_key == key && actual.matches_exact(modifiers)));
        input.events.retain(|event| !matches!(event,
            egui::Event::Key { key: actual_key, pressed: true, modifiers: actual, .. }
                if *actual_key == key && actual.matches_exact(modifiers)));
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
    fn restart_shortcut_requires_a_fresh_press_and_unblocked_workspace() {
        let ctx = egui::Context::default();
        let frame = |pressed, allowed| {
            let mut restart = false;
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::R,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    restart = take_fresh_shortcut(
                        ctx,
                        allowed,
                        egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                        egui::Key::R,
                    );
                },
            );
            restart
        };
        assert!(!frame(true, false));
        assert!(!frame(true, true));
        assert!(!frame(false, true));
        assert!(frame(true, true));
        assert!(!frame(true, true));
    }
    #[test]
    fn restart_failure_restores_the_old_terminal_without_starting_a_replacement() {
        let terminal = Terminal::spawn().unwrap();
        let (sender, receiver) = mpsc::channel();
        let mut session = Session {
            restarting: Some(Restart {
                result: receiver,
                preferences: None,
            }),
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            codex_seed: None,
            conversation_id: None,
            settings_changed: false,
            launch_fingerprint: None,
            session_options: None,
            pane_name: None,
            id: 1,
            project: None,
            title: "Synthetic failure".into(),
            launch: Launch::Shell,
            cwd: std::env::temp_dir(),
            terminal: None,
            loading: None,
            error: String::new(),
        };
        assert!(!session.poll());
        sender
            .send(Err((terminal, "synthetic stop failure".into())))
            .ok()
            .unwrap();
        assert!(session.poll());
        assert!(session.restarting.is_none());
        assert!(session.loading.is_none());
        assert!(session.terminal.is_some());
        assert_eq!(session.error, "synthetic stop failure");
        session.terminal.take();
        let (sender, result) = mpsc::channel();
        session.restarting = Some(Restart {
            result,
            preferences: None,
        });
        drop(sender);
        assert!(session.poll());
        assert!(session.restarting.is_none());
        assert!(session.loading.is_none());
        assert!(session.terminal.is_none());
        assert!(session.error.contains("without confirmation"));
    }
    #[test]
    #[cfg(windows)]
    fn restart_waits_for_old_cli_and_reuses_the_conversation_with_new_settings() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("restart fixture.ps1");
        std::fs::write(&script, r#"$root = $env:SYNTHETIC_ROOT
$lock = [IO.File]::Open((Join-Path $root 'exclusive.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
if (!$lock) { exit 2 }
Add-Content -LiteralPath (Join-Path $root 'invocations.txt') -Value $args[0]
if ($args[0] -eq '--session-id') {
  $project = Join-Path $root 'projects/fixture'
  New-Item -ItemType Directory -Path $project -Force | Out-Null
  Set-Content -LiteralPath (Join-Path $project ($args[1] + '.jsonl')) -Value 'synthetic'
  Set-Content -LiteralPath (Join-Path $root 'ready') -Value 'ready'
  while ($true) { Start-Sleep -Milliseconds 50 }
}
Set-Content -LiteralPath (Join-Path $root 'resumed.json') -Value (ConvertTo-Json -InputObject @($args))
$lock.Dispose()
"#).unwrap();
        let mut prefs = crate::agent_preferences::Preferences {
            command: format!("& '{}'", script.display().to_string().replace('\'', "''")),
            env: format!(
                "CLAUDE_CONFIG_DIR={}\nSYNTHETIC_ROOT={}",
                dir.path().display(),
                dir.path().display()
            ),
            ..Default::default()
        };
        let mut session = Session {
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            codex_seed: None,
            conversation_id: None,
            settings_changed: false,
            launch_fingerprint: None,
            session_options: None,
            pane_name: None,
            id: 1,
            project: None,
            title: "Synthetic".into(),
            launch: Launch::Agent("claude".into()),
            cwd: dir.path().to_owned(),
            terminal: None,
            loading: None,
            error: String::new(),
        };
        session.start(Some(prefs.clone()));
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !dir.path().join("ready").exists() {
            session.poll();
            assert!(session.error.is_empty(), "{}", session.error);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        session.poll();
        let id = session.conversation_id.clone().unwrap();
        prefs.args = "--model synthetic".into();
        assert!(session.can_restart(Some(&prefs)));
        session.restart(Some(prefs.clone()));
        assert!(session.restarting.is_some());
        assert!(session.terminal.is_none());
        assert!(!session.can_restart(Some(&prefs)));
        session.restart(Some(prefs.clone())); // A repeated request cannot create a second worker.
        loop {
            session.poll();
            assert!(session.error.is_empty(), "{}", session.error);
            if session.terminal.as_ref().is_some_and(Terminal::exited) {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(session.conversation_id.as_deref(), Some(id.as_str()));
        let args: Vec<String> =
            serde_json::from_slice(&std::fs::read(dir.path().join("resumed.json")).unwrap())
                .unwrap();
        assert_eq!(args, ["--resume", &id, "--model", "synthetic"]);
        let runs = std::fs::read_to_string(dir.path().join("invocations.txt")).unwrap();
        assert_eq!(
            runs.lines().collect::<Vec<_>>(),
            ["--session-id", "--resume"]
        );
        assert_eq!(
            session.launch_fingerprint,
            crate::launch_fingerprint::for_agent("claude", &prefs)
        );
    }

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
    fn restart_status_requires_a_running_known_launch_and_tracks_effective_changes() {
        let prefs = crate::agent_preferences::Preferences::default();
        let mut session = Session {
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            settings_changed: false,
            codex_seed: None,
            conversation_id: None,
            launch_fingerprint: crate::launch_fingerprint::for_agent("codex", &prefs),
            session_options: None,
            pane_name: None,
            id: 1,
            project: None,
            title: "Synthetic agent metadata".into(),
            launch: Launch::Agent("codex".into()),
            cwd: std::env::temp_dir(),
            terminal: Some(Terminal::spawn().unwrap()),
            loading: None,
            error: String::new(),
        };
        assert!(!session.launch_stale(Some(&prefs)));
        let changed = crate::agent_preferences::Preferences {
            args: "-m different-model".into(),
            ..prefs.clone()
        };
        session.settings_changed = session.launch_stale(Some(&changed));
        assert!(session.needs_restart());
        let (sender, receiver) = mpsc::channel();
        session.loading = Some(receiver);
        assert!(!session.needs_restart());
        assert!(!session.launch_stale(Some(&changed)));
        drop(sender);
        session.loading = None;
        session.launch_fingerprint = None;
        assert!(!session.launch_stale(Some(&changed)));
        session.terminal = None;
        assert!(!session.needs_restart());
    }
    #[test]
    fn native_panes_cannot_start_pty_workers() {
        for launch in [Launch::CodexChat, Launch::ClaudeChat] {
            let mut session = Session {
                restarting: None,
                codex_discovery: None,
                codex_restore: None,
                model_picker: Default::default(),
                last_model: None,
                model_save_attempt: None,
                settings_changed: false,
                codex_seed: None,
                conversation_id: None,
                launch_fingerprint: None,
                session_options: None,
                pane_name: None,
                id: 1,
                project: Some(1),
                title: "Native".into(),
                launch,
                cwd: PathBuf::from("unused"),
                terminal: None,
                loading: None,
                error: String::new(),
            };
            session.start(None);
            assert!(session.loading.is_none());
            assert!(session.terminal.is_none());
            assert!(session.error.is_empty());
        }
    }

    #[test]
    #[cfg(windows)]
    fn codex_terminal_discovers_and_resumes_only_its_verified_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("synthetic codex.ps1");
        let id = "12345678-1234-4234-8234-123456789abc";
        std::fs::write(&script, r#"
Set-Content -LiteralPath (Join-Path $env:SYNTHETIC_ROOT 'args.json') -Value (ConvertTo-Json -InputObject @($args))
if ($args.Count -eq 0 -or $args[0] -ne 'resume') {
    $date = Get-Date
    $folder = Join-Path $env:CODEX_HOME ('sessions/{0:0000}/{1:00}/{2:00}' -f $date.Year, $date.Month, $date.Day)
    New-Item -ItemType Directory -Path $folder -Force | Out-Null
    $meta = @{ type='session_meta'; payload=@{ id='12345678-1234-4234-8234-123456789abc'; cwd=$env:SYNTHETIC_ROOT; timestamp=[DateTime]::UtcNow.ToString('o') } }
    $file = Join-Path $folder 'rollout-synthetic-12345678-1234-4234-8234-123456789abc.jsonl'
    [IO.File]::WriteAllText($file, ($meta | ConvertTo-Json -Depth 3 -Compress))
}
"#).unwrap();
        let mut prefs = crate::agent_preferences::Preferences {
            command: format!("& '{}'", script.display().to_string().replace('\'', "''")),
            env: format!(
                "CODEX_HOME={}\nSYNTHETIC_ROOT={}",
                dir.path().display(),
                dir.path().display()
            ),
            ..Default::default()
        };
        let mut session = Session {
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            restarting: None,
            codex_seed: None,
            conversation_id: None,
            settings_changed: false,
            launch_fingerprint: None,
            session_options: None,
            pane_name: None,
            id: 1,
            project: None,
            title: "Synthetic Codex".into(),
            launch: Launch::Agent("codex".into()),
            cwd: dir.path().into(),
            terminal: None,
            loading: None,
            error: String::new(),
        };
        session.start(Some(prefs.clone()));
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        // Simulate app closure after the CLI wrote history but before discovery.
        let seed = session.codex_seed.clone().unwrap();
        while !session.terminal.as_ref().is_some_and(Terminal::exited) {
            session.poll();
            assert!(session.error.is_empty(), "{}", session.error);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        session.terminal = None;
        session.codex_discovery = None;
        session.codex_seed =
            Some(serde_json::from_str(&serde_json::to_string(&seed).unwrap()).unwrap());
        assert!(session.conversation_id.is_none());
        session.start(Some(prefs.clone()));
        assert!(session.codex_restore.is_some());
        assert!(session.loading.is_none());
        assert!(session.terminal.is_none());
        let mut claimed = std::collections::HashSet::new();
        while session.conversation_id.is_none() {
            session.poll();
            session.poll_conversation(&mut claimed);
            assert!(session.error.is_empty(), "{}", session.error);
            assert!(
                std::time::Instant::now() < deadline,
                "Synthetic conversation not discovered"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(session.conversation_id.as_deref(), Some(id));
        assert!(claimed.contains(id));
        assert!(session.codex_seed.is_none());
        while !session.terminal.as_ref().is_some_and(Terminal::exited) {
            session.poll();
            assert!(session.error.is_empty(), "{}", session.error);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let recovered_args: Vec<String> =
            serde_json::from_slice(&std::fs::read(dir.path().join("args.json")).unwrap()).unwrap();
        assert_eq!(recovered_args, ["resume", id]);
        session.terminal = None;
        prefs.args = "--model synthetic-next".into();
        session.start(Some(prefs.clone()));
        while !session.terminal.as_ref().is_some_and(Terminal::exited) {
            session.poll();
            assert!(session.error.is_empty(), "{}", session.error);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let args: Vec<String> =
            serde_json::from_slice(&std::fs::read(dir.path().join("args.json")).unwrap()).unwrap();
        assert_eq!(args, ["resume", id, "--model", "synthetic-next"]);
        session.terminal = None;
        prefs.env = format!(
            "CODEX_HOME={}\nSYNTHETIC_ROOT={}",
            dir.path().join("another-profile").display(),
            dir.path().display()
        );
        session.start(Some(prefs.clone()));
        while session.loading.is_some() {
            session.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(session.error.contains("not found in this profile"));
        assert!(session.terminal.is_none());
        assert_eq!(session.conversation_id.as_deref(), Some(id));
        session.conversation_id = None;
        session.codex_seed = Some(seed);
        session.start(Some(prefs.clone()));
        assert!(session.error.contains("different profile or folder"));
        assert!(session.loading.is_none());
        assert!(session.codex_restore.is_none());
        session.codex_seed = Some(
            crate::codex_terminal::Discovery::new(&prefs, dir.path())
                .unwrap()
                .seed(),
        );
        session.start(Some(prefs.clone()));
        while session.codex_restore.is_some() {
            session.poll_conversation(&mut claimed);
            assert!(session.loading.is_none());
            assert!(session.terminal.is_none());
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(session.error.contains("no new conversation was started"));
        assert!(session.codex_seed.is_some());
        prefs.args = "resume explicit-user-session".into();
        session.start(Some(prefs));
        assert!(session.conversation_id.is_none());
        assert!(session.codex_discovery.is_none());
        assert!(session.codex_seed.is_none());
        while !session.terminal.as_ref().is_some_and(Terminal::exited) {
            session.poll();
            assert!(session.error.is_empty(), "{}", session.error);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    #[cfg(windows)]
    fn claude_terminal_wake_reuses_its_id_and_resumes_only_an_existing_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("synthetic cli.ps1");
        std::fs::write(&script, r#"Set-Content -LiteralPath (Join-Path $env:SYNTHETIC_ROOT 'args.json') -Value (ConvertTo-Json -InputObject @($args))"#).unwrap();
        let prefs = crate::agent_preferences::Preferences {
            command: format!("& '{}'", script.display().to_string().replace('\'', "''")),
            env: format!(
                "CLAUDE_CONFIG_DIR={}\nSYNTHETIC_ROOT={}",
                dir.path().display(),
                dir.path().display()
            ),
            ..Default::default()
        };
        let mut session = Session {
            codex_seed: None,
            conversation_id: None,
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            settings_changed: false,
            launch_fingerprint: None,
            session_options: None,
            pane_name: None,
            id: 1,
            project: None,
            title: "Synthetic Claude".into(),
            launch: Launch::Agent("claude".into()),
            cwd: dir.path().to_owned(),
            terminal: None,
            loading: None,
            error: String::new(),
        };
        for expected_flag in ["--session-id", "--resume"] {
            session.start(Some(prefs.clone()));
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            loop {
                session.poll();
                assert!(session.error.is_empty(), "{}", session.error);
                if session.terminal.as_ref().is_some_and(Terminal::exited) {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "Synthetic CLI did not finish"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            let args: Vec<String> =
                serde_json::from_slice(&std::fs::read(dir.path().join("args.json")).unwrap())
                    .unwrap();
            let id = session.conversation_id.as_deref().unwrap();
            assert_eq!(args, [expected_flag, id]);
            assert!(crate::claude::valid_uuid(id));
            let project = dir.path().join("projects").join("synthetic");
            std::fs::create_dir_all(&project).unwrap();
            std::fs::write(project.join(format!("{id}.jsonl")), "synthetic transcript").unwrap();
            session.terminal = None;
        }
    }
    #[test]
    fn disconnected_terminal_startup_releases_loading_and_can_receive_a_retry_result() {
        let (sender, receiver) = mpsc::channel();
        let mut session = Session {
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            settings_changed: false,
            codex_seed: None,
            conversation_id: None,
            launch_fingerprint: None,
            session_options: None,
            pane_name: None,
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
    #[test]
    fn names_are_stable_distinct_and_keep_provider_titles_separate() {
        let make = |id, launch| Session {
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            settings_changed: false,
            codex_seed: None,
            conversation_id: None,
            launch_fingerprint: None,
            session_options: None,
            pane_name: None,
            id,
            project: Some(1),
            title: "Provider".into(),
            launch,
            cwd: ".".into(),
            terminal: None,
            loading: None,
            error: String::new(),
        };
        let mut sessions = vec![
            make(1, Launch::CodexChat),
            make(2, Launch::CodexChat),
            make(3, Launch::Shell),
        ];
        normalize_session_names(&mut sessions);
        assert_eq!(sessions[0].label(), "Ada");
        assert_eq!(sessions[1].label(), "Bohr");
        assert_eq!(sessions[2].label(), "Provider");
        assert_eq!(sessions[0].title, "Provider");
        sessions.remove(0);
        sessions.push(make(4, Launch::ClaudeChat));
        normalize_session_names(&mut sessions);
        assert_eq!(sessions[0].label(), "Bohr");
        assert_eq!(sessions[2].label(), "Ada");
    }
}
