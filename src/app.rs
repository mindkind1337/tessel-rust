mod notification_inbox;
use crate::i18n::tr;
use crate::{
    files::Document,
    launch::{self, Launch},
    terminal::Terminal,
    workspace::{self, Host, Store, Workspace},
};
use eframe::egui;
#[path = "app/floating_terminal.rs"]
mod floating_terminal;
#[path = "app/gpui_bridge.rs"]
pub(crate) mod gpui_bridge;
mod linear_task;
#[cfg(windows)]
#[path = "app/native_terminals.rs"]
mod native_terminals;
#[path = "app/new_task.rs"]
mod new_task;
#[path = "app/task_launch.rs"]
mod task_launch;
#[path = "app/task_review.rs"]
mod task_review;
use crate::view_tabs_browser_model::{DiffTabArea, TabsLang, diff_label};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

// Restored sessions may use a non-verbatim Windows path while projects store
// canonical paths. Resolve differing absolute spellings rather than comparing
// case-folded strings (which would be incorrect on case-sensitive filesystems).
fn same_worker_directory(a: &std::path::Path, b: &std::path::Path) -> bool {
    if !a.is_absolute() || !b.is_absolute() {
        return false;
    }
    if a == b {
        return true;
    }
    match (dunce::canonicalize(a), dunce::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

struct Restart {
    preferred_shell: Option<String>,
    switch_mode: bool,
    result: Receiver<Result<(), (Terminal, String)>>,
    preferences: Option<crate::agent_preferences::Preferences>,
}
struct Session {
    ssh_tmux_id: Option<String>,
    ssh_retry: crate::ssh_reconnect::Retry,
    native_host: Option<workspace::SavedHostTerminal>,
    last_model: Option<crate::pane_session_model::SavedPaneModel>,
    model_save_attempt: Option<std::time::Instant>,
    model_picker: crate::session_option_picker_ui::SessionOptionPickerUi,
    codex_seed: Option<crate::codex_terminal::Seed>,
    codex_restore: Option<crate::agent_preferences::Preferences>,
    codex_discovery: Option<crate::codex_terminal::Discovery>,
    restarting: Option<Restart>,
    conversation_id: Option<String>,
    profile_binding: Option<crate::session_profile_identity::Binding>,
    profile_data_dir: Option<PathBuf>,
    profile_root: Option<PathBuf>,
    profile_migration: Option<crate::session_profile_migration::Migration>,
    resume_required: bool,
    settings_changed: bool,
    launch_fingerprint: Option<String>,
    session_options: Option<serde_json::Value>,
    worktree: Option<workspace::SavedWorktree>,
    pane_name: Option<String>,
    broadcast: bool,
    id: u64,
    project: Option<u64>,
    title: String,
    launch: Launch,
    cwd: PathBuf,
    scrollback: usize,
    terminal: Option<Terminal>,
    loading: Option<Receiver<Result<Terminal, String>>>,
    error: String,
}
impl Session {
    fn ssh_terminal_alias(&self) -> Option<&str> {
        match &self.launch {
            Launch::Ssh(host) if self.native_host.is_none() => Some(&host.alias),
            _ => None,
        }
    }

    fn exit_banner(&self, code: Option<i32>, french: bool) -> (String, &'static str, &'static str) {
        let mut labels =
            crate::terminal_exit::ended_session_copy(self.ssh_terminal_alias(), code, french);
        if let Some(note) = self.ssh_retry.notice(
            french,
            self.terminal
                .as_ref()
                .is_some_and(Terminal::ssh_transport_lost),
        ) {
            labels.0.push(' ');
            labels.0.push_str(note);
        }
        labels
    }

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
            && self.profile_migration.is_none()
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
            && self.native_host.is_none()
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
        self.stop_for_relaunch(prefs, None);
    }

    fn retry_terminal(
        &mut self,
        prefs: Option<crate::agent_preferences::Preferences>,
        preferred_shell: Option<String>,
    ) {
        if self.restarting.is_some() || self.loading.is_some() || self.native_host.is_some() {
            return;
        }
        match &self.terminal {
            Some(terminal) if terminal.exited() => {
                self.ssh_retry = Default::default();
                self.stop_for_relaunch(prefs, preferred_shell);
            }
            None if !self.error.is_empty() => {
                self.ssh_retry = Default::default();
                self.start_with_shell(prefs, preferred_shell);
            }
            _ => {}
        }
    }

    fn stop_for_relaunch(
        &mut self,
        prefs: Option<crate::agent_preferences::Preferences>,
        preferred_shell: Option<String>,
    ) {
        let Some(mut terminal) = self.terminal.take() else {
            return;
        };
        let (sender, result) = mpsc::channel();
        self.restarting = Some(Restart {
            preferred_shell,
            switch_mode: false,
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
            || self.profile_migration.is_some()
            || self.terminal.as_ref().is_none_or(Terminal::exited)
        {
            return false;
        }
        let (Launch::Agent(id), Some(before), Some(current)) =
            (&self.launch, &self.launch_fingerprint, current)
        else {
            return false;
        };
        let bound = self
            .profile_root
            .as_deref()
            .and_then(|root| crate::session_profile::with_root(id, current, root).ok());
        crate::launch_fingerprint::for_agent(id, bound.as_ref().unwrap_or(current))
            .is_some_and(|now| now != *before)
    }
    fn label(&self) -> &str {
        self.pane_name.as_deref().unwrap_or(&self.title)
    }
    fn start(&mut self, prefs: Option<crate::agent_preferences::Preferences>) {
        self.start_with_shell(prefs, None);
    }
    fn start_with_shell(
        &mut self,
        prefs: Option<crate::agent_preferences::Preferences>,
        preferred_shell: Option<String>,
    ) {
        self.start_with_trust(prefs, preferred_shell, None);
    }
    fn bind_profile(
        &mut self,
        agent: &str,
        prefs: &crate::agent_preferences::Preferences,
    ) -> Result<crate::agent_preferences::Preferences, String> {
        let home = directories::BaseDirs::new().ok_or("Cannot locate the local home folder.")?;
        let roster = self
            .profile_data_dir
            .as_deref()
            .map(|dir| crate::settings_extra_model::load_managed_accounts(dir, agent))
            .unwrap_or_default();
        let inherited = std::env::var(crate::session_profile::env_key(agent)?).ok();
        let context = crate::session_profile::Context {
            agent,
            prefs,
            cwd: &self.cwd,
            home: home.home_dir(),
            data_dir: self.profile_data_dir.as_deref(),
            roster: &roster,
            inherited: inherited.as_deref(),
        };
        let binding = match &self.profile_binding {
            Some(binding) => binding.clone(),
            None if self.resume_required => return Err("This older pane has no saved profile identity; resume it from Past conversations in its original account.".into()),
            None => context.capture()?,
        };
        let root = context.resolve(&binding)?;
        let prefs = crate::session_profile::with_root(agent, prefs, &root)?;
        self.profile_binding = Some(binding);
        self.profile_root = Some(root);
        Ok(prefs)
    }

    fn start_with_trust(
        &mut self,
        mut prefs: Option<crate::agent_preferences::Preferences>,
        preferred_shell: Option<String>,
        trust_home: Option<PathBuf>,
    ) {
        if self.native_host.is_some()
            || self.codex_restore.is_some()
            || self.restarting.is_some()
            || self.launch.native_chat().is_some()
            || self.terminal.is_some()
            || self.loading.is_some()
            || self.profile_migration.is_some()
        {
            return;
        }
        let launch = self.launch.clone();
        if let (Launch::Agent(agent), Some(current)) = (&launch, &prefs)
            && (crate::claude_terminal::managed(agent, current)
                || crate::codex_terminal::managed(agent, current))
        {
            if self.resume_required && self.profile_binding.is_none() {
                let Some(id) = self.conversation_id.clone() else {
                    self.error = "Saved conversation identity is missing.".into();
                    return;
                };
                let Some(home) = directories::BaseDirs::new() else {
                    self.error = "Cannot locate the local home folder.".into();
                    return;
                };
                match crate::session_profile_migration::Migration::start(
                    agent.clone(),
                    id,
                    self.cwd.clone(),
                    self.profile_data_dir.clone(),
                    home.home_dir().to_path_buf(),
                    current.clone(),
                    preferred_shell,
                    trust_home,
                ) {
                    Ok(migration) => {
                        self.profile_migration = Some(migration);
                        self.error.clear();
                    }
                    Err(error) => self.error = error,
                }
                return;
            }
            match self.bind_profile(agent, current) {
                Ok(bound) => prefs = Some(bound),
                Err(error) => {
                    self.error = error;
                    return;
                }
            }
        }

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
        if let Err(error) = launch::prepare_ssh_tmux_id(&launch, &mut self.ssh_tmux_id) {
            self.error = error;
            return;
        }
        let ssh_tmux_id = self.ssh_tmux_id.clone();
        let automatic = self.ssh_retry.automatic();
        let cwd = self.cwd.clone();
        let scrollback = self.scrollback;
        let (sender, receiver) = mpsc::channel();
        self.loading = Some(receiver);
        self.error.clear();
        let require_existing = self.resume_required;
        let profile_binding = self.profile_binding.clone();
        let profile_data_dir = self.profile_data_dir.clone();
        thread::spawn(move || {
            let effective = match (&launch, &prefs, conversation) {
                (Launch::Agent(agent), Some(prefs), Some(id)) if agent == "codex" => {
                    crate::codex_terminal::prepare(prefs, &cwd, &id).map(Some)
                }
                (Launch::Agent(agent), Some(prefs), Some(id)) => {
                    crate::claude_terminal::prepare_with_existing(
                        agent,
                        prefs,
                        &cwd,
                        &id,
                        require_existing,
                    )
                    .map(Some)
                }
                _ => Ok(prefs),
            };
            let result = effective.and_then(|prefs| {
                if require_existing && !cwd.is_dir() { return Err("The saved conversation folder no longer exists; no replacement session was started.".into()); }
                if let (Some(binding), Launch::Agent(agent)) = (&profile_binding,&launch) {
                    crate::session_profile::check_account_available(binding,agent,profile_data_dir.as_deref())?;
                }
                let channel = crate::ssh_reconnect::Channel::for_launch(&launch, automatic)?;
                let command = launch::pane_command(
                    &launch,
                    &cwd,
                    prefs.as_ref(),
                    preferred_shell.as_deref(),
                    ssh_tmux_id.as_deref(),
                    channel.as_ref(),
                )?;
                if let Some(home) = &trust_home
                    && matches!(&launch, Launch::Agent(id) if id == "claude")
                {
                    let config = crate::remote_agent_tools::claude_config_path(
                        home,
                        command.get_env("CLAUDE_CONFIG_DIR"),
                        command
                            .get_env("CLAUDE_CODE_CUSTOM_OAUTH_URL")
                            .is_some_and(|v| !v.is_empty()),
                    );
                    let _ = crate::remote_agent_tools::apply_local_claude_folder_trust_at(
                        home, &config, &cwd, true, false,
                    );
                }
                let custom = prefs
                    .as_ref()
                    .is_some_and(|p| !crate::agent_env::trim(&p.command).is_empty());
                match &launch {
                    Launch::Agent(agent) if !custom => {
                        Terminal::spawn_agent_command_with_scrollback(agent, command, scrollback)
                    }
                    _ => Terminal::spawn_command_with_scrollback(command, scrollback).map(|mut t| { t.ssh_channel = channel; t }),
                }
                .map_err(|e| e.to_string())
            });
            let _ = sender.send(result);
        });
    }
    fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(result) = self
            .profile_migration
            .as_ref()
            .and_then(|migration| migration.poll())
        {
            let migration = self.profile_migration.take().expect("pending migration");
            changed = true;
            match result {
                Ok(binding) => {
                    self.profile_binding = Some(binding);
                    self.error.clear();
                    self.start_with_trust(
                        Some(migration.prefs.clone()),
                        migration.preferred_shell.clone(),
                        migration.trust_home.clone(),
                    );
                }
                Err(error) => self.error = error,
            }
        }

        let stopped = match self
            .restarting
            .as_ref()
            .filter(|pending| !pending.switch_mode)
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
                    self.start_with_shell(pending.preferences, pending.preferred_shell);
                }
                Ok(Err((mut terminal, error))) => {
                    self.ssh_retry.restore_output(&mut terminal, false);
                    if self.ssh_retry.automatic() {
                        self.ssh_retry.give_up();
                    }
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
                Ok(mut t) => {
                    self.ssh_retry.restore_output(&mut t, true);
                    self.resume_required = self.conversation_id.is_some();
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
            if let Some(channel) = &t.ssh_channel {
                let was_pending = self.ssh_retry.pending();
                if self
                    .ssh_retry
                    .due(std::time::Instant::now(), channel, t.exit_code())
                {
                    self.ssh_retry.retain_output(t);
                    self.stop_for_relaunch(None, None);
                    changed = true;
                } else {
                    changed |= was_pending != self.ssh_retry.pending();
                }
            }
        }
        changed
    }
}
struct PendingDocument {
    position: Option<crate::files::FilePosition>,
    request: std::sync::Arc<()>,
    pinned: bool,
}
enum Message {
    TranscriptTrash(
        crate::side_panels_model::SessionHistoryEntry,
        Result<(), String>,
    ),
    #[cfg(windows)]
    RevealError(String),
    ExternalOpen(PathBuf, Result<Option<PathBuf>, String>),
    DownloadProgress(PathBuf, usize),
    DownloadFinished(PathBuf, Result<crate::local_download::Report, String>),
    FileAction(Result<crate::file_actions::Change, String>),
    Folder(Option<PathBuf>),
    AddProjectPickedFolder(Option<PathBuf>),
    CloneParentPicked(Option<PathBuf>),
    InitParentPicked(Option<PathBuf>),
    Document {
        path: PathBuf,
        request: std::sync::Arc<()>,
        result: Result<Box<Document>, String>,
    },
    DiffDocument {
        source_doc: Option<TimelineOrigin>,
        root: PathBuf,
        path: PathBuf,
        area: DiffTabArea,
        pinned: bool,
        result: Result<String, String>,
    },
}
enum SessionStart<'a> {
    Project,
    Directory(PathBuf),
    History(&'a crate::side_panels_model::SessionHistoryEntry),
}
enum TimelineOrigin {
    Tab(u64),
    Pinned(Option<u64>),
}
#[derive(PartialEq)]
enum View {
    GuestFiles,
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
    General,
    Appearance,
    Terminal,
    Editor,
    Browser,
    Git,
    QuickCommands,
    Hosts,
    Agents,
    Accounts,
    Skills,
    Mcp,
    Connections,
    Context,
    Orchestration,
    Alerts,
    Voice,
    Devtools,
    Automations,
    Stats,
    Updates,
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
struct ActiveAgentInstall {
    session_id: u64,
    agent_name: String,
    pending_command: Option<String>,
    attached_at: Option<std::time::Instant>,
    finished: bool,
}

pub struct Tessel {
    #[cfg(windows)]
    window_frame: Option<crate::window_frame::WindowFrame>,
    pub(crate) state: Workspace,
    store: Option<Store>,
    error: String,
    sessions: Vec<Session>,
    floating: floating_terminal::State,
    next_session: u64,
    active_session: Option<u64>,
    documents: Vec<Document>,
    document_pending: std::collections::HashMap<PathBuf, PendingDocument>,
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
    settings_pages: crate::settings_pages_panel::SettingsPagesState,
    settings_extra: crate::settings_extra_panel::SettingsExtraState,
    claude_managed_accounts: crate::settings_extra_model::ManagedAccountsRoster,
    codex_managed_accounts: crate::settings_extra_model::ManagedAccountsRoster,
    claude_system_email: Option<String>,
    codex_system_email: Option<String>,
    provider_secret_flags: std::collections::BTreeMap<String, bool>,
    cli_sign_ins: crate::settings_extra_model::CliSignInProbe,
    scanned_skills: Vec<crate::settings_extra_model::SkillEntry>,
    installed_mcp_servers: Vec<crate::settings_extra_model::InstalledMcpServer>,
    context_scan: crate::settings_extra_model::ContextScanReport,
    devtools_statuses: Vec<crate::settings_extra_model::DevToolStatus>,
    devtools_rx: Option<Receiver<Vec<crate::settings_extra_model::DevToolStatus>>>,
    mcp_test_rx: Option<
        Receiver<(
            String,
            Result<crate::settings_extra_model::McpProbeOutcome, String>,
        )>,
    >,
    retire_old_google: crate::settings_extra_model::RetireOldGoogleTracker,
    dev_update: crate::dev_update::DevUpdatePoller,
    dev_update_dialog_open: bool,
    dev_update_notice: Option<String>,
    #[allow(dead_code)]
    pending_shell_commands: Vec<(u64, String, Option<std::time::Instant>)>,
    automations_store: crate::automations::AutomationsFile,
    last_automation_minute: Option<i64>,
    appearance_panel: crate::appearance_panel::AppearancePanel,
    command_palette: crate::palette::CommandPaletteState,
    command_palette_focused: bool,
    toolbar_menu_open: bool,
    /// Measured right toolbar group with everything shown and its secondary buttons' width
    /// (`toolbarFit.js`), kept from the frames where they were visible.
    toolbar_right_full: f32,
    toolbar_secondary_w: f32,
    pane_context_menu: Option<(u64, egui::Pos2)>,
    pane_context_menu_just_opened: bool,
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
    native_hosts: crate::native_host_manager::Manager,
    native_host_panel: crate::native_host_panel::Panel,
    #[cfg(windows)]
    native_terminals: native_terminals::State,
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
    timeline: crate::file_timeline::State,
    scm_state: crate::scm_state::ScmState,
    scm_panel: crate::scm_changes_panel::ScmChangesPanel,
    commit_in_flight: bool,
    pending_scm_discard: Option<(
        crate::scm_changes_panel::ScmArea,
        Vec<String>,
        crate::explorer_scm_model::DiscardConfirmationCopy,
    )>,
    pending_agent_install: Option<crate::agent_installs::InstallConfirmSpec>,
    active_agent_installs: Vec<ActiveAgentInstall>,
    rename_project_modal: Option<(u64, String, bool)>,
    pending_close_session: Option<u64>,
    pending_remove_project: Option<u64>,
    pending_quick_command: Option<(u64, String, bool)>,
    owner_task_launches: Vec<task_launch::Pending>,
    owner_task_turns: Vec<task_launch::Running>,
    owner_task_reviews: Vec<task_review::Pending>,
    worktree_state: crate::worktree_state::WorktreeState,
    worktree_cleanup: Option<crate::worktree_cleanup_panel::WorktreeCleanupModal>,
    pending_transcript_trash: Option<crate::side_panels_model::SessionHistoryEntry>,
    transcript_trash_busy: bool,
    pending_delete_worktree: Option<(u64, PathBuf, String)>,
    git_username: Option<String>,
    git_username_rx: Option<Receiver<Option<String>>>,
    right_fullscreen: bool,
    remember_next_project_launch: bool,
    wsl_available: bool,
    chats: Vec<crate::chat::Chat>,
    claude_chats: Vec<crate::claude::Chat>,
    share: crate::share_panel::SharePanel,
    share_tasks: crate::shared_task_apply::SnapshotCache,
    remote: crate::remote_panel::RemotePanel,
    status_bar: crate::status_strip::StatusBarState,
    status_telemetry: crate::status_metrics::StatusTelemetry,
    keep_awake: crate::status_metrics::KeepAwakeController,
    side_panels_ui: crate::side_panels_ui::SidePanelsUiState,
    pub(crate) new_task: crate::new_task_form::Panel,
    pub(crate) linear: crate::linear_model::panel::Panel,
    dialogs_toolbar_ui: crate::dialogs_toolbar_ui::DialogsToolbarUiState,
    teams_hub: crate::teams_relay::TeamsRelayHub,
    team_mcp: crate::team_mcp_runtime::Runtime,
    teams_ui: crate::teams_ui::TeamsUiState,
    notifications: crate::dialogs_toolbar_model::NotificationCenter,
    notified_waiting_panes: std::collections::HashSet<u64>,
    activity_store: crate::side_panels_model::ActivityStore,
    past_sessions_cache: Vec<crate::side_panels_model::SessionHistoryEntry>,
    scm_commits_cache: Vec<crate::side_panels_model::CommitHistoryRow>,
    scm_commits_project: Option<u64>,
    pub(crate) gpui_extra: crate::app::gpui_bridge::GpuiExtraUiState,
    #[cfg(windows)]
    browser_tab_open: bool,
    #[cfg(windows)]
    pub(crate) browser: crate::browser::Browser,
    #[cfg(windows)]
    pub(crate) media_player: crate::files::playback::native::Player,
    #[cfg(windows)]
    html_trust: crate::browser::html_trust::Trust,
    #[cfg(windows)]
    pub(crate) editor_page: crate::browser::editor_page::EditorPage,
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
        #[cfg(windows)]
        crate::file_actions::set_recycle_owner(cc);
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
            Ok((s, w)) => {
                let warning = crate::i18n::translate(w.language, &s.warning).to_owned();
                (Some(s), w, warning)
            }
            Err(e) => (None, Workspace::default(), e),
        };
        #[cfg(windows)]
        crate::terminal::configure_fonts(&cc.egui_ctx);
        crate::i18n::set_language(state.language);
        crate::theme::apply(&cc.egui_ctx, state.dark, state.theme);
        let next_session = state.next_pane_id();
        let remember_next_project_launch = state.launcher_prefs.remember_default.is_some();
        let wsl_available = cfg!(windows)
            && std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .is_some_and(|sys| sys.join("System32").join("wsl.exe").is_file());
        let model_discovery = store
            .as_ref()
            .map(|s| crate::agent_model_discovery::Discovery::open(s.directory()));
        let claude_managed_accounts = store
            .as_ref()
            .map(|s| crate::settings_extra_model::load_managed_accounts(s.directory(), "claude"))
            .unwrap_or_default();
        let codex_managed_accounts = store
            .as_ref()
            .map(|s| crate::settings_extra_model::load_managed_accounts(s.directory(), "codex"))
            .unwrap_or_default();
        let provider_secret_flags = store
            .as_ref()
            .map(|s| crate::settings_extra_model::load_provider_secret_flags(s.directory()))
            .unwrap_or_default();
        let automations_store = store
            .as_ref()
            .map(|s| crate::automations::AutomationsFile::load(s.directory()))
            .unwrap_or_default();
        let notifications = notification_inbox::load(store.as_ref());
        let activity_store = store
            .as_ref()
            .map(|s| crate::side_panels_model::ActivityStore::load(s.directory()))
            .unwrap_or_default();
        let remote = crate::remote_panel::RemotePanel::with_profile(
            store.as_ref().map(|s| s.directory().to_owned()),
        );
        #[cfg(windows)]
        let browser = crate::browser::Browser::default();
        #[cfg(windows)]
        let editor_page = crate::browser::editor_page::EditorPage::new(&browser);
        let mut app = Self {
            #[cfg(windows)]
            window_frame,
            floating: floating_terminal::State::new(&cc.egui_ctx, state.floating),
            state,
            #[cfg(windows)]
            html_trust: crate::browser::html_trust::Trust::load(
                store.as_ref().map(|s| s.directory()),
            ),
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
            settings_section: SettingsSection::General,
            settings_nav: Default::default(),
            settings_pages: crate::settings_pages_panel::SettingsPagesState::new(),
            settings_extra: crate::settings_extra_panel::SettingsExtraState::new(),
            claude_managed_accounts,
            codex_managed_accounts,
            claude_system_email: directories::BaseDirs::new().and_then(|b| {
                crate::settings_extra_model::probe_system_provider_email(b.home_dir(), "claude")
            }),
            codex_system_email: directories::BaseDirs::new().and_then(|b| {
                crate::settings_extra_model::probe_system_provider_email(b.home_dir(), "codex")
            }),
            provider_secret_flags,
            cli_sign_ins: directories::BaseDirs::new()
                .map(|b| crate::settings_extra_model::probe_cli_sign_ins(b.home_dir()))
                .unwrap_or_default(),
            scanned_skills: Vec::new(),
            installed_mcp_servers: Vec::new(),
            context_scan: Default::default(),
            devtools_statuses: Vec::new(),
            devtools_rx: None,
            mcp_test_rx: None,
            retire_old_google: Default::default(),
            dev_update: crate::dev_update::DevUpdatePoller::new(true),
            dev_update_dialog_open: false,
            dev_update_notice: None,
            pending_shell_commands: Vec::new(),
            automations_store,
            last_automation_minute: None,
            appearance_panel: Default::default(),
            command_palette: Default::default(),
            command_palette_focused: false,
            toolbar_menu_open: false,
            toolbar_right_full: 380.0,
            toolbar_secondary_w: 160.0,
            pane_context_menu: None,
            pane_context_menu_just_opened: false,
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
            native_hosts: Default::default(),
            native_host_panel: Default::default(),
            #[cfg(windows)]
            native_terminals: Default::default(),
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
            timeline: Default::default(),
            scm_state: Default::default(),
            scm_panel: Default::default(),
            commit_in_flight: false,
            pending_scm_discard: None,
            pending_agent_install: None,
            pending_transcript_trash: None,
            transcript_trash_busy: false,
            active_agent_installs: Vec::new(),
            rename_project_modal: None,
            pending_close_session: None,
            pending_remove_project: None,
            pending_quick_command: None,
            owner_task_launches: Vec::new(),
            owner_task_turns: Vec::new(),
            owner_task_reviews: Vec::new(),
            worktree_state: Default::default(),
            worktree_cleanup: None,
            pending_delete_worktree: None,
            git_username: None,
            git_username_rx: {
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || {
                    let mut cmd = std::process::Command::new("git");
                    crate::agent_env::clean_command(&mut cmd);
                    cmd.args(["config", "--global", "user.name"]);
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        cmd.creation_flags(0x0800_0000);
                    }
                    let name = cmd
                        .output()
                        .ok()
                        .filter(|o| o.status.success())
                        .and_then(|o| String::from_utf8(o.stdout).ok())
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty());
                    let _ = tx.send(name);
                });
                Some(rx)
            },
            right_fullscreen: false,
            remember_next_project_launch,
            wsl_available,
            chats: Vec::new(),
            claude_chats: Vec::new(),
            share: crate::share_panel::SharePanel::default(),
            share_tasks: crate::shared_task_apply::SnapshotCache::default(),
            remote,
            status_bar: Default::default(),
            status_telemetry: crate::status_metrics::StatusTelemetry::spawn({
                let ctx = cc.egui_ctx.clone();
                move || ctx.request_repaint()
            }),
            keep_awake: Default::default(),
            side_panels_ui: crate::side_panels_ui::SidePanelsUiState::new(),
            new_task: Default::default(),
            linear: Default::default(),
            dialogs_toolbar_ui: crate::dialogs_toolbar_ui::DialogsToolbarUiState::new(),
            teams_hub: Default::default(),
            team_mcp: Default::default(),
            teams_ui: Default::default(),
            notifications,
            notified_waiting_panes: Default::default(),
            activity_store,
            past_sessions_cache: Vec::new(),
            scm_commits_cache: Vec::new(),
            scm_commits_project: None,
            gpui_extra: Default::default(),
            #[cfg(windows)]
            browser_tab_open: false,
            #[cfg(windows)]
            browser,
            #[cfg(windows)]
            media_player: Default::default(),
            #[cfg(windows)]
            editor_page,
            #[cfg(windows)]
            agent_browsers: Default::default(),
            #[cfg(windows)]
            show_agent_browser: false,
            #[cfg(windows)]
            vault: crate::vault_panel::VaultPanel::with_parent(cc),
            #[cfg(windows)]
            account: Default::default(),
        };
        app.explorer_tree
            .set_show_dotfiles(app.state.editor_prefs.show_git_ignored);
        app.explorer_tree
            .set_compact_folders(app.state.editor_prefs.compact_folders);
        app.scm_panel
            .set_group_order(app.state.git_prefs.scm_group_order.into_panel());
        app.scm_panel
            .set_view_mode(app.state.git_prefs.scm_view_mode.into_panel());
        if app.state.general_prefs.restore_workspaces {
            let initial_scrollback = app.state.terminal_prefs.scrollback;
            for saved in &app.state.sessions {
                app.next_session = app.next_session.max(saved.id + 1);
                app.sessions.push(Session {
                    profile_data_dir: app.store.as_ref().map(|store| store.directory().to_owned()),
                    profile_root: None,
                    profile_migration: None,
                    resume_required: saved.conversation_id.is_some(),
                    ssh_retry: Default::default(),
                    ssh_tmux_id: saved.ssh_tmux_id.clone(),
                    native_host: saved.native_host.clone(),
                    restarting: None,
                    codex_discovery: None,
                    codex_restore: None,
                    model_picker: Default::default(),
                    last_model: saved.last_model.clone(),
                    model_save_attempt: None,
                    settings_changed: false,
                    codex_seed: saved.codex_seed.clone(),
                    conversation_id: saved.conversation_id.clone(),
                    profile_binding: saved.profile_binding.clone(),
                    launch_fingerprint: saved.launch_fingerprint.clone(),
                    session_options: if saved.launch.native_chat().is_some() {
                        saved.native_options.clone()
                    } else {
                        saved.session_options.clone()
                    },
                    worktree: saved.worktree.clone(),
                    pane_name: saved.pane_name.clone(),
                    broadcast: saved.broadcast,
                    id: saved.id,
                    project: saved.project,
                    title: saved.title.clone(),
                    launch: saved.launch.clone(),
                    cwd: saved.cwd.clone(),
                    scrollback: initial_scrollback,
                    terminal: None,
                    loading: None,
                    error: String::new(),
                });
            }
        }
        app.active_session = app.active_project_work_pane();
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
                app.settings_section = SettingsSection::General;
                app.restore_preview = Some(Workspace::default());
            }
            if std::env::args().any(|a| a == "--settings-fixture") {
                app.overlay = Some(Overlay::Settings);
            }
            if std::env::args().any(|a| a == "--native-host-fixture") {
                app.overlay = Some(Overlay::Settings);
                app.settings_section = SettingsSection::Hosts;
                app.state.hosts = vec![Host {
                    alias: "fixture-host".into(),
                    folder: "/demo".into(),
                    ..Default::default()
                }];
                app.native_host_panel = crate::native_host_panel::Panel::fixture();
            }
            #[cfg(windows)]
            if std::env::args().any(|a| a == "--vault-panel-fixture") {
                app.overlay = Some(Overlay::Settings);
                app.settings_section = SettingsSection::Passwords;
                app.vault.load_fixture();
            }
            if std::env::args().any(|a| a == "--new-task-fixture") {
                app.state.language = crate::i18n::Language::French;
                app.agents = crate::agent_detection::Detection::model_picker_fixture("codex");
                app.new_task.begin(
                    7,
                    PathBuf::from("C:/fixtures/project"),
                    "Projet de démonstration".into(),
                    vec![
                        crate::new_task_form::Choice {
                            id: "codex".into(),
                            label: "Codex".into(),
                            enabled: true,
                        },
                        crate::new_task_form::Choice {
                            id: "claude".into(),
                            label: "Claude".into(),
                            enabled: false,
                        },
                    ],
                    vec![crate::native_review::Choice {
                        label: "Réviseur de démonstration".into(),
                        target: crate::native_review::Target {
                            pane: 99,
                            project: 7,
                            cwd: PathBuf::from("C:/fixtures/project"),
                            kind: crate::native_review::Kind::Codex,
                            epoch: 1,
                        },
                    }],
                );
                app.new_task.existing = app.new_task.reviewers.clone();
                app.new_task.choices.push(crate::new_task_form::Choice {
                    id: "pane:99".into(),
                    label: "Agent de démonstration".into(),
                    enabled: true,
                });
                app.new_task.isolation = Some(true);
                app.new_task.edit(
                    crate::new_task_form::Field::Title,
                    "Vérifier la page de démonstration".into(),
                );
                app.new_task.edit(crate::new_task_form::Field::Brief, "Utiliser uniquement les fichiers fictifs. Vérifier le résultat avant de terminer.".into());
            }
            if std::env::args().any(|a| a == "--linear-task-fixture") {
                app.state.language = crate::i18n::Language::French;
                crate::i18n::set_language(app.state.language);
                app.agents = crate::agent_detection::Detection::model_picker_fixture("codex");
                app.install_linear_view_fixture();
                // Rendering only: no bound project, worker, account secret or launch permit.
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
        let native_host_fixture =
            cfg!(feature = "visual-test") && std::env::args().any(|a| a == "--native-host-fixture");
        let vault_fixture =
            cfg!(feature = "visual-test") && std::env::args().any(|a| a == "--vault-panel-fixture");
        let new_task_fixture =
            cfg!(feature = "visual-test") && std::env::args().any(|a| a == "--new-task-fixture");
        let linear_task_fixture =
            cfg!(feature = "visual-test") && std::env::args().any(|a| a == "--linear-task-fixture");
        if !models_fixture
            && !native_host_fixture
            && !vault_fixture
            && !new_task_fixture
            && !linear_task_fixture
        {
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
        app.worktree_state.refresh_all(&app.state.projects);
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
                        profile_data_dir: app
                            .store
                            .as_ref()
                            .map(|store| store.directory().to_owned()),
                        profile_root: None,
                        profile_migration: None,
                        resume_required: false,
                        ssh_retry: Default::default(),
                        ssh_tmux_id: None,
                        native_host: None,
                        restarting: None,
                        codex_discovery: None,
                        codex_restore: None,
                        model_picker: Default::default(),
                        last_model: None,
                        model_save_attempt: None,
                        settings_changed: false,
                        codex_seed: None,
                        conversation_id: None,
                        profile_binding: None,
                        launch_fingerprint: None,
                        session_options: None,
                        worktree: None,
                        pane_name: None,
                        broadcast: true,
                        id,
                        project: app.state.active_project,
                        title: "Terminal rendering fixture".into(),
                        launch: Launch::Shell,
                        cwd: launch::default_directory(),
                        scrollback: app.state.terminal_prefs.scrollback,
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
                    profile_data_dir: app.store.as_ref().map(|store| store.directory().to_owned()),
                    profile_root: None,
                    profile_migration: None,
                    resume_required: false,
                    ssh_retry: Default::default(),
                    ssh_tmux_id: None,
                    native_host: None,
                    restarting: None,
                    codex_discovery: None,
                    codex_restore: None,
                    model_picker: Default::default(),
                    last_model: None,
                    model_save_attempt: None,
                    settings_changed: false,
                    codex_seed: None,
                    conversation_id: None,
                    profile_binding: None,
                    launch_fingerprint: None,
                    session_options: None,
                    worktree: None,
                    pane_name: None,
                    broadcast: true,
                    id,
                    project: app.state.active_project,
                    title: format!("Terminal {id}"),
                    launch: Launch::Shell,
                    cwd: launch::default_directory(),
                    scrollback: app.state.terminal_prefs.scrollback,
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
        #[cfg(feature = "visual-test")]
        if std::env::args().any(|arg| arg == "--floating-fixture") {
            app.floating_fixture();
        }
        app
    }
    fn capture_runtime_state(&mut self) {
        self.refresh_launch_status();
        normalize_session_names(&mut self.sessions);
        self.state.next_pane = self.next_session;
        let previous_numbers: std::collections::HashMap<_, _> = self
            .state
            .sessions
            .iter()
            .filter_map(|session| session.legacy_number.map(|number| (session.id, number)))
            .collect();
        self.state.sessions = self
            .sessions
            .iter()
            .map(|s| workspace::SavedSession {
                legacy_number: previous_numbers.get(&s.id).copied(),
                ssh_tmux_id: s.ssh_tmux_id.clone(),
                native_host: s.native_host.clone(),
                last_model: s.last_model.clone(),
                codex_seed: s.codex_seed.clone(),
                conversation_id: s.conversation_id.clone(),
                profile_binding: s.profile_binding.clone(),
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
                worktree: s.worktree.clone(),
                pane_name: s.pane_name.clone(),
                broadcast: s.broadcast,
                id: s.id,
                project: s.project,
                title: s.title.clone(),
                launch: s.launch.clone(),
                cwd: s.cwd.clone(),
            })
            .collect();
        self.state.sync_terminal_layouts(self.grid_room);
        self.remember_active_work_pane();
        self.state.normalize_pane_numbers();
    }
    fn auto_save_documents(&mut self, now: std::time::Instant) -> bool {
        let mut changed = false;
        if self.state.editor_prefs.auto_save
            && !self.file_action_pending
            && !self.transcript_trash_busy
        {
            let delay = Duration::from_millis(u64::from(
                self.state
                    .editor_prefs
                    .auto_save_delay_ms
                    .clamp(250, 10_000),
            ));

            for doc in &mut self.documents {
                if !doc.media
                    && doc.diff_title.is_none()
                    && !doc.deleted_on_disk
                    && doc.dirty()
                    && let Some(edited) = doc.last_edited_at
                    && now.saturating_duration_since(edited) >= delay
                {
                    changed = true;
                    match doc.save(self.store.as_ref().map(|s| s.directory())) {
                        Ok(()) => doc.error.clear(),
                        Err(err) => {
                            doc.last_edited_at = None;
                            doc.error = err;
                        }
                    }
                }
            }
        }
        changed
    }

    fn save(&mut self) {
        self.capture_runtime_state();
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
        let custom = self.state.custom_agents.clone();
        self.agents.start(move || {
            let search = crate::agent_paths::SearchPath::for_detection()?;
            launch::discover(&search, &custom)
        });
    }
    fn refresh_custom_agents(&mut self, changed: Option<&str>) {
        self.agents.invalidate_custom(changed);
        if !self.agents.detecting() {
            self.detect_agents();
        }
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
        self.gpui_extra.layout_menu_right = None;
        let visible = crate::terminal_layout::project_key(self.state.active_project);
        for (key, layout) in &mut self.state.terminal_layouts {
            if key != &visible {
                layout.maximized = None;
            }
        }
        self.remember_active_work_pane();
        self.active_session = self.active_project_work_pane();
        if self.tasks_board_project != self.state.active_project {
            self.tasks_board.reset();
            self.tasks_board_project = self.state.active_project;
        }
        self.finder.close();
        self.sync_git_project();
        self.explorer_tree
            .set_root(self.state.active().map(|p| p.path.as_path()));
        self.worktree_state.retain_projects(&self.state.projects);
        self.worktree_state.refresh_missing(&self.state.projects);
        if let Some(proj) = self.state.active() {
            self.worktree_state
                .refresh_project(proj.id, proj.path.clone());
        }
    }

    fn worktree_open_pane_counts(
        &self,
        project_id: u64,
    ) -> std::collections::HashMap<String, usize> {
        let mut counts = std::collections::HashMap::new();
        for s in &self.sessions {
            if s.project != Some(project_id) {
                continue;
            }
            let key = if let Some(wt) = &s.worktree {
                crate::sidebar_model::norm_sidebar_path(&wt.path.to_string_lossy())
            } else {
                crate::sidebar_model::norm_sidebar_path(&s.cwd.to_string_lossy())
            };
            if !key.is_empty() {
                *counts.entry(key).or_insert(0) += 1;
            }
        }
        counts
    }

    fn open_worktree_cleanup(&mut self, project_id: u64) {
        let Some(proj) = self.state.projects.iter().find(|p| p.id == project_id) else {
            return;
        };
        let counts = self.worktree_open_pane_counts(project_id);
        self.worktree_cleanup = Some(
            crate::worktree_cleanup_panel::WorktreeCleanupModal::new_scanning(
                project_id,
                proj.name.clone(),
            ),
        );
        self.worktree_state
            .request_cleanup_scan(project_id, proj.path.clone(), counts);
    }

    fn start_new_worktree_pane(
        &mut self,
        project_id: u64,
        label: &str,
        target: crate::worktree_state::WorktreeLaunchTarget,
    ) {
        let Some(proj) = self.state.projects.iter().find(|p| p.id == project_id) else {
            return;
        };
        self.worktree_state
            .request_create(crate::worktree_state::CreateRequest {
                project_id,
                cwd: proj.path.clone(),
                label: label.to_owned(),
                base_branch: None,
                copy_env: true,
                run_setup: false,
                general_prefs: self.state.general_prefs.clone(),
                git_prefs: self.state.git_prefs.clone(),
                target,
            });
    }

    fn close_sessions_for_worktree_paths(&mut self, project_id: u64, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        let matching_ids: Vec<u64> = self
            .sessions
            .iter()
            .filter(|s| {
                s.project == Some(project_id)
                    && paths.iter().any(|p| {
                        let p_str = p.to_string_lossy();
                        crate::sidebar_model::same_path(&s.cwd.to_string_lossy(), &p_str)
                            || s.worktree.as_ref().is_some_and(|wt| {
                                crate::sidebar_model::same_path(&wt.path.to_string_lossy(), &p_str)
                            })
                    })
            })
            .map(|s| s.id)
            .collect();
        for sid in matching_ids {
            self.close_session_immediate(sid);
        }
    }

    fn accept_prepared_worktree(
        &mut self,
        project_id: u64,
        source: &std::path::Path,
        prepared: &std::path::Path,
    ) -> bool {
        if self
            .state
            .projects
            .iter()
            .any(|p| p.id == project_id && p.path == source)
        {
            return true;
        }
        self.error = format!(
            "The project folder changed. The prepared copy was kept at {}. No agent was started.",
            prepared.display()
        );
        false
    }

    fn poll_worktrees(&mut self) {
        use crate::finder_model::{LaunchMenuItem, LaunchPlacement};
        use crate::worktree_state::{WorktreeEvent, WorktreeLaunchTarget};

        for ev in self.worktree_state.poll(&self.state.projects) {
            match ev {
                WorktreeEvent::Inspected {
                    project_id,
                    source_cwd,
                    summary,
                } => {
                    if self.new_task.open && self.new_task.project == project_id {
                        if self.new_task.source == source_cwd {
                            self.new_task.isolation = Some(summary.is_some_and(|s| s.has_commits));
                        } else if self.new_task.isolation.is_none() {
                            self.worktree_state
                                .refresh_project(project_id, self.new_task.source.clone());
                        }
                    }
                }
                WorktreeEvent::Created {
                    target: WorktreeLaunchTarget::Task(id),
                    result,
                    ..
                } => {
                    self.finish_owner_task_copy(id, result.map(|(copy, _)| *copy));
                }
                WorktreeEvent::Created {
                    project_id,
                    source_cwd,
                    target,
                    result,
                } => match result {
                    Err(err) => self.error = err,
                    Ok((created, _)) => {
                        if !self.accept_prepared_worktree(project_id, &source_cwd, &created.path) {
                            continue;
                        }
                        if self.state.active_project != Some(project_id) {
                            self.state.active_project = Some(project_id);
                            self.refresh_project();
                        }
                        let saved_wt = workspace::SavedWorktree {
                            path: created.path,
                            branch: created.branch,
                            base_branch: Some(created.base_branch),
                        };
                        match target {
                            WorktreeLaunchTarget::Task(_) => unreachable!("handled above"),
                            WorktreeLaunchTarget::ShellNewPane => {
                                self.new_session_in_worktree(
                                    Some(project_id),
                                    Launch::Shell,
                                    None,
                                    false,
                                    Some(saved_wt),
                                );
                            }
                            WorktreeLaunchTarget::Menu { item, placement } => {
                                let launch = match item {
                                    LaunchMenuItem::Shell { .. } => Launch::Shell,
                                    LaunchMenuItem::Agent { id, .. } => Launch::Agent(id),
                                    LaunchMenuItem::ChatAgent { id, .. } if id == "codex" => {
                                        Launch::CodexChat
                                    }
                                    LaunchMenuItem::ChatAgent { id, .. } if id == "claude" => {
                                        Launch::ClaudeChat
                                    }
                                    _ => Launch::Shell,
                                };
                                let active_target = self
                                    .sessions
                                    .iter()
                                    .find(|s| {
                                        Some(s.id) == self.active_session
                                            && s.project == Some(project_id)
                                    })
                                    .map(|s| s.id);
                                let split = active_target.and_then(|id| match placement {
                                    LaunchPlacement::Auto => None,
                                    LaunchPlacement::Left | LaunchPlacement::Right => {
                                        Some((id, crate::pane_grid::SplitDir::Row))
                                    }
                                    LaunchPlacement::Down => {
                                        Some((id, crate::pane_grid::SplitDir::Col))
                                    }
                                });
                                self.new_session_in_worktree(
                                    Some(project_id),
                                    launch,
                                    split,
                                    placement == LaunchPlacement::Left,
                                    Some(saved_wt),
                                );
                            }
                        }
                    }
                },
                WorktreeEvent::CleanupScanned { project_id, result } => {
                    if let Some(modal) = &mut self.worktree_cleanup
                        && modal.project_id == project_id
                    {
                        match result {
                            Ok((default_branch, items)) => {
                                modal.set_scan_results(default_branch, items);
                            }
                            Err(err) => modal.set_error(err),
                        }
                    }
                }
                WorktreeEvent::Removed {
                    project_id,
                    removed,
                    errors,
                    ..
                } => {
                    if !removed.is_empty() {
                        self.close_sessions_for_worktree_paths(project_id, &removed);
                        if let Some(prefs) = self.state.project_worktrees.get_mut(&project_id) {
                            prefs.shown_worktrees.retain(|shown| {
                                !removed.iter().any(|r| {
                                    crate::sidebar_model::same_path(shown, &r.to_string_lossy())
                                })
                            });
                        }
                        self.save();
                    }
                    if errors.is_empty() {
                        if self
                            .worktree_cleanup
                            .as_ref()
                            .is_some_and(|m| m.project_id == project_id)
                        {
                            self.worktree_cleanup = None;
                        }
                    } else {
                        let joined = errors.join("\n");
                        if let Some(modal) = &mut self.worktree_cleanup
                            && modal.project_id == project_id
                        {
                            modal.set_error(joined);
                        } else {
                            self.error = joined;
                        }
                    }
                }
            }
        }
    }
    fn open_document(&mut self, path: PathBuf) {
        self.open_document_mode(path, true);
    }
    fn open_document_preview(&mut self, path: PathBuf) {
        let pinned = !self.state.editor_prefs.preview_tabs;
        self.open_document_mode(path, pinned);
    }
    fn open_document_mode(&mut self, path: PathBuf, pinned: bool) {
        let Some(root) = self.state.active().map(|p| p.path.clone()) else {
            return;
        };
        self.open_document_at_mode(root, path, pinned);
    }
    fn open_explorer_content_hit(&mut self, path: PathBuf, line: usize) {
        if let Some(root) = self.state.active().map(|p| p.path.clone()) {
            self.explorer_tree.select_content_hit(&path, line);
            self.open_document_at_position(
                root,
                path,
                !self.state.editor_prefs.preview_tabs,
                Some(crate::files::FilePosition { line, column: 1 }),
            );
        }
    }
    fn open_chat_citation(&mut self, pane: u64, link: &crate::chat::citations::FileLink) {
        let path = &link.path;
        let Some(project) = self.state.active() else {
            return;
        };
        let Some(session) = self.sessions.iter().find(|s| {
            s.id == pane && s.project == Some(project.id) && s.launch.native_chat().is_some()
        }) else {
            return;
        };
        let root = session
            .worktree
            .as_ref()
            .filter(|wt| {
                session.cwd.starts_with(&wt.path) && session.cwd.join(path).starts_with(&wt.path)
            })
            .map_or_else(|| project.path.clone(), |wt| wt.path.clone());
        match crate::chat::citations::destination(&root, &session.cwd, path) {
            Ok(file) => self.open_document_at_position(root, file, true, link.position),
            Err(error) => self.error = error,
        }
    }
    fn open_linked_document(&mut self, root: PathBuf, path: PathBuf) {
        if let Some(source) = self.documents.get_mut(self.active_document) {
            source.unpinned_preview = false;
        }
        self.open_document_at(root, path);
    }
    fn open_document_at(&mut self, root: PathBuf, path: PathBuf) {
        self.open_document_at_mode(root, path, true);
    }
    fn open_document_at_mode(&mut self, root: PathBuf, path: PathBuf, pinned: bool) {
        self.open_document_at_position(root, path, pinned, None);
    }
    fn open_document_at_position(
        &mut self,
        root: PathBuf,
        path: PathBuf,
        pinned: bool,
        position: Option<crate::files::FilePosition>,
    ) {
        let key = dunce::simplified(&path);
        if let Some(i) = self
            .documents
            .iter()
            .position(|d| d.diff_title.is_none() && dunce::simplified(&d.path) == key)
        {
            if pinned {
                self.documents[i].unpinned_preview = false;
            }
            self.documents[i].reveal_location(position);
            self.active_document = i;
            self.view = View::Files;
            return;
        }
        if let Some(pending) = self.document_pending.get_mut(key) {
            pending.pinned |= pinned;
            if position.is_some() {
                pending.position = position;
            }
            return;
        }
        let replaceable = !pinned
            && self
                .documents
                .iter()
                .any(|d| d.unpinned_preview && !d.dirty());
        if !replaceable && self.documents.len() + self.document_pending.len() >= 24 {
            self.error = "Close a document before opening more (24-tab limit)".into();
            return;
        }
        let request = std::sync::Arc::new(());
        self.document_pending.insert(
            key.to_owned(),
            PendingDocument {
                position,
                request: request.clone(),
                pinned,
            },
        );
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = Document::open(&root, &path).map(Box::new);
            let _ = sender.send(Message::Document {
                path,
                request,
                result,
            });
        });
    }
    fn timeline_snapshot(&mut self, visible: bool) -> crate::file_timeline::Snapshot {
        let shown = visible && self.state.editor_prefs.timeline_shown;
        if self
            .timeline
            .pin
            .as_ref()
            .is_some_and(|p| p.project != self.state.active_project)
        {
            self.timeline.pin = None;
        }
        let active = (shown && self.view == View::Files)
            .then(|| self.documents.get(self.active_document))
            .flatten()
            .filter(|d| d.root.is_absolute() && d.path.is_absolute())
            .map(|d| d.path.clone());
        let pin = self
            .timeline
            .pin
            .as_ref()
            .filter(|_| shown && self.view == View::Files);
        let pinned = pin.is_some();
        let file = pin.map(|p| p.path.clone()).or(active);
        self.timeline.profile = self.store.as_ref().map(|s| s.directory().to_owned());
        let sources = crate::file_timeline::Sources {
            git: self.state.editor_prefs.timeline_git,
            local: self.state.editor_prefs.timeline_local,
        };
        if !shown {
            self.timeline.filters_open = false;
        }
        self.timeline.select(file.clone(), sources);
        crate::file_timeline::Snapshot {
            sources,
            filters_open: self.timeline.filters_open,
            shown,
            pinned,
            file,
            data: self.timeline.data.clone(),
        }
    }
    fn timeline_action(&mut self, action: crate::file_timeline::Action) {
        use crate::file_timeline::Action;
        match action {
            Action::Toggle => {
                self.state.editor_prefs.timeline_shown = !self.state.editor_prefs.timeline_shown;
                self.save();
            }
            Action::OpenMail(target) => {
                if let Err(error) = open::that_detached(target) {
                    self.error = error.to_string();
                }
            }
            Action::Filters(open) => self.timeline.filters_open = open,
            Action::ToggleSource(source) => {
                let prefs = &mut self.state.editor_prefs;
                let value = match source {
                    crate::file_timeline::Source::Git => &mut prefs.timeline_git,
                    crate::file_timeline::Source::Local => &mut prefs.timeline_local,
                };
                *value = !*value;
                self.timeline.filters_open = false;
                self.save();
            }
            Action::TogglePin(path) if self.timeline.file.as_ref() == Some(&path) => {
                if self.timeline.pin.is_some() {
                    self.timeline.pin = None;
                } else if let Some(doc) = self
                    .documents
                    .get(self.active_document)
                    .filter(|d| d.path == path && self.view == View::Files)
                {
                    self.timeline.pin = Some(crate::file_timeline::PinnedFile {
                        project: self.state.active_project,
                        root: doc.root.clone(),
                        path,
                    });
                }
            }
            Action::Refresh(path, more) if self.timeline.file.as_ref() == Some(&path) => {
                self.timeline.refresh(more)
            }
            Action::Open(path, commit, pinned) if self.timeline.file.as_ref() == Some(&path) => {
                if self.view != View::Files {
                    return;
                }
                let (origin, root) = if let Some(pin) = self
                    .timeline
                    .pin
                    .as_ref()
                    .filter(|p| p.path == path && p.project == self.state.active_project)
                {
                    (TimelineOrigin::Pinned(pin.project), pin.root.clone())
                } else if let Some(doc) = self
                    .documents
                    .get(self.active_document)
                    .filter(|d| d.path == path)
                {
                    (TimelineOrigin::Tab(doc.instance_id()), doc.root.clone())
                } else {
                    return;
                };
                let Some(Ok(history)) = self.timeline.data.clone() else {
                    return;
                };
                let source_doc = Some(origin);
                let saved = commit
                    .as_deref()
                    .and_then(|id| id.strip_prefix("local:"))
                    .and_then(|id| history.saves.iter().find(|e| e.id == id));
                let area = if let Some(saved) = saved {
                    let date = chrono::DateTime::from_timestamp_millis(saved.timestamp)
                        .map(|d| {
                            d.with_timezone(&chrono::Local)
                                .format("%Y-%m-%d %H:%M:%S%.3f")
                                .to_string()
                        })
                        .unwrap_or_default();
                    DiffTabArea::Local {
                        id: saved.id.clone(),
                        formatted_date: date,
                    }
                } else if let Some(commit) = &commit {
                    DiffTabArea::Commit {
                        commit: commit.clone(),
                    }
                } else {
                    DiffTabArea::Unstaged
                };
                let profile = self.timeline.profile.clone();
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let result = if let Some(id) =
                        commit.as_deref().and_then(|s| s.strip_prefix("local:"))
                    {
                        profile
                            .as_deref()
                            .ok_or_else(|| "Local history unavailable".to_owned())
                            .and_then(|dir| crate::local_file_history::diff(dir, &root, &path, id))
                    } else {
                        history.diff(commit.as_deref())
                    };
                    let _ = sender.send(Message::DiffDocument {
                        source_doc,
                        root,
                        path,
                        area,
                        pinned,
                        result,
                    });
                });
            }
            _ => {}
        }
    }
    fn open_scm_diff(
        &mut self,
        area: crate::scm_changes_panel::ScmArea,
        relative: String,
        old_relative: Option<String>,
        pinned: bool,
    ) {
        use crate::scm_changes_panel::ScmArea;
        let Some(root) = self.state.active().map(|p| p.path.clone()) else {
            return;
        };
        let (entry_path, entry_old_path, staged, untracked) =
            match self.scm_state.validate_diff_entry(area, &relative) {
                Ok(entry) => entry,
                Err(error) => {
                    self.error = error;
                    return;
                }
            };
        let full_path = root.join(&entry_path);
        let area = match area {
            ScmArea::Staged => DiffTabArea::Staged,
            ScmArea::Unstaged | ScmArea::Untracked => DiffTabArea::Unstaged,
        };
        let existing = self
            .documents
            .iter()
            .any(|d| d.path == full_path && d.diff_area.as_ref() == Some(&area));
        let replaceable = !pinned
            && self
                .documents
                .iter()
                .any(|d| d.unpinned_preview && !d.dirty());
        if !existing && !replaceable && self.documents.len() + self.document_pending.len() >= 24 {
            self.error = "Close a document before opening more (24-tab limit)".into();
            return;
        }
        let old_rel = old_relative.or(entry_old_path);
        let rel = entry_path;
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = crate::git::file_diff(&root, &rel, old_rel.as_deref(), staged, untracked);
            let _ = sender.send(Message::DiffDocument {
                source_doc: None,
                root,
                path: full_path,
                area,
                pinned,
                result,
            });
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
    fn poll(&mut self) -> bool {
        let mut native_changed = self.share.poll_guest_requests();
        if self.view == View::GuestFiles
            && (self.share.workspace_tabs().is_empty()
                || self.share.guest_sub_view != crate::share_panel::GuestSubView::Files)
        {
            self.view = View::Work;
            native_changed = true;
        }
        native_changed |= self.timeline.poll();
        let mut completed_switches = Vec::new();
        for session in &mut self.sessions {
            if let Some(pending) = &session.restarting
                && pending.switch_mode
            {
                let result = pending.result.try_recv();
                if matches!(result, Err(mpsc::TryRecvError::Empty)) {
                    continue;
                }
                session.restarting = None;
                native_changed = true;
                match result {
                    Ok(Ok(())) => completed_switches.push(session.id),
                    Ok(Err((terminal, error))) => {
                        session.terminal = Some(terminal);
                        session.error = error;
                    }
                    Err(_) => {
                        session.error =
                            "Terminal stop was not confirmed; chat mode was kept.".into()
                    }
                }
            }
        }
        for pane in completed_switches {
            self.switch_session_chat_mode(pane);
        }
        if !self.state.panels.right_open
            || self.state.panels.right_tab != crate::workspace::SideTab::Conversations
        {
            self.gpui_extra.conversation_loader.pause();
        }
        if let Some(entries) = self.gpui_extra.session_discovery.poll() {
            self.past_sessions_cache = entries;
            self.gpui_extra.session_query.invalidate();
        }
        self.side_panels_ui.sessions_loading = self.gpui_extra.session_discovery.busy();
        self.side_panels_ui.sessions_limited = self.gpui_extra.session_discovery.limited;
        if let Some(rx) = &self.git_username_rx {
            match rx.try_recv() {
                Ok(name) => {
                    self.git_username = name;
                    self.git_username_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.git_username_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.devtools_rx {
            match rx.try_recv() {
                Ok(statuses) => {
                    self.devtools_statuses = statuses;
                    self.devtools_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.devtools_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.mcp_test_rx {
            match rx.try_recv() {
                Ok((srv_name, Ok(outcome))) => {
                    let french = self.state.language == crate::i18n::Language::French;
                    self.settings_extra.mcp_test_status = match &outcome {
                        crate::settings_extra_model::McpProbeOutcome::Connected(msg) => msg.clone(),
                        crate::settings_extra_model::McpProbeOutcome::SignsInFromAgent => format!(
                            "{srv_name}: {}",
                            crate::settings_extra_model::McpProbeOutcome::signs_in_badge(french)
                        ),
                    };
                    self.settings_extra
                        .mcp_probe_outcomes
                        .insert(srv_name, outcome);
                    self.mcp_test_rx = None;
                }
                Ok((srv_name, Err(err))) => {
                    self.settings_extra.mcp_probe_outcomes.remove(&srv_name);
                    self.settings_extra.mcp_test_status = err;
                    self.mcp_test_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.mcp_test_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(applied) = self.dev_update.poll() {
            let french = self.state.language == crate::i18n::Language::French;
            self.dev_update_notice = Some(match applied {
                crate::dev_update::ApplyDevUpdateResult::Updated(st) => {
                    if french {
                        format!(
                            "Code source mis à jour vers b{} ({}). Recompilez / relancez Tessel.",
                            st.local_build, st.local_commit
                        )
                    } else {
                        format!(
                            "Source updated to b{} ({}). Rebuild / restart Tessel to run it.",
                            st.local_build, st.local_commit
                        )
                    }
                }
                crate::dev_update::ApplyDevUpdateResult::UpToDate(st) => {
                    if french {
                        format!("Déjà à jour sur b{} ({}).", st.local_build, st.local_commit)
                    } else {
                        format!(
                            "Already up to date at b{} ({}).",
                            st.local_build, st.local_commit
                        )
                    }
                }
                crate::dev_update::ApplyDevUpdateResult::Dirty { overlap } => {
                    crate::dev_update::format_overlap_message(&overlap, french)
                }
                crate::dev_update::ApplyDevUpdateResult::Ahead { count } => {
                    if french {
                        format!("Cette branche a {count} commit(s) locaux d'avance.")
                    } else {
                        format!("This branch is {count} local commit(s) ahead.")
                    }
                }
                crate::dev_update::ApplyDevUpdateResult::Error(err) => err,
            });
        }
        self.poll_automations();
        if self.agents.poll() {
            self.detect_agents();
        }
        self.refresh_new_task_agents();
        let mut launch_changed = self
            .model_discovery
            .as_mut()
            .is_some_and(|discovery| discovery.poll());
        self.poll_host_import();
        self.sync_git_project();
        if let Some(completion) = self.git_result.poll() {
            let owner = completion.owner;
            let was_commit = std::mem::take(&mut self.commit_in_flight);
            self.gpui_extra
                .scm_drafts
                .complete(&owner, completion.success);
            self.scm_state.refresh_for(&owner);
            self.explorer_tree.refresh();
            if self.gpui_scm_owner().as_ref() == Some(&owner) {
                if !completion.success {
                    self.error = self.git_result.output.clone();
                } else if was_commit {
                    self.scm_panel.clear_commit_message();
                }
            }
        }
        self.scm_state.poll();
        self.poll_worktrees();
        native_changed |= self.poll_owner_task_launches();
        self.explorer_tree.poll();
        self.remote.poll();
        self.remote
            .poll_native_tree(&mut self.native_hosts, &self.state.hosts);
        self.finder.poll();
        #[cfg(windows)]
        {
            self.browser.poll();
            self.account.poll();
            if let Some(ws) = self.account.take_restored_workspace() {
                self.restore_preview = Some(ws);
            }
            self.agent_browsers.retain(&self.state.projects);
        }
        let mut chat_metadata_changed = false;
        for chat in &mut self.chats {
            native_changed |= chat.poll();
            #[cfg(windows)]
            self.agent_browsers.poll(chat);
        }
        if self.chats.iter().any(|chat| !chat.tool_calls.is_empty()) {
            self.capture_runtime_state();
        }
        for chat in &mut self.chats {
            while let Some(call) = chat.tool_calls.pop_front() {
                #[cfg(windows)]
                if call.name == "tessel_browser" {
                    if !self.state.browser_prefs.separate_agent_session {
                        chat.tool_result(
                            call.id,
                            Err(
                                "Isolated project agent browser sessions are disabled in Settings > Browser."
                                    .into(),
                            ),
                        );
                    } else {
                        self.agent_browsers.dispatch(chat, call);
                    }
                    continue;
                }
                let result = crate::agent_tools::execute_persisted_for_pane(
                    &mut self.state,
                    self.store.as_ref(),
                    &mut self.teams_hub,
                    chat.pane,
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
            native_changed |= chat.poll();
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
        native_changed |= self.poll_owner_task_completions();
        native_changed |= self.poll_owner_task_reviews();
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
        self.poll_agent_installs();
        while let Ok(message) = self.receiver.try_recv() {
            match message {
                Message::TranscriptTrash(entry, result) => {
                    self.transcript_trash_busy = false;
                    match result {
                        Ok(()) => {
                            self.past_sessions_cache.retain(|cached| {
                                !crate::transcript_trash::same_identity(cached, &entry)
                            });
                            self.refresh_past_sessions();
                        }
                        Err(error) => self.error = error,
                    }
                }
                #[cfg(windows)]
                Message::RevealError(error) => self.error = error,
                Message::ExternalOpen(root, result) => {
                    native_changed = true;
                    match result {
                        Ok(Some(path)) if self.state.active().is_some_and(|p| p.path == root) => {
                            self.open_document_at(root, path)
                        }
                        Err(error) => self.error = error,
                        _ => {}
                    }
                }
                Message::DownloadProgress(root, n) => {
                    native_changed = true;
                    self.explorer_tree.download_notice =
                        Some((root, format!("{} {n}", tr("Files copied:"))));
                }
                Message::DownloadFinished(root, result) => {
                    native_changed = true;
                    self.file_action_pending = false;
                    match result {
                        Ok(report) => {
                            self.explorer_tree.download_notice =
                                Some((root, report.message(crate::i18n::is_french())))
                        }
                        Err(error) => {
                            self.explorer_tree.download_notice = None;
                            self.error = error;
                        }
                    }
                }
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
                                    self.documents.retain_mut(|doc| {
                                        if doc.path.starts_with(before) {
                                            if doc.dirty() {
                                                doc.deleted_on_disk = true;
                                                true
                                            } else {
                                                false
                                            }
                                        } else {
                                            true
                                        }
                                    });
                                    self.active_document = self
                                        .active_document
                                        .min(self.documents.len().saturating_sub(1));
                                    if self.documents.is_empty() && self.view == View::Files {
                                        self.view = View::Work;
                                    }
                                    self.pending_close = None;
                                }
                            }
                            self.explorer_tree.refresh();
                            self.scm_state.refresh();
                        }
                    }
                }

                Message::Folder(Some(path)) => match self.state.add_project(&path) {
                    Ok(project_id) => {
                        self.save();
                        self.refresh_project();
                        if !self.sessions.iter().any(|s| s.project == Some(project_id))
                            && let Some(default_id) =
                                self.state.launcher_prefs.remember_default.clone()
                        {
                            self.launch_empty_state_entry(&default_id);
                        }
                    }
                    Err(e) => self.error = e,
                },
                Message::Folder(None) => {}
                Message::AddProjectPickedFolder(Some(path)) => {
                    self.dialogs_toolbar_ui.set_picked_local_folder(path);
                }
                Message::AddProjectPickedFolder(None) => {}
                Message::CloneParentPicked(Some(path)) => {
                    self.dialogs_toolbar_ui.clone_parent = path;
                }
                Message::CloneParentPicked(None) => {}
                Message::InitParentPicked(Some(path)) => {
                    self.dialogs_toolbar_ui.init_parent = path;
                }
                Message::InitParentPicked(None) => {}
                Message::Document {
                    path,
                    request,
                    result,
                } => {
                    let std::collections::hash_map::Entry::Occupied(entry) = self
                        .document_pending
                        .entry(dunce::simplified(&path).to_owned())
                    else {
                        continue;
                    };
                    if !std::sync::Arc::ptr_eq(&entry.get().request, &request) {
                        continue;
                    }
                    let pending = entry.remove();
                    let pinned = pending.pinned;
                    match result {
                        Ok(mut d) => {
                            d.unpinned_preview = !pinned;
                            let d = *d;
                            if let Some(i) = self
                                .documents
                                .iter()
                                .position(|old| old.diff_title.is_none() && old.path == d.path)
                            {
                                if pinned {
                                    self.documents[i].unpinned_preview = false;
                                }
                                self.active_document = i;
                            } else if !pinned
                                && let Some(i) = self
                                    .documents
                                    .iter()
                                    .position(|old| old.unpinned_preview && !old.dirty())
                            {
                                self.documents[i] = d;
                                self.active_document = i;
                            } else if self.documents.len() >= 24 {
                                self.error =
                                    "Close a document before opening more (24-tab limit)".into();
                                continue;
                            } else {
                                self.documents.push(d);
                                self.active_document = self.documents.len() - 1;
                            }
                            self.documents[self.active_document].reveal_location(pending.position);
                            self.view = View::Files;
                        }
                        Err(e) => self.error = e,
                    }
                }
                Message::DiffDocument {
                    source_doc: Some(TimelineOrigin::Pinned(project)),
                    ref path,
                    ..
                } if self.view != View::Files
                    || self.state.active_project != project
                    || self
                        .timeline
                        .pin
                        .as_ref()
                        .is_none_or(|p| &p.path != path || p.project != project) => {}
                Message::DiffDocument {
                    source_doc: Some(TimelineOrigin::Tab(id)),
                    ..
                } if self.view != View::Files
                    || self
                        .documents
                        .get(self.active_document)
                        .is_none_or(|d| d.instance_id() != id) => {}
                Message::DiffDocument {
                    source_doc: _,
                    root,
                    path,
                    area,
                    pinned,
                    result,
                } => match result {
                    Ok(diff_text) => {
                        native_changed = true;
                        let locale = self.tab_language();
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        let title = diff_label(&name, &area, locale);
                        let mut doc =
                            Document::from_diff(&root, path.clone(), title, diff_text, !pinned);
                        doc.diff_area = Some(area);
                        if let Some(i) = self
                            .documents
                            .iter()
                            .position(|old| old.path == path && old.diff_area == doc.diff_area)
                        {
                            let keep_pinned = !self.documents[i].unpinned_preview || pinned;
                            self.documents[i] = doc;
                            if keep_pinned {
                                self.documents[i].unpinned_preview = false;
                            }
                            self.active_document = i;
                        } else if !pinned
                            && let Some(i) = self
                                .documents
                                .iter()
                                .position(|old| old.unpinned_preview && !old.dirty())
                        {
                            self.documents[i] = doc;
                            self.active_document = i;
                        } else if self.documents.len() < 24 {
                            self.documents.push(doc);
                            self.active_document = self.documents.len() - 1;
                        }
                        self.view = View::Files;
                    }
                    Err(e) => self.error = e,
                },
            }
        }
        native_changed |= self.auto_save_documents(std::time::Instant::now());
        if let Some((sid, ref text, press_enter)) = self.pending_quick_command {
            let text_owned = text.clone();
            if let Some(sess) = self.sessions.iter_mut().find(|s| s.id == sid) {
                if let Some(term) = &mut sess.terminal {
                    if !term.exited() {
                        let _ = term.run_quick_command(&text_owned, press_enter);
                    }
                    self.pending_quick_command = None;
                } else if sess.loading.is_none() && sess.restarting.is_none() {
                    self.pending_quick_command = None;
                }
            } else {
                self.pending_quick_command = None;
            }
        }
        // Both renderers consume worker requests here. A pane or failure must
        // also request a GPUI refresh even when no process output has arrived.
        native_changed |= self.poll_teams_and_relay();
        native_changed |= self.linear.poll();
        native_changed |= self.poll_linear_task();
        native_changed
    }
    fn new_session(&mut self, launch: Launch) {
        self.new_session_placed(launch, None);
    }
    fn request_agent_install(&mut self, id: &str) {
        let french = self.state.language == crate::i18n::Language::French;
        let preferred = self.state.launcher_prefs.preferred_shell.as_deref();
        match crate::agent_installs::plan_agent_install(
            id,
            preferred,
            cfg!(windows),
            self.wsl_available,
            french,
        ) {
            Ok(crate::agent_installs::PlannedAgentInstall::Immediate(exec)) => {
                self.start_agent_install(exec);
            }
            Ok(crate::agent_installs::PlannedAgentInstall::NeedsConfirmation(spec)) => {
                self.pending_agent_install = Some(spec);
            }
            Err(error) => {
                self.agents_notice = Some(error.clone());
                self.error = error;
            }
        }
    }
    fn start_agent_install(&mut self, execution: crate::agent_installs::InstallExecution) {
        if self.sessions.len() >= 100 {
            self.error = "Session limit: 100".into();
            return;
        }
        let cwd = self
            .state
            .active()
            .map(|p| p.path.clone())
            .unwrap_or_else(launch::default_directory);
        let id = self.next_session;
        let Some(next) = self.next_session.checked_add(1) else {
            self.error = "Session identifiers exhausted".into();
            return;
        };
        self.next_session = next;
        let split_target = self
            .sessions
            .iter()
            .find(|s| Some(s.id) == self.active_session && s.project == self.state.active_project)
            .map(|s| s.id);
        let mut s = Session {
            profile_data_dir: None,
            profile_root: None,
            profile_migration: None,
            resume_required: false,
            ssh_retry: Default::default(),
            ssh_tmux_id: None,
            native_host: None,
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            settings_changed: false,
            codex_seed: None,
            conversation_id: None,
            profile_binding: None,
            launch_fingerprint: None,
            session_options: None,
            worktree: None,
            pane_name: None,
            broadcast: false,
            id,
            project: self.state.active_project,
            title: execution.agent_name.clone(),
            launch: Launch::Shell,
            cwd,
            scrollback: self.state.terminal_prefs.scrollback,
            terminal: None,
            loading: None,
            error: String::new(),
        };
        s.start_with_shell(None, Some(execution.shell_id));
        self.sessions.push(s);
        if let Some(target) = split_target {
            let layout = self
                .state
                .terminal_layouts
                .entry(crate::terminal_layout::project_key(
                    self.state.active_project,
                ))
                .or_default();
            layout.split(target, id, crate::pane_grid::SplitDir::Col);
        }
        self.active_session = Some(id);
        self.overlay = None;
        self.view = View::Work;
        self.agents_notice = Some(execution.notice);
        self.active_agent_installs.push(ActiveAgentInstall {
            session_id: id,
            agent_name: execution.agent_name,
            pending_command: Some(execution.command_line),
            attached_at: None,
            finished: false,
        });
        self.save();
    }
    fn poll_agent_installs(&mut self) {
        if self.active_agent_installs.is_empty() {
            return;
        }
        let french = self.state.language == crate::i18n::Language::French;
        let mut need_redetect = false;
        let mut error_msg: Option<String> = None;
        let mut installed_notice: Option<String> = None;
        for inst in &mut self.active_agent_installs {
            let Some(session) = self.sessions.iter_mut().find(|s| s.id == inst.session_id) else {
                inst.finished = true;
                continue;
            };
            if !session.error.is_empty() {
                inst.finished = true;
                continue;
            }
            let Some(term) = session.terminal.as_mut() else {
                continue;
            };
            let attached = *inst.attached_at.get_or_insert_with(std::time::Instant::now);
            if inst.pending_command.is_some() {
                if attached.elapsed() >= Duration::from_millis(350)
                    && term.accepts_broadcast_input()
                    && let Some(cmd) = inst.pending_command.take()
                    && let Err(err) = term.send_installer_line(&cmd)
                {
                    error_msg = Some(err);
                    inst.finished = true;
                }
            } else if term.screen_contains(crate::agent_installs::MARK_OK) {
                inst.finished = true;
                need_redetect = true;
                installed_notice = Some(if french {
                    format!("{} est installé.", inst.agent_name)
                } else {
                    format!("{} is installed.", inst.agent_name)
                });
            } else if term.screen_contains(crate::agent_installs::MARK_FAILED) || term.exited() {
                inst.finished = true;
                need_redetect = true;
            }
        }
        self.active_agent_installs.retain(|i| !i.finished);
        if let Some(err) = error_msg {
            self.error = err;
        }
        if let Some(notice) = installed_notice {
            self.agents_notice = Some(notice);
        }
        if need_redetect {
            self.detect_agents();
        }
    }
    pub(crate) fn save_builtin_agent_prefs(
        &mut self,
        id: &str,
        prefs: crate::agent_preferences::Preferences,
    ) {
        self.agents_notice = self
            .store
            .as_ref()
            .ok_or_else(|| "Settings store unavailable".to_owned())
            .and_then(|store| crate::agent_preferences::save(&mut self.state, store, id, prefs))
            .err();
        self.refresh_launch_status();
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
        self.new_session_in_worktree(self.state.active_project, launch, split, before, None);
    }

    fn new_session_in_worktree(
        &mut self,
        project_id: Option<u64>,
        launch: Launch,
        split: Option<(u64, crate::pane_grid::SplitDir)>,
        before: bool,
        worktree: Option<workspace::SavedWorktree>,
    ) {
        let _ = self.new_session_with_history(project_id, launch, split, before, worktree, None);
    }
    fn new_session_with_history(
        &mut self,
        project_id: Option<u64>,
        launch: Launch,
        split: Option<(u64, crate::pane_grid::SplitDir)>,
        before: bool,
        worktree: Option<workspace::SavedWorktree>,
        resume: Option<&crate::side_panels_model::SessionHistoryEntry>,
    ) -> Option<u64> {
        self.new_session_at(
            project_id,
            launch,
            split,
            before,
            worktree,
            resume.map_or(SessionStart::Project, SessionStart::History),
        )
    }
    fn new_session_at(
        &mut self,
        project_id: Option<u64>,
        launch: Launch,
        split: Option<(u64, crate::pane_grid::SplitDir)>,
        before: bool,
        worktree: Option<workspace::SavedWorktree>,
        start: SessionStart<'_>,
    ) -> Option<u64> {
        let (directory, resume) = match start {
            SessionStart::Project => (None, None),
            SessionStart::Directory(path) => (Some(path), None),
            SessionStart::History(entry) => (Some(entry.folder.clone()), Some(entry)),
        };
        if launch.native_chat().is_some() && project_id.is_none() {
            self.error = tr("Select a project for native chat.").into();
            return None;
        }
        if self.sessions.len() >= 100 {
            self.error = "Session limit: 100".into();
            return None;
        }
        // Only a selected local project is eligible. Perform the account-specific
        // write on the launch worker before the child starts, never in a racing thread.
        let trust_home = if self.state.agent_folder_trust
            && matches!(&launch, Launch::Agent(id) if id == "claude")
            && worktree.is_none()
            && project_id
                .and_then(|pid| self.state.projects.iter().find(|p| p.id == pid))
                .is_some()
        {
            directories::BaseDirs::new().map(|b| b.home_dir().to_owned())
        } else {
            None
        };
        let cwd = directory
            .or_else(|| worktree.as_ref().map(|wt| wt.path.clone()))
            .or_else(|| {
                project_id.and_then(|pid| {
                    self.state
                        .projects
                        .iter()
                        .find(|p| p.id == pid)
                        .map(|p| p.path.clone())
                })
            })
            .unwrap_or_else(launch::default_directory);
        let title = match &launch {
            Launch::CodexChat => "Codex".into(),
            Launch::ClaudeChat => "Claude".into(),
            Launch::Shell => match self.state.launcher_prefs.preferred_shell.as_deref() {
                Some("cmd") if cfg!(windows) => "CMD".into(),
                Some("wsl") if cfg!(windows) => "WSL".into(),
                _ => "Terminal".into(),
            },
            Launch::Agent(n) => self
                .state
                .custom_agents
                .iter()
                .find(|c| c.id == *n)
                .map_or_else(|| n.clone(), |c| c.name.clone()),
            Launch::Ssh(h) => format!("SSH · {}", h.alias),
        };
        let id = self.next_session;
        let Some(next) = self.next_session.checked_add(1) else {
            self.error = "Session identifiers exhausted".into();
            return None;
        };
        self.next_session = next;
        let mut s = Session {
            profile_data_dir: None,
            profile_root: None,
            profile_migration: None,
            resume_required: resume.is_some(),
            ssh_retry: Default::default(),
            ssh_tmux_id: None,
            native_host: None,
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            settings_changed: false,
            codex_seed: None,
            conversation_id: resume.map(|entry| entry.id.clone()),
            profile_binding: resume.and_then(|entry| entry.profile_binding.clone()),
            launch_fingerprint: None,
            session_options: None,
            worktree,
            pane_name: None,
            broadcast: true,
            id,
            project: project_id,
            title,
            launch,
            cwd,
            scrollback: self.state.terminal_prefs.scrollback,
            terminal: None,
            loading: None,
            error: String::new(),
        };
        let prefs = match &s.launch {
            Launch::Agent(id) => {
                let mut prefs = crate::agent_preferences::for_pane_launch(
                    &self.state,
                    id,
                    self.model_discovery.as_ref().and_then(|d| d.models(id)),
                    s.session_options.as_ref(),
                );
                if resume.is_none()
                    && matches!(id.as_str(), "claude" | "codex")
                    && let Some(store) = &self.store
                    && let Some(env_line) =
                        crate::settings_extra_model::active_managed_env_line(store.directory(), id)
                {
                    let key = crate::session_profile::env_key(id).expect("supported provider");
                    if crate::agent_env::parse(&prefs.env)
                        .is_ok_and(|env| crate::agent_env::effective_value(&env, key).is_none())
                    {
                        if !prefs.env.trim().is_empty() && !prefs.env.ends_with('\n') {
                            prefs.env.push('\n');
                        }
                        prefs.env.push_str(&env_line);
                    }
                }
                Some(prefs)
            }
            _ => None,
        };
        let preferred_shell = matches!(s.launch, Launch::Shell)
            .then(|| self.state.launcher_prefs.preferred_shell.clone())
            .flatten();
        s.profile_data_dir = self
            .store
            .as_ref()
            .map(|store| store.directory().to_owned());
        s.start_with_trust(prefs, preferred_shell, trust_home);
        if let Some((target, dir)) = split {
            let siblings: Vec<_> = self
                .sessions
                .iter()
                .filter(|session| session.project == project_id)
                .map(|session| session.id)
                .collect();
            let layout = self
                .state
                .terminal_layouts
                .entry(crate::terminal_layout::project_key(project_id))
                .or_default();
            layout.sync(&siblings, self.grid_room);
            if before {
                layout.split_with_order(target, id, dir, true);
            } else {
                layout.split(target, id, dir);
            }
        }
        self.sessions.push(s);
        self.active_session = Some(id);
        self.view = View::Work;
        self.save();
        Some(id)
    }
    fn session_is_running_agent(&self, id: u64) -> bool {
        let Some(sess) = self.sessions.iter().find(|s| s.id == id) else {
            return false;
        };
        match &sess.launch {
            Launch::Agent(_) => {
                (sess.loading.is_some() || sess.profile_migration.is_some())
                    || sess.restarting.is_some()
                    || sess.terminal.as_ref().is_some_and(|t| !t.exited())
            }
            Launch::CodexChat => self.chats.iter().any(|c| c.pane == id && c.busy()),
            Launch::ClaudeChat => self.claude_chats.iter().any(|c| c.pane == id && c.busy()),
            _ => false,
        }
    }
    fn close_session(&mut self, id: u64) {
        if self.state.general_prefs.confirm_close_agent && self.session_is_running_agent(id) {
            self.pending_close_session = Some(id);
            return;
        }
        self.close_session_immediate(id);
    }
    fn close_session_immediate(&mut self, id: u64) {
        #[cfg(windows)]
        if self
            .sessions
            .iter_mut()
            .find(|session| session.id == id)
            .and_then(|session| session.terminal.as_mut())
            .is_some_and(|terminal| !terminal.request_native_close())
        {
            return;
        }
        self.remove_session(id);
    }
    fn remove_session(&mut self, id: u64) {
        self.cancel_owner_task_pane(id);
        self.team_mcp.revoke(id);
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
                    Kind::Trash => "Move to Recycle Bin",
                    Kind::Delete => "Delete permanently",
                }));
                ui.label(action.path.display().to_string());
                if action.kind == Kind::Delete {
                    ui.label(tr(
                        "This does not use a recycle bin. Only empty folders can be deleted.",
                    ));
                } else if action.kind == Kind::Trash {
                    ui.label(tr(
                        "Moves this item to the Recycle Bin so it can be restored.",
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
    fn refresh_language(&mut self) {
        crate::i18n::set_language(self.state.language);
        let lang = self.tab_language();
        for doc in &mut self.documents {
            if let Some(area) = &doc.diff_area {
                let name = doc.path.file_name().unwrap_or_default().to_string_lossy();
                doc.diff_title = Some(diff_label(&name, area, lang));
            }
        }
    }
    fn tab_language(&self) -> TabsLang {
        if self.state.language == crate::i18n::Language::French {
            TabsLang::Fr
        } else {
            TabsLang::En
        }
    }
    fn document_tab_labels(&self, paths: &[String]) -> (Vec<String>, Vec<(String, String)>) {
        use crate::view_tabs_browser_model::{file_name_of, qualified_tab_name};
        let keys: Vec<_> = paths
            .iter()
            .zip(&self.documents)
            .map(|(path, doc)| (file_name_of(path).to_lowercase(), doc.diff_title.is_some()))
            .collect();
        let tabs = self.share.workspace_tabs();
        let guest_keys: Vec<_> = tabs
            .iter()
            .map(|tab| (file_name_of(&tab.path).to_lowercase(), false))
            .collect();
        let mut counts = std::collections::HashMap::new();
        for key in keys.iter().chain(&guest_keys) {
            *counts.entry(key).or_insert(0usize) += 1;
        }
        let local = paths
            .iter()
            .zip(&self.documents)
            .zip(&keys)
            .map(|((path, doc), key)| {
                let label = qualified_tab_name(path, counts[key] > 1);
                if let Some(area) = &doc.diff_area {
                    return diff_label(&label, area, self.tab_language());
                }
                match &doc.diff_title {
                    Some(title) => match title.strip_prefix(file_name_of(path)) {
                        Some(suffix) if suffix.starts_with(" (") => format!("{label}{suffix}"),
                        _ => title.clone(),
                    },
                    None => label,
                }
            })
            .collect();
        let context = self.share.workspace_tab_context();
        let guests = tabs
            .iter()
            .zip(&guest_keys)
            .map(|(tab, key)| {
                let twin = counts[key] > 1;
                let label = if twin
                    && !tab.path.contains('/')
                    && let Some((_, project)) = &context
                {
                    format!("{} · {project}", tab.path)
                } else {
                    qualified_tab_name(&tab.path, twin)
                };
                let host = context
                    .as_ref()
                    .map_or("Live Share", |(name, _)| name.as_str());
                let path = &tab.path;
                let hint = match self.tab_language() {
                    TabsLang::Fr => format!("Onglet de {host} : {path}"),
                    TabsLang::En => format!("{host}’s tab: {path}"),
                };
                (label, hint)
            })
            .collect();
        (local, guests)
    }
    fn center_tabs(&mut self, ui: &mut egui::Ui) {
        let paths: Vec<_> = self
            .documents
            .iter()
            .map(|d| d.path.to_string_lossy().into_owned())
            .collect();
        let (labels, guest_labels) = self.document_tab_labels(&paths);
        let mut close = None;
        let mut keep_open = None;
        let mut close_others = None;
        let mut close_all = false;
        #[cfg(windows)]
        let mut close_browser = false;
        egui::ScrollArea::horizontal()
            .id_salt("center-tabs")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let terminal = crate::view_tabs::show(
                        ui,
                        crate::view_tabs::Tab {
                            title: tr("Terminal"),
                            file_path: None,
                            hint: tr("Terminal"),
                            kind: crate::view_tabs::Kind::Terminal,
                            selected: self.view == View::Work,
                            dirty: false,
                            closable: false,
                            preview: false,
                            deleted: false,
                            has_file_menu: false,
                        },
                    );
                    if terminal.body.clicked() {
                        self.view = View::Work;
                    }
                    #[cfg(windows)]
                    if self.browser_tab_open || self.view == View::Browser {
                        let browser = crate::view_tabs::show(
                            ui,
                            crate::view_tabs::Tab {
                                title: tr("Browser"),
                                file_path: None,
                                hint: tr("Browser"),
                                kind: crate::view_tabs::Kind::Browser,
                                selected: self.view == View::Browser,
                                dirty: false,
                                closable: true,
                                preview: false,
                                deleted: false,
                                has_file_menu: false,
                            },
                        );
                        if browser.close {
                            close_browser = true;
                        } else if browser.body.clicked() {
                            self.view = View::Browser;
                        }
                    }
                    for (i, document) in self.documents.iter().enumerate() {
                        let title = &labels[i];
                        let kind = if document.diff_title.is_some() {
                            crate::view_tabs::Kind::Diff
                        } else {
                            crate::view_tabs::Kind::Editor
                        };
                        let tab = crate::view_tabs::show(
                            ui,
                            crate::view_tabs::Tab {
                                title,
                                file_path: Some(&paths[i]),
                                hint: &paths[i],
                                kind,
                                selected: self.view == View::Files && self.active_document == i,
                                dirty: document.dirty(),
                                closable: true,
                                preview: document.unpinned_preview,
                                deleted: document.deleted_on_disk,
                                has_file_menu: true,
                            },
                        );
                        if tab.keep_open {
                            keep_open = Some(i);
                        }
                        if tab.copy_path {
                            ui.ctx().copy_text(paths[i].clone());
                        }
                        if tab.close {
                            close = Some(i);
                        } else if tab.close_others {
                            close_others = Some(i);
                        } else if tab.close_all {
                            close_all = true;
                        } else if tab.body.clicked() {
                            self.view = View::Files;
                            self.active_document = i;
                        }
                    }
                    let mut clicked = None;
                    for (key, (title, hint)) in
                        self.share.workspace_tabs().iter().zip(&guest_labels)
                    {
                        let tab = crate::view_tabs::show(
                            ui,
                            crate::view_tabs::Tab {
                                title,
                                file_path: Some(&key.path),
                                hint,
                                kind: crate::view_tabs::Kind::Editor,
                                selected: self.view == View::GuestFiles
                                    && self.share.guest_tab_active(key),
                                dirty: self.share.guest_tab_dirty(key),
                                closable: true,
                                preview: false,
                                deleted: false,
                                has_file_menu: false,
                            },
                        );
                        if tab.close || tab.body.clicked() {
                            clicked = Some((key.clone(), tab.close));
                        }
                    }
                    if let Some((key, close)) = clicked {
                        if close {
                            self.share.close_guest_tab(&key);
                        } else if self.share.select_workspace_file(key) {
                            self.view = View::GuestFiles;
                        }
                    }
                });
            });
        #[cfg(windows)]
        if close_browser {
            self.browser_tab_open = false;
            self.browser.close_popups();
            if self.view == View::Browser {
                self.view = if self.documents.is_empty() {
                    View::Work
                } else {
                    View::Files
                };
            }
        }
        if let Some(index) = keep_open
            && let Some(doc) = self.documents.get_mut(index)
        {
            doc.unpinned_preview = false;
        }
        if let Some(keep_idx) = close_others {
            for idx in (0..self.documents.len()).rev() {
                if idx == keep_idx {
                    continue;
                }
                if self.documents[idx].dirty() {
                    if self.pending_close.is_none() {
                        self.pending_close = Some(idx);
                    }
                } else {
                    self.remove_document(idx);
                }
            }
        } else if close_all {
            for idx in (0..self.documents.len()).rev() {
                if self.documents[idx].dirty() {
                    if self.pending_close.is_none() {
                        self.pending_close = Some(idx);
                    }
                } else {
                    self.remove_document(idx);
                }
            }
        } else if let Some(index) = close {
            self.request_document_close(index);
        }
        ui.separator();
    }
    fn default_new_launch(&self) -> Launch {
        for preferred in ["claude", "codex"] {
            if crate::agent_preferences::enabled(&self.state, preferred)
                && self
                    .agents
                    .entries()
                    .iter()
                    .any(|(id, installed)| *installed && id == preferred)
            {
                return Launch::Agent(preferred.into());
            }
        }
        if let Some((id, _)) = self.agents.entries().iter().find(|(id, installed)| {
            *installed && crate::agent_preferences::enabled(&self.state, id)
        }) {
            return Launch::Agent(id.clone());
        }
        Launch::Shell
    }

    fn cycle_workspace(&mut self, step: i32) {
        let len = self.state.projects.len();
        if len == 0 {
            return;
        }
        let current = self
            .state
            .active_project
            .and_then(|id| self.state.projects.iter().position(|p| p.id == id));
        let next_idx = match current {
            Some(idx) => {
                let n = len as i32;
                ((idx as i32 + step).rem_euclid(n)) as usize
            }
            None => {
                if step < 0 {
                    len - 1
                } else {
                    0
                }
            }
        };
        let next = self.state.projects[next_idx].id;
        self.select_workspace_project(next);
    }

    fn sync_waiting_notifications(&mut self) -> Vec<(u64, &'static str)> {
        let waiting_panes: Vec<_> = self
            .chats
            .iter()
            .filter(|c| c.waiting_for_user())
            .map(|c| (c.pane, "Codex chat waiting for input"))
            .chain(
                self.claude_chats
                    .iter()
                    .filter(|c| c.waiting_for_user())
                    .map(|c| (c.pane, "Claude chat waiting for input")),
            )
            .collect();
        self.notified_waiting_panes
            .retain(|pid| waiting_panes.iter().any(|(id, _)| id == pid));
        for (pid, detail) in &waiting_panes {
            if self.notified_waiting_panes.insert(*pid)
                && self.state.notification_prefs.notifications_enabled
                && self.state.notification_prefs.in_app_alerts
            {
                self.push_notification(
                    crate::dialogs_toolbar_model::NotificationKind::ApprovalNeeded,
                    "Agent waiting for response",
                    *detail,
                    Some(*pid),
                );
            }
        }
        waiting_panes
    }

    fn reveal_workspace_pane(&mut self, id: u64) {
        let Some(session) = self.sessions.iter().find(|session| session.id == id) else {
            return;
        };
        let project = session.project;
        let was_maximized = self
            .state
            .terminal_layouts
            .get(&crate::terminal_layout::project_key(
                self.state.active_project,
            ))
            .is_some_and(|layout| layout.maximized.is_some());
        if self.state.active_project != project {
            self.state.active_project = project;
            self.refresh_project();
        }
        if let Some(layout) = self
            .state
            .terminal_layouts
            .get_mut(&crate::terminal_layout::project_key(project))
            && (was_maximized || layout.maximized.is_some())
        {
            layout.maximized = Some(id);
        }
        self.read_pane_notifications(id);
        self.active_session = Some(id);
        self.view = View::Work;
        self.save();
    }

    fn select_workspace_project(&mut self, id: u64) {
        if self.state.active_project == Some(id)
            || !self.state.projects.iter().any(|project| project.id == id)
        {
            return;
        }
        self.state.active_project = Some(id);
        self.view = View::Work;
        self.refresh_project();
        self.save();
    }

    fn zoom_font(&mut self, delta: i8) {
        let next = if delta == 0 {
            15.0_f32
        } else {
            (self.state.font_size + f32::from(delta)).clamp(10.0_f32, 28.0_f32)
        };
        if (self.state.font_size - next).abs() > f32::EPSILON {
            self.state.font_size = next;
            self.save();
        }
    }

    fn finder_target(&self) -> Option<u64> {
        self.active_session.filter(|id| {
            self.sessions.iter().any(|session| {
                session.id == *id
                    && session.project == self.state.active_project
                    && session
                        .terminal
                        .as_ref()
                        .is_some_and(|t| !t.exited() && t.accepts_broadcast_input())
            })
        })
    }
    fn handle_finder_action(
        &mut self,
        ctx: &egui::Context,
        action: crate::finder_panel::FinderAction,
    ) {
        use crate::finder_panel::FinderAction;
        match action {
            FinderAction::OpenFile(path) => self.open_document(path),
            FinderAction::InsertInTerminal { path, relative } => {
                if let Some(target) = self.finder_target()
                    && let Some(term) = self
                        .sessions
                        .iter_mut()
                        .find(|s| s.id == target)
                        .and_then(|s| s.terminal.as_mut())
                {
                    let text = if relative.chars().any(char::is_whitespace) {
                        format!("\"{relative}\"")
                    } else {
                        relative
                    };
                    term.paste_text(&text);
                    term.request_focus(ctx);
                    self.view = View::Work;
                } else {
                    self.open_document(path);
                }
            }
        }
    }

    fn paste_into_active_terminal(&mut self, ctx: &egui::Context, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        if self.floating.ui.is_open()
            && self.floating.ui.has_keyboard()
            && self.floating.paste_text(text)
        {
            return true;
        }
        let target_id = self
            .active_session
            .filter(|id| {
                self.sessions.iter().any(|s| {
                    s.id == *id
                        && s.project == self.state.active_project
                        && s.terminal.as_ref().is_some_and(|t| !t.exited())
                })
            })
            .or_else(|| {
                self.sessions
                    .iter()
                    .find(|s| {
                        s.project == self.state.active_project
                            && s.terminal.as_ref().is_some_and(|t| !t.exited())
                    })
                    .map(|s| s.id)
            });
        if let Some(id) = target_id
            && let Some(session) = self.sessions.iter_mut().find(|s| s.id == id)
            && let Some(terminal) = &mut session.terminal
        {
            terminal.paste_text(text);
            terminal.request_focus(ctx);
            self.active_session = Some(id);
            self.view = View::Work;
            return true;
        }
        false
    }

    fn run_quick_command_by_id(&mut self, ctx: &egui::Context, cmd_id: u64) {
        let Some(cmd) = self
            .state
            .quick_commands
            .iter()
            .find(|c| c.id == cmd_id)
            .cloned()
        else {
            return;
        };
        if cmd.text.is_empty() {
            return;
        }
        self.overlay = None;
        if self.floating.ui.is_open()
            && self.floating.ui.has_keyboard()
            && self.floating.run_quick_command(&cmd.text, cmd.press_enter)
        {
            return;
        }
        let target_id = self
            .active_session
            .filter(|id| {
                self.sessions.iter().any(|s| {
                    s.id == *id
                        && s.project == self.state.active_project
                        && s.terminal.as_ref().is_some_and(|t| !t.exited())
                })
            })
            .or_else(|| {
                self.sessions
                    .iter()
                    .find(|s| {
                        s.project == self.state.active_project
                            && s.terminal.as_ref().is_some_and(|t| !t.exited())
                    })
                    .map(|s| s.id)
            });
        if let Some(id) = target_id
            && let Some(session) = self.sessions.iter_mut().find(|s| s.id == id)
            && let Some(terminal) = &mut session.terminal
        {
            let _ = terminal.run_quick_command(&cmd.text, cmd.press_enter);
            terminal.request_focus(ctx);
            self.active_session = Some(id);
            self.view = View::Work;
            return;
        }
        self.new_session(Launch::Shell);
        if let Some(id) = self.active_session {
            self.pending_quick_command = Some((id, cmd.text, cmd.press_enter));
        }
    }

    pub(crate) fn current_settings_page_id(&self) -> &'static str {
        match self.settings_section {
            SettingsSection::Agents => "agents",
            SettingsSection::Accounts => "accounts",
            SettingsSection::Skills => "skills",
            SettingsSection::Mcp => "mcp",
            SettingsSection::Connections => "connections",
            SettingsSection::Context => "context",
            SettingsSection::Orchestration => "orchestration",
            #[cfg(windows)]
            SettingsSection::Account => "tessel-account",
            SettingsSection::General => "general",
            SettingsSection::Appearance => "appearance",
            SettingsSection::Terminal => "terminal",
            SettingsSection::Editor => "editor",
            SettingsSection::Browser => "browser",
            SettingsSection::Alerts => "alerts",
            SettingsSection::Voice => "voice",
            #[cfg(windows)]
            SettingsSection::Passwords => "passwords",
            SettingsSection::Git => "git",
            SettingsSection::Devtools => "devtools",
            SettingsSection::Automations => "automations",
            SettingsSection::QuickCommands => "quick-commands",
            SettingsSection::Hosts => "ssh",
            SettingsSection::Stats => "stats",
            SettingsSection::Updates => "updates",
        }
    }

    fn show_command_palette(&mut self, ctx: &egui::Context) {
        if !self.command_palette.open {
            self.command_palette_focused = false;
            return;
        }
        let current_theme = crate::theme::active(ctx);
        let commands = self.build_palette_commands(current_theme);
        let selected = crate::command_palette_panel::show(
            ctx,
            &mut self.command_palette,
            &mut self.command_palette_focused,
            &commands,
        );
        if let Some(id) = selected {
            self.command_palette.close();
            self.execute_palette_command(ctx, &id);
        }
        if !self.command_palette.open {
            self.command_palette_focused = false;
        }
    }

    pub(crate) fn build_palette_commands(
        &self,
        current_theme: crate::theme_palettes::ThemeId,
    ) -> Vec<crate::palette::PaletteCommand> {
        use crate::palette::PaletteCommand;
        let french = self.state.language == crate::i18n::Language::French;
        let mut commands: Vec<PaletteCommand> = Vec::new();

        // 1. New
        let new_group = tr("New");
        commands.push(
            PaletteCommand::new("new:default", new_group, tr("New default pane"))
                .with_shortcut("Ctrl+Shift+T"),
        );
        commands.push(PaletteCommand::new(
            "terminal",
            new_group,
            tr("Open terminal"),
        ));
        commands.push(
            PaletteCommand::new("launcher", new_group, tr("Open a terminal or agent"))
                .with_shortcut("Ctrl+Shift+Space"),
        );
        for (agent_id, installed) in self.agents.entries() {
            if *installed && crate::agent_preferences::enabled(&self.state, agent_id) {
                let agent_name = crate::agent_preferences::display_name(&self.state, agent_id);
                commands.push(PaletteCommand::new(
                    format!("new:agent:{agent_id}"),
                    new_group,
                    agent_name,
                ));
            }
        }
        if crate::agent_preferences::enabled(&self.state, "codex") {
            commands.push(
                PaletteCommand::new("chat", new_group, tr("Native Codex chat")).with_hint(tr(
                    "Codex without a terminal: team messages reach it as turns of their own",
                )),
            );
        }
        if crate::agent_preferences::enabled(&self.state, "claude") {
            commands.push(
                PaletteCommand::new("claude-chat", new_group, tr("Native Claude chat")).with_hint(
                    tr("Claude without a terminal: team messages reach it as turns of their own"),
                ),
            );
        }

        // 2. Project / Workspaces
        let proj_group = tr("Project");
        commands.push(
            PaletteCommand::new("project:new", proj_group, tr("Open project folder…"))
                .with_shortcut("Ctrl+Shift+N"),
        );
        if let Some(active_pid) = self.state.active_project
            && let Some(summary) = self.worktree_state.summary(active_pid)
            && summary.has_commits
        {
            commands.push(PaletteCommand::new(
                "worktree:new",
                proj_group,
                tr("New worktree pane"),
            ));
            if summary.non_primary_count() > 0 {
                commands.push(PaletteCommand::new(
                    "worktree:cleanup",
                    proj_group,
                    tr("Clean up worktrees…"),
                ));
            }
        }
        if self.state.projects.len() > 1 {
            commands.push(
                PaletteCommand::new("project:prev", proj_group, tr("Previous workspace"))
                    .with_shortcut("Ctrl+PageUp"),
            );
            commands.push(
                PaletteCommand::new("project:next", proj_group, tr("Next workspace"))
                    .with_shortcut("Ctrl+PageDown"),
            );
        }

        // 3. Layout
        let pane_count = self
            .sessions
            .iter()
            .filter(|s| s.project == self.state.active_project)
            .count();
        let active_pane_id = self.active_project_work_pane();
        let has_active_pane = active_pane_id.is_some();
        let can_apply_grid = pane_count > 0 || self.sessions.len() < 100;
        let layout_group = crate::layout_broadcast_ui::layout_toolbar_label(french);
        let layout_key = crate::terminal_layout::project_key(self.state.active_project);
        let is_maximized = self
            .state
            .terminal_layouts
            .get(&layout_key)
            .and_then(|l| l.maximized)
            .is_some();
        if has_active_pane {
            let max_cmd = PaletteCommand::new(
                "layout:maximize",
                layout_group,
                tr(if is_maximized {
                    "Restore pane"
                } else {
                    "Maximize pane"
                }),
            );
            commands.push(if is_maximized {
                max_cmd.with_shortcut("Esc")
            } else {
                max_cmd
            });
            commands.push(
                PaletteCommand::new(
                    "layout:split-right",
                    layout_group,
                    crate::layout_broadcast_ui::split_right_label(french),
                )
                .with_shortcut("Ctrl+Shift+E"),
            );
            commands.push(
                PaletteCommand::new(
                    "layout:split-down",
                    layout_group,
                    crate::layout_broadcast_ui::split_down_label(french),
                )
                .with_shortcut("Ctrl+Shift+O"),
            );
        }
        if pane_count > 1 {
            commands.push(
                PaletteCommand::new(
                    "layout:arrange",
                    layout_group,
                    crate::layout_broadcast_ui::arrange_grid_label(french),
                )
                .with_hint(crate::layout_broadcast_ui::arrange_grid_hint(french)),
            );
        }
        if can_apply_grid {
            for preset in crate::pane_grid::GridPreset::ALL {
                commands.push(PaletteCommand::new(
                    format!(
                        "layout:preset:{}",
                        crate::layout_broadcast_ui::preset_display_label(preset)
                    ),
                    layout_group,
                    crate::layout_broadcast_ui::even_grid_command_title(french, preset),
                ));
            }
        }
        if let Some(active_id) = active_pane_id
            && let Some(sess) = self.sessions.iter().find(|s| s.id == active_id)
        {
            let prefs = match &sess.launch {
                Launch::Agent(agent) => Some(crate::agent_preferences::for_pane_launch(
                    &self.state,
                    agent,
                    self.model_discovery.as_ref().and_then(|d| d.models(agent)),
                    sess.session_options.as_ref(),
                )),
                _ => None,
            };
            if sess.can_restart(prefs.as_ref()) {
                commands.push(
                    PaletteCommand::new(
                        "layout:restart-active",
                        layout_group,
                        tr("Restart active pane"),
                    )
                    .with_shortcut("Ctrl+Shift+R"),
                );
            }
            if sess.terminal.is_some() {
                commands.push(
                    PaletteCommand::new(
                        "layout:reset-terminal",
                        layout_group,
                        tr("Reset active terminal"),
                    )
                    .with_hint(tr("Turns off modes a program left on; it keeps running")),
                );
            }
        }
        if has_active_pane {
            commands.push(
                PaletteCommand::new(
                    "layout:close-active",
                    layout_group,
                    crate::layout_broadcast_ui::close_active_command_title(french),
                )
                .with_shortcut("Ctrl+Shift+W"),
            );
        }
        commands.push(
            PaletteCommand::new("floating", layout_group, tr("Toggle floating terminal")).with_hint(tr("A terminal over the workspace, in the project's folder; it keeps running when hidden"))
                .with_shortcut("Ctrl+`"),
        );
        commands.push(PaletteCommand::new(
            "layout:toggle-left",
            layout_group,
            tr(if self.state.panels.left_open {
                "Hide left sidebar"
            } else {
                "Show left sidebar"
            }),
        ));
        commands.push(PaletteCommand::new(
            "layout:toggle-right",
            layout_group,
            tr(if self.state.panels.right_open {
                "Hide right panel"
            } else {
                "Show right panel"
            }),
        ));
        commands.push(PaletteCommand::new(
            "layout:toggle-status-bar",
            layout_group,
            tr(if self.state.status_bar.visible {
                "Hide status bar"
            } else {
                "Show status bar"
            }),
        ));

        // 4. Files & Git
        let files_group = tr("Files");
        commands.push(
            PaletteCommand::new("finder", files_group, tr("Find a file"))
                .with_shortcut("Ctrl+Shift+J"),
        );
        let files_showing =
            self.state.panels.right_open && self.state.panels.right_tab == RightTab::Files;
        commands.push(
            PaletteCommand::new(
                "files",
                files_group,
                tr(if files_showing {
                    "Hide file explorer"
                } else {
                    "Show file explorer"
                }),
            )
            .with_shortcut("Ctrl+Shift+X"),
        );
        let changes_showing =
            self.state.panels.right_open && self.state.panels.right_tab == RightTab::Changes;
        commands.push(
            PaletteCommand::new(
                "changes",
                files_group,
                tr(if changes_showing {
                    "Hide Git changes"
                } else {
                    "Show Git changes"
                }),
            )
            .with_shortcut("Ctrl+Shift+G"),
        );
        commands.push(PaletteCommand::new(
            "remote",
            files_group,
            tr("Remote files"),
        ));

        // 5. Task & Side Panels
        let task_group = tr("Task");
        let dashboard_showing =
            self.state.panels.right_open && self.state.panels.right_tab == RightTab::Dashboard;
        commands.push(
            PaletteCommand::new(
                "dashboard",
                task_group,
                tr(if dashboard_showing {
                    "Hide agent dashboard"
                } else {
                    "Show agent dashboard"
                }),
            )
            .with_hint(tr("Every agent of every project at a glance")),
        );
        let tasks_showing =
            self.state.panels.right_open && self.state.panels.right_tab == RightTab::Tasks;
        commands.push(
            PaletteCommand::new(
                "tasks",
                task_group,
                tr(if tasks_showing {
                    "Hide task board"
                } else {
                    "Show task board"
                }),
            )
            .with_shortcut("Ctrl+Shift+K"),
        );
        let task_hist_showing =
            self.state.panels.right_open && self.state.panels.right_tab == RightTab::TaskHistory;
        commands.push(
            PaletteCommand::new(
                "task_history",
                task_group,
                tr(if task_hist_showing {
                    "Hide task history"
                } else {
                    "Show task history"
                }),
            )
            .with_hint(tr(
                "Finished tasks with their time, tokens and estimated cost",
            )),
        );
        let past_showing =
            self.state.panels.right_open && self.state.panels.right_tab == RightTab::History;
        commands.push(
            PaletteCommand::new(
                "history",
                task_group,
                tr(if past_showing {
                    "Hide past conversations"
                } else {
                    "Show past conversations"
                }),
            )
            .with_hint(tr("Browse, search and resume past agent conversations")),
        );
        commands.push(
            PaletteCommand::new("sessions:modal", task_group, tr("Past conversations"))
                .with_hint(tr("Reopen a past agent conversation")),
        );
        let conv_showing =
            self.state.panels.right_open && self.state.panels.right_tab == RightTab::Conversations;
        commands.push(
            PaletteCommand::new(
                "conversations",
                task_group,
                tr(if conv_showing {
                    "Hide conversation threads"
                } else {
                    "Show conversation threads"
                }),
            )
            .with_hint(tr(
                "What the agents of a team said to each other and to you",
            )),
        );
        if self.state.active_project.is_some() {
            commands.push(PaletteCommand::new("task:new", task_group, tr("New task…")));
            commands.push(PaletteCommand::new(
                "notes",
                files_group,
                tr("Project notes…"),
            ));
            commands.push(PaletteCommand::new(
                "review",
                task_group,
                tr("Review task branch…"),
            ));
        }
        commands.push(PaletteCommand::new(
            "github",
            "Issues",
            tr("GitHub PRs & Issues…"),
        ));
        commands.push(PaletteCommand::new(
            "linear",
            "Issues",
            tr("Linear issues…"),
        ));
        commands.push(PaletteCommand::new(
            "project:add",
            tr("Projects"),
            tr("Add workspace (Clone / Init / SSH)…"),
        ));
        commands.push(PaletteCommand::new(
            "notifications:clear",
            "Tessel",
            tr("Clear notifications"),
        ));

        // 6. Theme
        let theme_group = tr("Theme");
        for theme_id in &crate::theme_palettes::THEMES {
            let cmd = PaletteCommand::new(
                format!("theme:{}", theme_id.as_str()),
                theme_group,
                theme_id.label(french),
            );
            commands.push(if *theme_id == current_theme {
                cmd.with_hint(tr("Active"))
            } else {
                cmd
            });
        }

        // 7. Tessel / App
        commands.push(
            PaletteCommand::new(
                "broadcast",
                "Tessel",
                crate::layout_broadcast_ui::broadcast_command_title(french, self.state.broadcast),
            )
            .with_shortcut("Ctrl+Shift+B"),
        );
        commands
            .push(PaletteCommand::new("zoom:in", "Tessel", tr("Zoom in")).with_shortcut("Ctrl+="));
        commands.push(
            PaletteCommand::new("zoom:out", "Tessel", tr("Zoom out")).with_shortcut("Ctrl+-"),
        );
        commands.push(
            PaletteCommand::new("zoom:reset", "Tessel", tr("Reset text size"))
                .with_shortcut("Ctrl+0"),
        );
        commands.push(PaletteCommand::new("share", "Tessel", tr("Live Share…")));
        commands.push(PaletteCommand::new(
            "share:join",
            "Tessel",
            tr("Join Live Share session…"),
        ));
        #[cfg(windows)]
        commands.push(PaletteCommand::new("browser", "Tessel", tr("Browser")));
        #[cfg(windows)]
        commands.push(PaletteCommand::new("vault", "Tessel", tr("Passwords")));
        #[cfg(windows)]
        commands.push(PaletteCommand::new("account", "Tessel", tr("Account")));
        commands.push(
            PaletteCommand::new("settings", "Tessel", tr("Settings")).with_shortcut("Ctrl+,"),
        );
        let settings_lang = if french {
            crate::settings_model::SettingsLang::Fr
        } else {
            crate::settings_model::SettingsLang::En
        };
        for page in crate::settings_model::ALL_SETTINGS_PAGES {
            commands.push(PaletteCommand::new(
                format!("settings:{}", page.as_str()),
                tr("Settings"),
                page.title(settings_lang),
            ));
        }
        commands.push(
            PaletteCommand::new("help", "Tessel", tr("Keyboard shortcuts")).with_shortcut("F1"),
        );

        // 8. Quick commands, Go to workspace / Go to pane / Open editors
        let qc_group = tr("Quick commands");
        for qc in &self.state.quick_commands {
            let preview: String = qc
                .text
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(48)
                .collect();
            commands.push(
                PaletteCommand::new(format!("quickcmd:{}", qc.id), qc_group, &qc.name)
                    .with_hint(preview),
            );
        }
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
        // Secondary Git worktrees (b9061124): the sidebar's own Search button is gone, the
        // command box finds them.
        let worktree_group = tr("Worktrees");
        for project in &self.state.projects {
            let Some(summary) = self.worktree_state.summary(project.id) else {
                continue;
            };
            for wt in summary.worktrees.iter().filter(|w| !w.is_primary) {
                let title = if wt.branch.trim().is_empty() {
                    std::path::Path::new(&wt.path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| wt.path.clone())
                } else {
                    wt.branch.clone()
                };
                commands.push(
                    PaletteCommand::new(
                        format!("worktree:{}:{}", project.id, wt.path),
                        worktree_group,
                        title,
                    )
                    .with_hint(format!("{} · {}", project.name, wt.path)),
                );
            }
        }
        // Browser tabs.
        #[cfg(windows)]
        if self.browser_tab_open {
            let url = self.browser.active_url().unwrap_or_default().to_owned();
            commands.push(
                PaletteCommand::new("view:browser", tr("Browser tabs"), tr("Browser"))
                    .with_hint(url),
            );
        }

        commands
    }

    pub(crate) fn execute_palette_command(&mut self, ctx: &egui::Context, id: &str) {
        let active_pane_id = self.active_project_work_pane();
        let layout_key = crate::terminal_layout::project_key(self.state.active_project);
        match id {
            "new:default" => {
                let launch = self.default_new_launch();
                self.new_session(launch);
            }
            "terminal" => self.new_session(Launch::Shell),
            "floating" => self.toggle_floating(ctx),
            "launcher" => self.open_launcher(),
            "chat" => self.new_session(Launch::CodexChat),
            "claude-chat" => self.new_session(Launch::ClaudeChat),
            "project:new" => self.open_add_project_modal(),
            "project:add" => self.open_add_project_modal(),
            "github" => self.open_github_modal(self.state.active_project),
            "linear" => self.open_linear_modal(),
            "notifications:clear" => self.clear_notifications(),
            "worktree:new" => {
                if let Some(pid) = self.state.active_project {
                    self.start_new_worktree_pane(
                        pid,
                        "worktree",
                        crate::worktree_state::WorktreeLaunchTarget::ShellNewPane,
                    );
                }
            }
            "worktree:cleanup" => {
                if let Some(pid) = self.state.active_project {
                    self.open_worktree_cleanup(pid);
                }
            }
            "project:prev" => self.cycle_workspace(-1),
            "project:next" => self.cycle_workspace(1),
            "finder" => self.open_finder(),
            "dashboard" => self.state.panels.toggle_right(RightTab::Dashboard),
            "files" => self.state.panels.toggle_right(RightTab::Files),
            "changes" => self.state.panels.toggle_right(RightTab::Changes),
            "tasks" => self.state.panels.toggle_right(RightTab::Tasks),
            "task_history" => self.state.panels.toggle_right(RightTab::TaskHistory),
            "history" => {
                self.refresh_past_sessions();
                self.state.panels.toggle_right(RightTab::History);
            }
            "sessions:modal" => {
                self.refresh_past_sessions();
                self.side_panels_ui.sessions_modal_open = true;
            }
            "conversations" => self.state.panels.toggle_right(RightTab::Conversations),
            "notes" => self.open_notes_modal(),
            "review" => self.open_review_modal(),
            "task:new" => self.open_new_task(),
            "settings" => self.overlay = Some(Overlay::Settings),
            "help" => self.open_help(),
            "remote" => self.view = View::Remote,
            "share" => {
                self.share.tab = crate::share_panel::DialogTab::Share;
                self.overlay = Some(Overlay::Share);
            }
            "share:join" => {
                self.share.tab = crate::share_panel::DialogTab::Join;
                self.overlay = Some(Overlay::Share);
            }
            "broadcast" => {
                self.state.broadcast = !self.state.broadcast;
                self.save();
            }
            "zoom:in" => self.zoom_font(1),
            "zoom:out" => self.zoom_font(-1),
            "zoom:reset" => self.zoom_font(0),
            "layout:maximize" => {
                if let Some(active_id) = active_pane_id {
                    let layout = self.state.terminal_layouts.entry(layout_key).or_default();
                    layout.maximized = if layout.maximized == Some(active_id) {
                        None
                    } else {
                        Some(active_id)
                    };
                    self.grid_dirty = true;
                    self.save();
                }
            }
            "layout:restart-active" => {
                if let Some(active_id) = active_pane_id
                    && let Some(session) = self.sessions.iter_mut().find(|s| s.id == active_id)
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
            }
            "layout:reset-terminal" => {
                if let Some(active_id) = active_pane_id
                    && let Some(session) = self.sessions.iter_mut().find(|s| s.id == active_id)
                    && let Some(terminal) = &mut session.terminal
                {
                    terminal.reset_modes();
                }
            }
            "layout:toggle-left" => {
                self.state.panels.left_open = !self.state.panels.left_open;
            }
            "layout:toggle-right" => {
                self.state.panels.right_open = !self.state.panels.right_open;
            }
            "layout:toggle-status-bar" => {
                self.state.status_bar.visible = !self.state.status_bar.visible;
                self.save();
            }
            "layout:split-right" => {
                self.apply_layout_action(crate::layout_broadcast_ui::LayoutMenuAction::SplitRight);
            }
            "layout:split-down" => {
                self.apply_layout_action(crate::layout_broadcast_ui::LayoutMenuAction::SplitDown);
            }
            "layout:arrange" => {
                self.apply_layout_action(crate::layout_broadcast_ui::LayoutMenuAction::ArrangeGrid);
            }
            "layout:close-active" => {
                self.apply_layout_action(crate::layout_broadcast_ui::LayoutMenuAction::CloseActive);
            }
            id_str if id_str.starts_with("layout:preset:") => {
                if let Some(preset) = crate::pane_grid::GridPreset::ALL.into_iter().find(|p| {
                    format!(
                        "layout:preset:{}",
                        crate::layout_broadcast_ui::preset_display_label(*p)
                    ) == id_str
                }) {
                    self.apply_grid_preset(preset);
                }
            }
            id_str if id_str.starts_with("new:agent:") => {
                if let Some(agent_id) = id_str.strip_prefix("new:agent:")
                    && crate::agent_preferences::enabled(&self.state, agent_id)
                    && self
                        .agents
                        .entries()
                        .iter()
                        .any(|(known, installed)| *installed && known == agent_id)
                {
                    self.new_session(Launch::Agent(agent_id.into()));
                }
            }
            id_str if id_str.starts_with("settings:") => {
                if let Some(page_id) = id_str.strip_prefix("settings:") {
                    self.open_settings_by_page_id(page_id);
                }
            }
            id_str if id_str.starts_with("quickcmd:") => {
                if let Some(cmd_id) = id_str
                    .strip_prefix("quickcmd:")
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    self.run_quick_command_by_id(ctx, cmd_id);
                }
            }
            id_str if id_str.starts_with("theme:") => {
                if let Some(theme_id) = id_str
                    .strip_prefix("theme:")
                    .and_then(crate::theme_palettes::ThemeId::from_id)
                {
                    self.state.theme = Some(theme_id);
                    crate::theme::apply(ctx, self.state.dark, Some(theme_id));
                    self.save();
                }
            }
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
                } else if let Some((pid, path)) = id
                    .strip_prefix("worktree:")
                    .and_then(|rest| rest.split_once(':'))
                    .and_then(|(pid, path)| pid.parse::<u64>().ok().map(|pid| (pid, path)))
                {
                    self.open_worktree_from_palette(pid, path);
                } else if id == "view:browser" {
                    #[cfg(windows)]
                    {
                        self.browser_tab_open = true;
                        self.view = View::Browser;
                    }
                }
            }
        }
    }
    /// Command palette `worktree:<project>:<path>`: focus a pane already running in that
    /// worktree, else open a terminal there.
    fn open_worktree_from_palette(&mut self, project: u64, path: &str) {
        if !self.state.projects.iter().any(|p| p.id == project) {
            return;
        }
        if self.state.active_project != Some(project) {
            self.state.active_project = Some(project);
            self.refresh_project();
        }
        self.view = View::Work;
        if let Some(id) = self
            .sessions
            .iter()
            .find(|s| {
                s.project == Some(project)
                    && s.worktree.as_ref().is_some_and(|w| {
                        crate::sidebar_model::same_path(&w.path.to_string_lossy(), path)
                    })
            })
            .map(|s| s.id)
        {
            self.active_session = Some(id);
            self.save();
            return;
        }
        let branch = self
            .worktree_state
            .summary(project)
            .and_then(|s| {
                s.worktrees
                    .iter()
                    .find(|w| crate::sidebar_model::same_path(&w.path, path))
                    .map(|w| w.branch.clone())
            })
            .unwrap_or_default();
        self.new_session_in_worktree(
            Some(project),
            Launch::Shell,
            None,
            false,
            Some(workspace::SavedWorktree {
                path: PathBuf::from(path),
                branch,
                base_branch: None,
            }),
        );
    }

    pub(crate) fn open_finder(&mut self) {
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
            ProjectEntry, ProjectPanelInput, SessionEntry, SessionKind, SidebarOptions,
        };
        use crate::sidebar_model::{AgentDotState, PaneRuntimeKind, pane_dot_state};

        let waiting_chat_panes: std::collections::HashSet<u64> = self
            .chats
            .iter()
            .filter(|c| c.waiting_for_user())
            .map(|c| c.pane)
            .chain(
                self.claude_chats
                    .iter()
                    .filter(|c| c.waiting_for_user())
                    .map(|c| c.pane),
            )
            .collect();
        let working_chat_panes: std::collections::HashSet<u64> = self
            .chats
            .iter()
            .filter(|c| c.working())
            .map(|c| c.pane)
            .chain(
                self.claude_chats
                    .iter()
                    .filter(|c| c.working())
                    .map(|c| c.pane),
            )
            .collect();

        let is_sleeping = |session: &Session| -> bool {
            if let Some(provider) = session.launch.native_chat() {
                match provider {
                    "Codex" => !self.chats.iter().any(|c| c.pane == session.id && c.busy()),
                    "Claude" => !self
                        .claude_chats
                        .iter()
                        .any(|c| c.pane == session.id && c.busy()),
                    _ => true,
                }
            } else {
                session.terminal.is_none()
                    && session.loading.is_none()
                    && session.profile_migration.is_none()
                    && session.restarting.is_none()
            }
        };

        let mut session_states: std::collections::HashMap<u64, AgentDotState> =
            std::collections::HashMap::with_capacity(self.sessions.len());
        for s in &self.sessions {
            let sleeping = is_sleeping(s);
            let runtime_kind = match s.launch {
                Launch::Shell | Launch::Ssh(_) => PaneRuntimeKind::Shell,
                Launch::Agent(_) => PaneRuntimeKind::Agent,
                Launch::CodexChat | Launch::ClaudeChat => PaneRuntimeKind::Chat,
            };
            let raw_state = if waiting_chat_panes.contains(&s.id) {
                Some("approval")
            } else if working_chat_panes.contains(&s.id) {
                Some("working")
            } else if s.terminal.as_ref().is_some_and(Terminal::exited) {
                Some("stopped")
            } else if matches!(s.launch, Launch::Agent(_)) && !sleeping {
                Some("waiting")
            } else {
                None
            };
            session_states.insert(s.id, pane_dot_state(runtime_kind, sleeping, raw_state));
        }

        let entry = |session: &Session| SessionEntry {
            id: session.id,
            title: session.label().into(),
            kind: match session.launch {
                Launch::Shell | Launch::Ssh(_) => SessionKind::Shell,
                Launch::Agent(_) => SessionKind::Agent,
                Launch::CodexChat | Launch::ClaudeChat => SessionKind::Chat,
            },
            sleeping: is_sleeping(session),
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
        self.project_panel
            .set_live_share_projects(self.share.shared_projects_sidebar_map());
        self.project_panel.set_session_states(session_states);
        let mut branches = std::collections::HashMap::new();
        for p in &self.state.projects {
            if let Some(summary) = self.worktree_state.summary(p.id)
                && !summary.branch.is_empty()
            {
                branches.insert(p.id, summary.branch.clone());
            }
        }
        if let Some(active_id) = self.state.active_project
            && let Some(branch) = self.scm_state.branch_name()
        {
            branches.insert(active_id, branch.to_owned());
        }
        self.project_panel.set_project_branches(branches);

        let mut session_worktrees = std::collections::HashMap::new();
        for s in &self.sessions {
            if let Some(wt) = &s.worktree {
                session_worktrees.insert(s.id, (wt.path.display().to_string(), wt.branch.clone()));
            }
        }
        self.project_panel.set_session_worktrees(session_worktrees);

        let mut project_worktrees = std::collections::HashMap::new();
        for p in &self.state.projects {
            let p_str = p.path.display().to_string();
            let prefs = self
                .state
                .project_worktrees
                .get(&p.id)
                .cloned()
                .unwrap_or_default();
            let mut task_copies = Vec::new();
            for s in self.sessions.iter().filter(|s| s.project == Some(p.id)) {
                if let Some(wt) = &s.worktree {
                    let wt_str = wt.path.display().to_string();
                    if !crate::sidebar_model::same_path(&wt_str, &p_str)
                        && !task_copies
                            .iter()
                            .any(|c: &crate::sidebar_model::TaskCopyInput| {
                                crate::sidebar_model::same_path(&c.path, &wt_str)
                            })
                    {
                        task_copies.push(crate::sidebar_model::TaskCopyInput {
                            path: wt_str,
                            branch: wt.branch.clone(),
                            created_at: 0,
                        });
                    }
                }
            }
            let summary = self.worktree_state.summary(p.id);
            let discovered = summary.map(|s| s.worktrees.clone()).unwrap_or_default();
            let sidebar_proj = crate::sidebar_model::SidebarProjectInput {
                id: p.id.to_string(),
                name: p.name.clone(),
                cwd: p_str,
                task_copies: task_copies.clone(),
                shown_worktree_paths: prefs.shown_worktrees.clone(),
                worktrees_inbox_dismissed: prefs.inbox_dismissed,
                discovered_worktrees: discovered,
                ..Default::default()
            };
            let vis = crate::sidebar_model::project_worktree_visibility(&sidebar_proj);
            let mut visible_copies: Vec<crate::project_panel::ProjectWorktreeView> = task_copies
                .into_iter()
                .map(|c| {
                    let locked = summary
                        .and_then(|sum| {
                            sum.worktrees
                                .iter()
                                .find(|w| crate::sidebar_model::same_path(&w.path, &c.path))
                        })
                        .is_some_and(|w| w.locked);
                    crate::project_panel::ProjectWorktreeView {
                        path: c.path,
                        branch: c.branch,
                        is_external: false,
                        locked,
                    }
                })
                .collect();
            for wt in vis.shown {
                if !visible_copies
                    .iter()
                    .any(|c| crate::sidebar_model::same_path(&c.path, &wt.path))
                {
                    visible_copies.push(crate::project_panel::ProjectWorktreeView {
                        path: wt.path.clone(),
                        branch: wt.branch.clone(),
                        is_external: true,
                        locked: wt.locked,
                    });
                }
            }
            let hidden_inbox: Vec<(String, String)> = vis
                .inbox
                .into_iter()
                .map(|w| (w.path.clone(), w.branch.clone()))
                .collect();
            project_worktrees.insert(
                p.id,
                crate::project_panel::ProjectWorktreesInfo {
                    eligible: summary.is_some_and(|s| s.has_commits),
                    has_secondary: summary.is_some_and(|s| s.non_primary_count() > 0),
                    visible_copies,
                    hidden_inbox,
                    inbox_dismissed: prefs.inbox_dismissed,
                },
            );
        }
        self.project_panel.set_project_worktrees(project_worktrees);

        self.project_panel.set_options(SidebarOptions {
            sort_by_activity: self.state.sidebar.sort_by_activity,
            compact_rows: self.state.sidebar.compact_rows,
            hide_sleeping: self.state.sidebar.hide_sleeping,
            hide_agent_task_line: self.state.sidebar.hide_agent_task_line,
        });
        self.project_panel.set_project_groups(
            self.state
                .project_groups
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect(),
            self.state
                .collapsed_project_groups
                .iter()
                .cloned()
                .collect(),
        );

        self.project_panel
            .set_session_team_badges(self.teams_hub.session_badges(&self.state));

        let panel_disabled = self.native_modal_open()
            || self.side_panels_ui.any_modal_open()
            || self.dialogs_toolbar_ui.any_modal_open()
            || self.new_task.open
            || self.linear.view.open
            || self.teams_ui.manager_project.is_some()
            || !self.teams_hub.approval_prompts.is_empty()
            || self.share.has_pending_ask()
            || self.share.is_pill_popover_open()
            || self.overlay.is_some()
            || self.command_palette.open
            || self.finder.is_open()
            || self.toolbar_menu_open
            || self.pending_close.is_some()
            || self.pending_close_session.is_some()
            || self.pending_remove_project.is_some()
            || self.pending_scm_discard.is_some()
            || self.pending_agent_install.is_some()
            || self.pending_transcript_trash.is_some()
            || self.rename_project_modal.is_some()
            || self.worktree_cleanup.is_some()
            || self.pending_delete_worktree.is_some()
            || self.pending_exit
            || self.file_action.is_some()
            || self.restore_preview.is_some();

        let input = ProjectPanelInput {
            projects: &projects,
            global_sessions: &global_sessions,
            active_project: self.state.active_project,
            active_session: self.active_session,
            french: self.state.language == crate::i18n::Language::French,
        };

        let mut action = None;
        if self.state.panels.left_open {
            egui::SidePanel::left("projects")
                .default_width(280.0)
                .width_range(220.0..=500.0)
                .resizable(true)
                .show(ctx, |ui| {
                    if panel_disabled {
                        ui.disable();
                    }
                    action = self.project_panel.show(ui, input);
                });
        } else {
            egui::SidePanel::left("projects-rail")
                .exact_width(52.0)
                .resizable(false)
                .show(ctx, |ui| {
                    if panel_disabled {
                        ui.disable();
                    }
                    action = self.project_panel.show_collapsed_rail(ui, input);
                });
        }
        self.apply_project_action(ctx, action);
    }

    pub(crate) fn apply_project_action(
        &mut self,
        ctx: &egui::Context,
        action: Option<crate::project_panel::ProjectAction>,
    ) {
        use crate::project_panel::ProjectAction;
        let project = match action {
            Some(ProjectAction::Search) => {
                if !self.command_palette.open {
                    self.command_palette.toggle();
                }
                return;
            }
            Some(ProjectAction::OpenFolder) => {
                self.open_add_project_modal();
                return;
            }
            Some(ProjectAction::ExpandSidebar) => {
                self.state.panels.left_open = true;
                self.save();
                return;
            }
            Some(ProjectAction::NewTerminal(target_project)) => {
                if target_project.is_none()
                    || target_project
                        .is_some_and(|id| self.state.projects.iter().any(|p| p.id == id))
                {
                    if self.state.active_project != target_project {
                        self.state.active_project = target_project;
                        self.refresh_project();
                    }
                    self.new_session(Launch::Shell);
                }
                return;
            }
            Some(ProjectAction::NewWorktree(pid)) => {
                self.start_new_worktree_pane(
                    pid,
                    "worktree",
                    crate::worktree_state::WorktreeLaunchTarget::ShellNewPane,
                );
                return;
            }
            Some(ProjectAction::OpenWorktreeTerminal {
                project,
                path,
                branch,
            }) => {
                if self.state.projects.iter().any(|p| p.id == project) {
                    if self.state.active_project != Some(project) {
                        self.state.active_project = Some(project);
                        self.refresh_project();
                    }
                    self.new_session_in_worktree(
                        Some(project),
                        Launch::Shell,
                        None,
                        false,
                        Some(workspace::SavedWorktree {
                            path: PathBuf::from(path),
                            branch,
                            base_branch: None,
                        }),
                    );
                }
                return;
            }
            Some(ProjectAction::ShowHiddenWorktree { project, path }) => {
                let prefs = self.state.project_worktrees.entry(project).or_default();
                if !prefs
                    .shown_worktrees
                    .iter()
                    .any(|p| crate::sidebar_model::same_path(p, &path))
                {
                    prefs.shown_worktrees.push(path);
                    self.save();
                }
                return;
            }
            Some(ProjectAction::HideWorktree { project, path }) => {
                if let Some(prefs) = self.state.project_worktrees.get_mut(&project) {
                    prefs
                        .shown_worktrees
                        .retain(|p| !crate::sidebar_model::same_path(p, &path));
                    self.save();
                }
                return;
            }
            Some(ProjectAction::DismissWorktreesInbox(project)) => {
                self.state
                    .project_worktrees
                    .entry(project)
                    .or_default()
                    .inbox_dismissed = true;
                self.save();
                return;
            }
            Some(ProjectAction::DeleteWorktree {
                project,
                path,
                branch,
            }) => {
                let wt_path = PathBuf::from(path);
                if self.state.general_prefs.confirm_delete_workspace {
                    self.pending_delete_worktree = Some((project, wt_path, branch));
                } else if let Some(proj) = self
                    .state
                    .projects
                    .iter()
                    .find(|p| p.id == project)
                    .cloned()
                {
                    self.close_sessions_for_worktree_paths(project, std::slice::from_ref(&wt_path));
                    self.worktree_state.request_remove_batch(
                        project,
                        proj.path,
                        vec![(wt_path, branch, true)],
                    );
                }
                return;
            }
            Some(ProjectAction::CleanUpWorktrees(project)) => {
                self.open_worktree_cleanup(project);
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
            Some(ProjectAction::RenameProject(id)) => {
                if let Some(proj) = self.state.projects.iter().find(|p| p.id == id) {
                    self.rename_project_modal = Some((id, proj.name.clone(), false));
                }
                return;
            }
            Some(ProjectAction::RemoveProject(id)) => {
                if self.state.general_prefs.confirm_delete_workspace
                    && self.state.projects.iter().any(|p| p.id == id)
                {
                    self.pending_remove_project = Some(id);
                    return;
                }
                self.remove_project_immediate(id);
                return;
            }
            Some(ProjectAction::MoveProject { id, delta }) => {
                if let Some(idx) = self.state.projects.iter().position(|p| p.id == id) {
                    let target = (idx as isize + delta as isize)
                        .clamp(0, self.state.projects.len().saturating_sub(1) as isize)
                        as usize;
                    if target != idx {
                        self.state.projects.swap(idx, target);
                        self.save();
                    }
                }
                return;
            }
            Some(ProjectAction::RenameSession { id, .. }) => {
                if let Some(session) = self.sessions.iter().find(|s| s.id == id) {
                    let current = session.label().to_owned();
                    self.rename_agent = Some(RenameAgent {
                        pane: id,
                        expected: current.clone(),
                        draft: current,
                        error: String::new(),
                        focused: false,
                    });
                    self.overlay = Some(Overlay::RenameAgent);
                }
                return;
            }
            Some(ProjectAction::RestartSession { id, .. }) => {
                let prefs =
                    self.sessions
                        .iter()
                        .find(|s| s.id == id)
                        .and_then(|s| match &s.launch {
                            Launch::Agent(agent) => {
                                Some(crate::agent_preferences::for_pane_launch(
                                    &self.state,
                                    agent,
                                    self.model_discovery.as_ref().and_then(|d| d.models(agent)),
                                    s.session_options.as_ref(),
                                ))
                            }
                            _ => None,
                        });
                if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
                    session.restart(prefs);
                }
                return;
            }
            Some(ProjectAction::SleepSession { id, .. }) => {
                self.put_session_to_sleep(id);
                return;
            }
            Some(ProjectAction::SleepProject(pid)) => {
                let ids: Vec<u64> = self
                    .sessions
                    .iter()
                    .filter(|s| s.project == Some(pid))
                    .map(|s| s.id)
                    .collect();
                for sid in ids {
                    self.put_session_to_sleep(sid);
                }
                return;
            }
            Some(ProjectAction::EditProjectGroup(pid)) => {
                self.dialogs_toolbar_ui.group_modal_project = Some(pid);
                self.dialogs_toolbar_ui.group_modal_draft = self
                    .state
                    .project_groups
                    .get(&pid)
                    .cloned()
                    .unwrap_or_default();
                return;
            }
            Some(ProjectAction::ToggleProjectGroup(group)) => {
                if !self.state.collapsed_project_groups.remove(&group) {
                    self.state.collapsed_project_groups.insert(group);
                }
                self.save();
                return;
            }
            Some(ProjectAction::OpenGitHub(pid)) => {
                self.open_github_modal(Some(pid));
                return;
            }
            Some(ProjectAction::ManageTeams(pid)) => {
                self.teams_ui.open_manager(pid);
                return;
            }
            Some(ProjectAction::CloseSession { id, .. }) => {
                self.close_session(id);
                return;
            }
            Some(ProjectAction::UpdateOptions(opts)) => {
                self.state.sidebar.sort_by_activity = opts.sort_by_activity;
                self.state.sidebar.compact_rows = opts.compact_rows;
                self.state.sidebar.hide_sleeping = opts.hide_sleeping;
                self.state.sidebar.hide_agent_task_line = opts.hide_agent_task_line;
                self.save();
                return;
            }
            None => return,
        };
        if self.state.active_project != project {
            self.state.active_project = project;
            self.refresh_project();
        }
        self.save();
    }
    fn remove_project_immediate(&mut self, id: u64) {
        let Some(pos) = self.state.projects.iter().position(|p| p.id == id) else {
            return;
        };
        self.state.projects.remove(pos);
        self.state.project_worktrees.remove(&id);
        self.state.project_groups.remove(&id);
        self.worktree_state.retain_projects(&self.state.projects);
        self.state.tasks.retain(|t| t.project != id);
        let session_ids: Vec<u64> = self
            .sessions
            .iter()
            .filter(|s| s.project == Some(id))
            .map(|s| s.id)
            .collect();
        for sid in session_ids {
            self.remove_session(sid);
        }
        if self.state.active_project == Some(id) {
            self.state.active_project = self.state.projects.first().map(|p| p.id);
            self.refresh_project();
        }
        self.save();
    }
    fn explorer(&mut self, ui: &mut egui::Ui) {
        self.explorer_tree
            .set_show_dotfiles(self.state.editor_prefs.show_git_ignored);
        self.explorer_tree
            .set_compact_folders(self.state.editor_prefs.compact_folders);
        self.explorer_tree
            .set_git_badges(self.scm_state.explorer_badges());
        let mut close_doc = None;
        let mut select_doc = None;
        let mut pin_doc = None;
        egui::CollapsingHeader::new(tr("Open editors"))
            .default_open(true)
            .show(ui, |ui| {
                for (i, document) in self.documents.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let base_name = document.diff_title.clone().unwrap_or_else(|| {
                            document
                                .path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned()
                        });
                        let label_str =
                            format!("{base_name}{}", if document.dirty() { " •" } else { "" });
                        let mut rich = egui::RichText::new(label_str);
                        if document.unpinned_preview {
                            rich = rich.italics();
                        }
                        if document.deleted_on_disk {
                            rich = rich.strikethrough().color(ui.visuals().error_fg_color);
                        }
                        let resp = ui
                            .selectable_label(
                                self.view == View::Files && i == self.active_document,
                                rich,
                            )
                            .on_hover_text(document.path.display().to_string());
                        if resp.double_clicked() {
                            pin_doc = Some(i);
                            select_doc = Some(i);
                        } else if resp.clicked() {
                            select_doc = Some(i);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(egui::Button::new("×").frame(false).small())
                                .on_hover_text(tr("Close"))
                                .clicked()
                            {
                                close_doc = Some(i);
                            }
                        });
                    });
                }
            });
        if let Some(i) = pin_doc
            && let Some(doc) = self.documents.get_mut(i)
        {
            doc.unpinned_preview = false;
        }
        if let Some(i) = select_doc {
            self.active_document = i;
            self.view = View::Files;
        }
        if let Some(i) = close_doc {
            self.request_document_close(i);
        }
        ui.separator();
        let root = self.state.active().map(|p| p.path.clone());
        let active = (self.view == View::Files)
            .then(|| self.documents.get(self.active_document))
            .flatten()
            .map(|d| d.path.clone());
        let actions_enabled = self.file_actions_ready();
        let timeline = self.timeline_snapshot(true);
        egui::TopBottomPanel::bottom("files-timeline-section")
            .resizable(false)
            .show_inside(ui, |ui| {
                if let Some(action) =
                    crate::file_timeline::compat(ui, &timeline, crate::i18n::is_french())
                {
                    if let crate::file_timeline::Action::OpenWeb(url) = action {
                        self.open_web_link(ui.ctx(), url);
                    } else {
                        self.timeline_action(action);
                    }
                }
            });
        let old_prefs = self.state.editor_prefs;
        egui::TopBottomPanel::bottom("files-outline-section")
            .resizable(false)
            .show_inside(ui, |ui| {
                egui::CollapsingHeader::new(if crate::i18n::is_french() {
                    "Plan"
                } else {
                    "Outline"
                })
                .default_open(false)
                .show(ui, |ui| {
                    ui.menu_button("⋯", |ui| {
                        let label = if crate::i18n::is_french() {
                            "Suivre le curseur"
                        } else {
                            "Follow Cursor"
                        };
                        if ui
                            .checkbox(&mut self.state.editor_prefs.outline_follow_cursor, label)
                            .clicked()
                        {
                            ui.close();
                        }
                        let label = if crate::i18n::is_french() {
                            "Filtrer à la frappe"
                        } else {
                            "Filter on Type"
                        };
                        if ui
                            .checkbox(&mut self.state.editor_prefs.outline_filter_on_type, label)
                            .clicked()
                        {
                            ui.close();
                        }
                        for (mode, en, fr) in crate::json_outline_panel::SORT_OPTIONS {
                            let label = if crate::i18n::is_french() { fr } else { en };
                            if ui
                                .selectable_value(
                                    &mut self.state.editor_prefs.outline_sort,
                                    mode,
                                    label,
                                )
                                .clicked()
                            {
                                ui.close();
                            }
                        }
                    });
                    let Some(doc) = self.documents.get_mut(self.active_document).filter(|doc| {
                        self.view == View::Files && crate::json_outline_panel::supported(doc)
                    }) else {
                        ui.label(if crate::i18n::is_french() {
                            "L’éditeur actif ne fournit pas de plan."
                        } else {
                            "The active editor cannot provide outline information."
                        });
                        return;
                    };
                    crate::json_outline_panel::show(
                        ui,
                        doc,
                        self.state.editor_prefs.outline_sort,
                        self.state.editor_prefs.outline_follow_cursor,
                        self.state.editor_prefs.outline_filter_on_type,
                    );
                });
            });
        if old_prefs != self.state.editor_prefs {
            self.save();
        }
        let can_insert = self.explorer_terminal_id().is_some();
        let tree_action = self.explorer_tree.show(
            ui,
            root.as_deref(),
            active.as_deref(),
            actions_enabled,
            can_insert,
        );
        let new_dotfiles = self.explorer_tree.show_dotfiles();
        let new_compact = self.explorer_tree.compact_folders();
        if self.state.editor_prefs.show_git_ignored != new_dotfiles
            || self.state.editor_prefs.compact_folders != new_compact
        {
            self.state.editor_prefs.show_git_ignored = new_dotfiles;
            self.state.editor_prefs.compact_folders = new_compact;
            self.save();
        }
        if let Some(action) = tree_action {
            use crate::explorer_tree::{TreeAction, TreeContextKind};
            match action {
                TreeAction::OpenFile(path) if !self.file_action_pending => {
                    self.open_document_preview(path)
                }
                TreeAction::OpenPinnedFile(path) if !self.file_action_pending => {
                    self.open_document(path)
                }
                TreeAction::OpenContentHit { path, line } if !self.file_action_pending => {
                    self.open_explorer_content_hit(path, line)
                }
                #[cfg(windows)]
                TreeAction::Reveal { root: owner, path } if root.as_ref() == Some(&owner) => {
                    self.reveal_project_item(owner, path)
                }
                TreeAction::Download { root: owner, path } if root.as_ref() == Some(&owner) => {
                    self.download_project_item(owner, path)
                }
                TreeAction::InsertPath { root: owner, path } => {
                    self.insert_explorer_path(&owner, &path, Some(ui.ctx()))
                }
                TreeAction::ExternalEditor { root, path } => {
                    self.open_external_project_file(root, path)
                }
                TreeAction::TerminalHere { root: owner, path } => {
                    self.open_project_terminal_here(&owner, &path)
                }
                TreeAction::CopyPath(path) => {
                    ui.ctx().copy_text(path.display().to_string());
                }
                TreeAction::Context {
                    kind: TreeContextKind::Trash,
                    root: action_root,
                    path,
                    name,
                } if actions_enabled && root.as_ref() == Some(&action_root) => {
                    self.file_action_pending = true;
                    let sender = self.sender.clone();
                    let trash_action = crate::file_actions::Action {
                        kind: crate::file_actions::Kind::Trash,
                        root: action_root,
                        path,
                        name,
                    };
                    thread::spawn(move || {
                        let _ = sender.send(Message::FileAction(crate::file_actions::execute(
                            trash_action,
                        )));
                    });
                }
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
                        TreeContextKind::Trash => crate::file_actions::Kind::Trash,
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
    fn open_requested_remote_terminal(&mut self) {
        if let Some(host) = self.remote.terminal_request.take() {
            let split = self
                .active_session
                .filter(|id| {
                    self.sessions
                        .iter()
                        .any(|s| s.id == *id && s.project == self.state.active_project)
                })
                .map(|id| (id, crate::pane_grid::SplitDir::Col));
            self.new_session_placed(Launch::Ssh(host), split);
        }
    }

    fn explorer_terminal_id(&self) -> Option<u64> {
        if self.view != View::Work || self.floating.ui.is_open() {
            return None;
        }
        let id = self.active_session?;
        self.sessions
            .iter()
            .find(|s| {
                s.id == id
                    && s.project == self.state.active_project
                    && s.terminal
                        .as_ref()
                        .is_some_and(|t| t.accepts_broadcast_input())
            })
            .map(|s| s.id)
    }

    fn insert_explorer_path(&mut self, root: &Path, path: &Path, ctx: Option<&egui::Context>) {
        if self.state.active().is_none_or(|p| p.path != root) {
            return;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            return;
        };
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return;
        }
        let Some(raw) = relative
            .to_str()
            .filter(|s| !s.chars().any(char::is_control))
        else {
            return;
        };
        let text = if raw
            .chars()
            .any(|c| c.is_whitespace() || "&()^;,'\"".contains(c))
        {
            format!("\"{raw}\"")
        } else {
            raw.to_owned()
        };
        let Some(id) = self.explorer_terminal_id() else {
            return;
        };
        if let Some(term) = self
            .sessions
            .iter_mut()
            .find(|s| s.id == id)
            .and_then(|s| s.terminal.as_mut())
        {
            term.paste_text(&text);
            if let Some(ctx) = ctx {
                term.request_focus(ctx);
            }
            self.view = View::Work;
        }
    }

    fn open_active_file_externally(&mut self, id: u64) {
        let Some(doc) = self.documents.get(self.active_document).filter(|doc| {
            self.view == View::Files && doc.instance_id() == id && doc.external_available()
        }) else {
            return;
        };
        self.launch_external_editor(
            doc.root.clone(),
            doc.path.clone(),
            doc.external_position(),
            true,
        );
    }

    fn open_external_project_file(&mut self, root: PathBuf, path: PathBuf) {
        self.launch_external_editor(root, path, None, false);
    }

    fn launch_external_editor(
        &mut self,
        root: PathBuf,
        path: PathBuf,
        position: Option<crate::files::FilePosition>,
        from_editor: bool,
    ) {
        if self.state.active().is_none_or(|p| p.path != root) {
            return;
        }
        let sender = self.sender.clone();
        if let Err(error) = thread::Builder::new()
            .name("external-editor".into())
            .spawn(move || {
                let result =
                    crate::external_editor::open(&root, &path, position).and_then(|fallback| {
                        if from_editor && fallback.is_some() {
                            Err("VS Code is unavailable for this file.".into())
                        } else {
                            Ok(fallback)
                        }
                    });
                let _ = sender.send(Message::ExternalOpen(root, result));
            })
        {
            self.error = error.to_string();
        }
    }
    fn download_project_item(&mut self, root: PathBuf, path: PathBuf) {
        if self.file_action_pending
            || self
                .state
                .active()
                .is_none_or(|project| project.path != root)
        {
            return;
        }
        let src = match crate::local_download::source(&root, &path) {
            Ok(src) => src,
            Err(error) => {
                self.error = error;
                return;
            }
        };
        let folder = src.is_dir();
        let dest = if folder {
            rfd::FileDialog::new()
                .set_title(tr("Download the folder into"))
                .pick_folder()
        } else {
            let name = crate::download_name::suggest(
                &path.file_name().unwrap_or_default().to_string_lossy(),
            );
            crate::download_name::choose_download_destination(name.as_ref())
        };
        let Some(dest) = dest else { return };
        self.file_action_pending = true;
        self.explorer_tree.download_notice = Some((root.clone(), tr("Copying…").into()));
        let sender = self.sender.clone();
        if let Err(error) = thread::Builder::new()
            .name("explorer-download".into())
            .spawn(move || {
                let mut last = std::time::Instant::now();
                let result = crate::local_download::copy(&root, &path, &dest, folder, |n| {
                    if last.elapsed() >= std::time::Duration::from_millis(200) {
                        let _ = sender.send(Message::DownloadProgress(root.clone(), n));
                        last = std::time::Instant::now();
                    }
                });
                let _ = sender.send(Message::DownloadFinished(root, result));
            })
        {
            self.file_action_pending = false;
            self.explorer_tree.download_notice = None;
            self.error = error.to_string();
        }
    }

    fn open_project_terminal_here(&mut self, root: &Path, path: &Path) {
        let Some(project) = self
            .state
            .active()
            .filter(|project| project.path == root)
            .map(|project| project.id)
        else {
            return;
        };
        let target = match crate::files::confined(root, path) {
            Ok(path) => path,
            Err(error) => {
                self.error = error;
                return;
            }
        };
        let cwd = if target.is_dir() {
            target
        } else {
            let Some(parent) = target.parent() else {
                return;
            };
            parent.to_owned()
        };
        let split = self
            .active_session
            .filter(|id| {
                self.sessions
                    .iter()
                    .any(|s| s.id == *id && s.project == Some(project))
            })
            .map(|id| (id, crate::pane_grid::SplitDir::Col));
        self.new_session_at(
            Some(project),
            Launch::Shell,
            split,
            false,
            None,
            SessionStart::Directory(dunce::simplified(&cwd).to_owned()),
        );
    }

    #[cfg(windows)]
    fn reveal_project_item(&mut self, root: PathBuf, path: PathBuf) {
        let sender = self.sender.clone();
        if let Err(error) = thread::Builder::new()
            .name("explorer-reveal".into())
            .spawn(move || {
                if let Err(error) = crate::native_reveal::reveal(&root, &path) {
                    let _ = sender.send(Message::RevealError(error));
                }
            })
        {
            self.error = error.to_string();
        }
    }
    fn right_panel(&mut self, ctx: &egui::Context) {
        if !self.state.panels.right_open {
            self.right_fullscreen = false;
            return;
        }
        if self.state.active().is_some()
            && matches!(
                self.state.panels.right_tab,
                RightTab::Changes | RightTab::Files
            )
            && self.scm_state.result.is_none()
            && !self.scm_state.busy()
            && !self.git_result.busy()
        {
            self.sync_git_project();
            self.scm_state.refresh();
        }
        let avail_w = ctx.available_rect().width().max(260.0);
        let max_width = (avail_w - 420.0).max(260.0);
        let panel = if self.right_fullscreen {
            egui::SidePanel::right("tools-panel")
                .exact_width(avail_w)
                .resizable(false)
        } else {
            egui::SidePanel::right("tools-panel")
                .default_width(420.0)
                .width_range(260.0..=max_width)
        };
        let changes_badge = self.scm_state.badge_count();
        panel.show(ctx, |ui| {
            if self.native_modal_open()
                || self.side_panels_ui.any_modal_open()
                || self.pending_scm_discard.is_some()
                || self.rename_project_modal.is_some()
            {
                ui.disable();
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (tab, label, icon, shortcut, badge) in [
                    (
                        RightTab::Dashboard,
                        "Dashboard",
                        crate::icons::Icon::LayoutDashboard,
                        "",
                        0,
                    ),
                    (
                        RightTab::Files,
                        "Files",
                        crate::icons::Icon::Files,
                        "Ctrl+Shift+X",
                        0,
                    ),
                    (
                        RightTab::Changes,
                        "Changes",
                        crate::icons::Icon::GitBranch,
                        "Ctrl+Shift+G",
                        changes_badge,
                    ),
                    (
                        RightTab::Tasks,
                        "Tasks",
                        crate::icons::Icon::ListChecks,
                        "Ctrl+Shift+K",
                        0,
                    ),
                    (
                        RightTab::TaskHistory,
                        "Task history",
                        crate::icons::Icon::ReceiptText,
                        "",
                        0,
                    ),
                    (
                        RightTab::History,
                        "Past conversations",
                        crate::icons::Icon::History,
                        "",
                        0,
                    ),
                    (
                        RightTab::Conversations,
                        "Conversations",
                        crate::icons::Icon::MessagesSquare,
                        "",
                        0,
                    ),
                ] {
                    let tab_label = if shortcut.is_empty() {
                        tr(label).to_owned()
                    } else {
                        format!("{} ({shortcut})", tr(label))
                    };
                    let selected = self.state.panels.right_tab == tab;
                    let clicked = if badge > 0 {
                        crate::chrome::side_tab_with_badge(ui, icon, &tab_label, selected, badge)
                            .clicked()
                    } else {
                        crate::chrome::side_tab(ui, icon, &tab_label, selected).clicked()
                    };
                    if clicked {
                        if tab == RightTab::History
                            && self.past_sessions_cache.is_empty()
                            && !self.gpui_extra.session_discovery.requested()
                        {
                            self.refresh_past_sessions();
                        }
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
                        self.right_fullscreen = false;
                        self.state.panels.right_open = false;
                    }
                    let (fs_icon, fs_tip) = if self.right_fullscreen {
                        (crate::icons::Icon::Minimize2, tr("Restore panel width"))
                    } else {
                        (crate::icons::Icon::Maximize2, tr("Expand panel"))
                    };
                    if crate::pane_header::button(ui, fs_icon, fs_tip).clicked() {
                        self.right_fullscreen = !self.right_fullscreen;
                    }
                    if self.state.active_project.is_some()
                        && crate::pane_header::button(
                            ui,
                            crate::icons::Icon::NotebookPen,
                            tr("Project notes…"),
                        )
                        .clicked()
                    {
                        self.open_notes_modal();
                    }
                });
            });
            ui.separator();
            match self.state.panels.right_tab {
                RightTab::Dashboard => self.dashboard_view(ui),
                RightTab::Files => self.explorer(ui),
                RightTab::Changes => self.git_view(ui),
                RightTab::Tasks => self.tasks(ui),
                RightTab::TaskHistory => self.task_history_view(ui),
                RightTab::History => self.session_history_view(ui),
                RightTab::Conversations => self.conversations_view(ui),
            }
        });
    }
    fn remember_active_work_pane(&mut self) {
        if let Some(pane) = self
            .active_session
            .and_then(|id| self.sessions.iter().find(|pane| pane.id == id))
        {
            let key = crate::terminal_layout::project_key(pane.project);
            self.state.terminal_layouts.entry(key).or_default().focused = Some(pane.id);
        }
    }

    fn active_project_work_pane(&self) -> Option<u64> {
        self.active_session
            .filter(|id| {
                self.sessions
                    .iter()
                    .any(|s| s.id == *id && s.project == self.state.active_project)
            })
            .or_else(|| {
                let project = self.state.active_project;
                let key = crate::terminal_layout::project_key(project);
                let id = self.state.terminal_layouts.get(&key)?.focused?;
                self.sessions
                    .iter()
                    .any(|s| s.id == id && s.project == project)
                    .then_some(id)
            })
            .or_else(|| {
                self.sessions
                    .iter()
                    .find(|s| s.project == self.state.active_project)
                    .map(|s| s.id)
            })
    }
    fn split_shell_beside(&mut self, target: u64, dir: crate::pane_grid::SplitDir) {
        let Some(session) = self
            .sessions
            .iter()
            .find(|s| s.id == target && s.project == self.state.active_project)
        else {
            return;
        };
        let worktree = session.worktree.clone();
        self.new_session_in_worktree(
            self.state.active_project,
            Launch::Shell,
            Some((target, dir)),
            false,
            worktree,
        );
    }

    fn tab_can_start_grid(&self) -> bool {
        match self.view {
            View::Files => self.documents.get(self.active_document).is_some(),
            #[cfg(windows)]
            View::Browser => true,
            _ => false,
        }
    }

    fn can_split_shell(&self, has_pane: bool) -> bool {
        !matches!(self.view, View::GuestFiles | View::Remote)
            && (has_pane || self.tab_can_start_grid())
    }

    fn split_active_shell(&mut self, dir: crate::pane_grid::SplitDir) {
        if matches!(self.view, View::GuestFiles | View::Remote) {
            return;
        }
        if let Some(target) = self.active_project_work_pane() {
            self.split_shell_beside(target, dir);
        } else if self.tab_can_start_grid() {
            self.new_session_placed(Launch::Shell, None);
        }
    }

    fn apply_layout_action(&mut self, action: crate::layout_broadcast_ui::LayoutMenuAction) {
        use crate::layout_broadcast_ui::LayoutMenuAction;
        match action {
            LayoutMenuAction::SplitRight => {
                self.split_active_shell(crate::pane_grid::SplitDir::Row)
            }
            LayoutMenuAction::SplitDown => self.split_active_shell(crate::pane_grid::SplitDir::Col),
            LayoutMenuAction::ArrangeGrid => {
                self.view = View::Work;
                self.tile_sessions(None);
            }
            LayoutMenuAction::ApplyPreset(preset) => {
                self.apply_grid_preset(preset);
            }
            LayoutMenuAction::CloseActive => {
                if let Some(id) = self.active_project_work_pane() {
                    self.close_session(id);
                }
            }
        }
    }
    fn apply_grid_preset(&mut self, preset: crate::pane_grid::GridPreset) {
        let session_ids: Vec<u64> = self
            .sessions
            .iter()
            .filter(|s| s.project == self.state.active_project)
            .map(|s| s.id)
            .collect();
        let key = crate::terminal_layout::project_key(self.state.active_project);
        let existing_ids: Vec<u64> = {
            let layout = self.state.terminal_layouts.entry(key.clone()).or_default();
            layout.sync(&session_ids, self.grid_room);
            crate::pane_grid::tree_leaves(layout.tree.as_ref(), |_| true)
                .into_iter()
                .filter_map(|leaf| leaf.id.parse::<u64>().ok())
                .filter(|id| session_ids.contains(id))
                .collect()
        };
        let available_slots = 100usize.saturating_sub(self.sessions.len());
        let plan =
            crate::layout_broadcast_ui::plan_preset_grid(preset, &existing_ids, available_slots);
        let cwd = self
            .state
            .active()
            .map(|p| p.path.clone())
            .unwrap_or_else(launch::default_directory);
        let mut spawned_ids = Vec::with_capacity(plan.shells_to_spawn);
        for _ in 0..plan.shells_to_spawn {
            if self.sessions.len() >= 100 {
                self.error = "Session limit: 100".into();
                break;
            }
            let id = self.next_session;
            let Some(next) = self.next_session.checked_add(1) else {
                self.error = "Session identifiers exhausted".into();
                break;
            };
            self.next_session = next;
            let mut s = Session {
                profile_data_dir: None,
                profile_root: None,
                profile_migration: None,
                resume_required: false,
                ssh_retry: Default::default(),
                ssh_tmux_id: None,
                native_host: None,
                restarting: None,
                codex_discovery: None,
                codex_restore: None,
                model_picker: Default::default(),
                last_model: None,
                model_save_attempt: None,
                settings_changed: false,
                codex_seed: None,
                conversation_id: None,
                profile_binding: None,
                launch_fingerprint: None,
                session_options: None,
                worktree: None,
                pane_name: None,
                broadcast: true,
                id,
                project: self.state.active_project,
                title: "Terminal".into(),
                launch: Launch::Shell,
                cwd: cwd.clone(),
                scrollback: self.state.terminal_prefs.scrollback,
                terminal: None,
                loading: None,
                error: String::new(),
            };
            s.start(None);
            self.sessions.push(s);
            spawned_ids.push(id);
        }
        let fin = plan.finalize(&spawned_ids, self.active_session);
        if fin.ordered_ids.is_empty() {
            return;
        }
        self.state
            .terminal_layouts
            .entry(key)
            .or_default()
            .rebuild_ordered(&fin.ordered_ids, &fin.row_counts);
        self.active_session = fin.active_id;
        self.view = View::Work;
        self.grid_dirty = false;
        self.save();
    }
    fn tile_sessions(&mut self, columns: Option<usize>) {
        let key = crate::terminal_layout::project_key(self.state.active_project);
        if let Some(layout) = self.state.terminal_layouts.get_mut(&key) {
            layout.tile(self.grid_room, columns);
            self.save();
        }
    }
    fn launch_empty_state_entry(&mut self, id: &str) {
        match id {
            "terminal" => {
                self.new_session(Launch::Shell);
            }
            "folder" => self.choose_project(),
            "chat:codex" if self.state.active_project.is_some() => {
                self.new_session(Launch::CodexChat);
            }
            "chat:claude" if self.state.active_project.is_some() => {
                self.new_session(Launch::ClaudeChat);
            }
            #[cfg(windows)]
            "browser" => {
                self.browser_tab_open = true;
                self.view = View::Browser;
            }
            other => {
                if let Some(name) = other.strip_prefix("agent:")
                    && self
                        .agents
                        .entries()
                        .iter()
                        .any(|(known, installed)| *installed && known == name)
                {
                    self.new_session(Launch::Agent(name.into()));
                }
            }
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
            use crate::empty_state_panel::{EmptyStateAction, Entry, ShellOption};
            let agents_group = tr("Agents").to_owned();
            let other_group = tr("Terminals & pages").to_owned();
            let mut entries = Vec::new();
            for (name, installed) in self.agents.entries() {
                if *installed && crate::agent_preferences::enabled(&self.state, name) {
                    entries.push(Entry {
                        id: format!("agent:{name}"),
                        label: crate::agent_preferences::display_name(&self.state, name).into(),
                        group: agents_group.clone(),
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
                        group: agents_group.clone(),
                        enabled: true,
                    });
                }
            }
            entries.push(Entry {
                id: "terminal".into(),
                label: tr("Terminal").into(),
                group: other_group.clone(),
                enabled: true,
            });
            #[cfg(windows)]
            entries.push(Entry {
                id: "browser".into(),
                label: tr("Browser page").into(),
                group: other_group.clone(),
                enabled: true,
            });
            entries.push(Entry {
                id: "folder".into(),
                label: tr("Open folder").into(),
                group: other_group,
                enabled: true,
            });
            let title = self
                .state
                .active()
                .map(|p| tr("What do you want to start in {{name}}?").replace("{{name}}", &p.name))
                .unwrap_or_default();
            let subtitle = self
                .state
                .active()
                .map(|p| p.path.display().to_string())
                .unwrap_or_default();
            let mut shells = Vec::new();
            if cfg!(windows) {
                shells.push(ShellOption {
                    id: "powershell",
                    label: "PowerShell",
                });
                shells.push(ShellOption {
                    id: "cmd",
                    label: "CMD",
                });
                if self.wsl_available {
                    shells.push(ShellOption {
                        id: "wsl",
                        label: "WSL",
                    });
                }
            }
            let selected_shell = self
                .state
                .launcher_prefs
                .preferred_shell
                .as_deref()
                .unwrap_or("powershell")
                .to_owned();
            let action = self.empty_state.show_full(
                ui,
                &title,
                &subtitle,
                &entries,
                &shells,
                &selected_shell,
                self.remember_next_project_launch,
            );
            match action {
                Some(EmptyStateAction::Launch(id)) => {
                    if self.remember_next_project_launch && id != "folder" {
                        self.state.launcher_prefs.remember_default = Some(id.clone());
                        self.save();
                    }
                    self.launch_empty_state_entry(&id);
                }
                Some(EmptyStateAction::SelectShell(shell_id)) => {
                    self.state.launcher_prefs.preferred_shell = Some(shell_id);
                    self.save();
                }
                Some(EmptyStateAction::SetRememberDefault(checked)) => {
                    self.remember_next_project_launch = checked;
                    if !checked && self.state.launcher_prefs.remember_default.take().is_some() {
                        self.save();
                    }
                }
                None => {}
            }
            return;
        }
        if !self.active_session.is_some_and(|id| ids.contains(&id)) {
            self.active_session = self.active_project_work_pane();
        }
        let key = crate::terminal_layout::project_key(self.state.active_project);
        let room = ui.available_rect_before_wrap();
        self.grid_room = [room.width() as f64, room.height() as f64];
        let layout = self.state.terminal_layouts.entry(key.clone()).or_default();
        self.grid_dirty |= layout.sync_with_tiling(
            &ids,
            self.grid_room,
            self.state.terminal_prefs.tile_new_panes,
        );
        let maximized = layout.maximized;
        let geometry = pane_grid::grid_layout(layout.tree.as_ref());
        if maximized.is_none()
            && ui.is_enabled()
            && !self.floating.ui.has_keyboard()
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
                    self.grid_dirty = true;
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
        // One identity/profile path for both native renderers; never connects on restore.
        self.ensure_native_chat_instances();
        let mut rename = None;
        let mut close = None;
        let mut split = None;
        let mut maximize = None;
        let mut arrange = false;
        let mut model_choice = None;
        let mut send_selection: Option<(String, String)> = None;
        let mut open_here: Option<u64> = None;
        let mut switch_chat_mode: Option<u64> = None;
        let mut pending_tessel_browser_url: Option<String> = None;
        let mut observed_updates = Vec::new();
        let mut native_options_changed = false;
        #[cfg(windows)]
        let mut native_wake = None;
        let french = self.state.language == crate::i18n::Language::French;
        let spec = crate::theme::active(ui.ctx()).spec();
        let mut broadcast_membership_changed = false;
        let mut broadcast_batches: Vec<(u64, Vec<crate::terminal::BroadcastInput>)> = Vec::new();
        let picker_was_open = self.sessions.iter().any(|s| s.model_picker.is_open());
        let workspace_targets: Vec<crate::pane_menu_model::OtherPaneTarget> = self
            .sessions
            .iter()
            .filter(|s| s.project == self.state.active_project)
            .map(|s| crate::pane_menu_model::OtherPaneTarget {
                id: s.id.to_string(),
                title: s.label().to_owned(),
                where_hint: String::new(),
                is_agent: matches!(
                    s.launch,
                    Launch::Agent(_) | Launch::CodexChat | Launch::ClaudeChat
                ),
            })
            .collect();
        let waiting_chat_panes: std::collections::HashSet<u64> = self
            .chats
            .iter()
            .filter(|c| c.waiting_for_user())
            .map(|c| c.pane)
            .chain(
                self.claude_chats
                    .iter()
                    .filter(|c| c.waiting_for_user())
                    .map(|c| c.pane),
            )
            .collect();
        let working_chat_panes: std::collections::HashSet<u64> = self
            .chats
            .iter()
            .filter(|c| c.working())
            .map(|c| c.pane)
            .chain(
                self.claude_chats
                    .iter()
                    .filter(|c| c.working())
                    .map(|c| c.pane),
            )
            .collect();
        let mut team_membership_changed = false;
        let mut chat_citation = None;
        let mut deferred_teams_action: Option<crate::teams_ui::TeamsUiAction> = None;
        let ctx_menu_was_just_opened = std::mem::take(&mut self.pane_context_menu_just_opened);
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
                self.grid_dirty |= self.active_session != Some(id);
                self.active_session = Some(id);
            }
            let active = self.active_session == Some(id);
            let color = if active {
                ui.visuals().selection.stroke.color
            } else {
                ui.visuals().widgets.noninteractive.bg_stroke.color
            };
            let color = crate::layout_broadcast_ui::pane_active_stroke_color(
                &spec.ui,
                active,
                self.state.broadcast,
                session.broadcast,
                session.launch.native_chat().is_none(),
                color,
                color,
            );
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
                                let is_agent_pane = matches!(
                                    session.launch,
                                    Launch::Agent(_) | Launch::CodexChat | Launch::ClaudeChat
                                );
                                let chat_view_open = session.launch.native_chat().is_some();
                                let allowed_native_cwd = session
                                    .project
                                    .and_then(|pid| self.state.projects.iter().find(|p| p.id == pid))
                                    .is_some_and(|p| {
                                        p.path == session.cwd
                                            || session
                                                .worktree
                                                .as_ref()
                                                .is_some_and(|wt| wt.path == session.cwd)
                                    });
                                let can_switch_chat_mode = session.native_host.is_none()
                                    && session.loading.is_none()
                                    && session.profile_migration.is_none()
                                    && session.restarting.is_none()
                                    && allowed_native_cwd
                                    && (chat_view_open
                                        || matches!(
                                            &session.launch,
                                            Launch::Agent(a) if a == "claude" || a == "codex"
                                        ));
                                let id_str_self = id.to_string();
                                let peer_targets: Vec<_> = workspace_targets
                                    .iter()
                                    .filter(|t| t.id != id_str_self)
                                    .cloned()
                                    .collect();
                                let menu = crate::pane_header::MenuState {
                                    model_choice: session.has_model_choice(),
                                    model_pending: !session.can_pick_model(),
                                    model_manual: session.session_options.is_some(),
                                    can_switch_chat_mode,
                                    chat_view_open,
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
                                    can_rename: true,
                                    is_agent: is_agent_pane,
                                    sleeping: session.terminal.is_none()
                                        && session.loading.is_none()
                                    && session.profile_migration.is_none()
                                        && session.restarting.is_none()
                                        && session.launch.native_chat().is_none(),
                                    exited: session
                                        .terminal
                                        .as_ref()
                                        .is_some_and(Terminal::exited),
                                    asks_approval: waiting_chat_panes.contains(&id),
                                    activity: if working_chat_panes.contains(&id) {
                                        Some(crate::pane_menu_model::PaneAgentActivity::Busy)
                                    } else {
                                        None
                                    },
                                    has_selection: session
                                        .terminal
                                        .as_ref()
                                        .is_some_and(Terminal::has_selection),
                                    voice_typing: cfg!(windows),
                                    other_panes: peer_targets,
                                    language: if self.state.language
                                        == crate::i18n::Language::French
                                    {
                                        crate::pane_menu_model::PaneMenuLang::Fr
                                    } else {
                                        crate::pane_menu_model::PaneMenuLang::En
                                    },
                                    worktree: session.worktree.as_ref().map(|wt| {
                                        (wt.branch.clone(), wt.path.display().to_string())
                                    }),
                                    team: self
                                        .state
                                        .teams
                                        .iter()
                                        .find(|t| t.members.contains(&id))
                                        .map(|t| crate::pane_menu_model::TeamMembership {
                                            id: t.id.clone(),
                                            name: t.name.clone(),
                                            is_lead: t.lead_pane == Some(id),
                                            can_set_lead: is_agent_pane,
                                        }),
                                    ..Default::default()
                                };
                                let mut action = None;
                                ui.spacing_mut().item_spacing.x = 4.0;
                                let chip_width = if observed_model.is_some() && pane_header_width > crate::session_option_picker_ui::HEADER_MODEL_CHIP_HIDE_MAX_WIDTH {
                                    let text = crate::pane_session_model::model_text(observed_model.as_ref(), self.state.language == crate::i18n::Language::French);
                                    (ui.painter().layout_no_wrap(text, egui::FontId::proportional(10.5), ui.visuals().text_color()).size().x + 12.0).ceil().clamp(24.0, 140.0) + 4.0
                                } else { 0.0 };
                                let native_tag = session.terminal.as_ref().and_then(Terminal::native_header_tag)
                                    .filter(|tag| ui.available_width() > 164.0 + crate::host_terminal_panel::header_tag_width(ui, *tag, crate::i18n::is_french()));
                                let native_tag_width = native_tag.map(|tag| crate::host_terminal_panel::header_tag_width(ui, tag, crate::i18n::is_french()) + 4.0).unwrap_or(0.0);
                                let write_toggle_width = if self.state.broadcast
                                    && session.launch.native_chat().is_none()
                                {
                                    crate::layout_broadcast_ui::pane_broadcast_toggle_width(french)
                                        + 4.0
                                } else {
                                    0.0
                                };
                                let title_width = (ui.available_width() - 84.0 - chip_width - native_tag_width - write_toggle_width).max(0.0);
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
                                if let Some(team_badge) =
                                    self.teams_hub.session_badges(&self.state).get(&id)
                                    && crate::teams_ui::paint_pane_team_pills(
                                        ui,
                                        &spec.ui,
                                        team_badge,
                                        french,
                                    )
                                    && let Some(pid) = session.project
                                {
                                    self.teams_ui.open_manager(pid);
                                }
                                if let Some(tag) = native_tag { crate::host_terminal_panel::show_header_tag(ui, tag, crate::i18n::is_french()); }
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
                                if self.state.broadcast
                                    && session.launch.native_chat().is_none()
                                    && crate::layout_broadcast_ui::show_pane_broadcast_toggle(
                                        ui,
                                        &spec.ui,
                                        ("pane-broadcast-write", id),
                                        french,
                                        &mut session.broadcast,
                                    )
                                    .changed()
                                {
                                    broadcast_membership_changed = true;
                                }
                                if self.share.hosting() {
                                    let presence = self.share.pane_presence(id);
                                    crate::share_panel::SharePanel::paint_pane_presence(
                                        ui, &presence, french,
                                    );
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
                                });
                                if let Some((ctx_id, ctx_pos)) = self.pane_context_menu
                                    && ctx_id == id
                                {
                                    let area_resp = egui::Area::new(egui::Id::new((
                                        "pane-shift-ctx-menu",
                                        id,
                                    )))
                                    .order(egui::Order::Foreground)
                                    .fixed_pos(ctx_pos)
                                    .constrain(true)
                                    .show(ui.ctx(), |ui| {
                                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                                            ui.set_min_width(210.0);
                                            if let Some(selected) =
                                                crate::pane_header::show(ui, &menu)
                                            {
                                                action = Some(selected);
                                                self.pane_context_menu = None;
                                            }
                                        });
                                    });
                                    let escape = ui
                                        .ctx()
                                        .input(|i| i.key_pressed(egui::Key::Escape));
                                    let clicked_outside = !ctx_menu_was_just_opened
                                        && ui.ctx().input(|i| {
                                            i.pointer.any_pressed()
                                                && i.pointer.interact_pos().is_some_and(|pos| {
                                                    !area_resp.response.rect.contains(pos)
                                                })
                                        });
                                    if escape || clicked_outside {
                                        self.pane_context_menu = None;
                                    }
                                }
                                if let Some(action) = action {
                                    use crate::pane_menu_model::PaneMenuAction;
                                    match action {
                                        PaneMenuAction::ChooseModel if session.can_pick_model() => {
                                            session.model_picker.open_at_point(ui.ctx(), more.rect.left_bottom(), Some(more.id));
                                        }
                                        PaneMenuAction::Restart
                                        | PaneMenuAction::RestartApplySettings => {
                                            session.restart(pane_prefs.clone());
                                        }
                                        PaneMenuAction::CopySessionId => {
                                            if let Some(id) = &conversation_id {
                                                ui.ctx().copy_text(id.clone());
                                            }
                                        }
                                        PaneMenuAction::Copy => {
                                            if let Some(terminal) = &mut session.terminal {
                                                let _ = terminal.copy_selection(ui.ctx());
                                            }
                                        }
                                        PaneMenuAction::Paste => {
                                            if let Some(terminal) = &mut session.terminal {
                                                terminal.paste_from_clipboard();
                                            }
                                        }
                                        PaneMenuAction::CopyOutput => {
                                            if let Some(terminal) = &session.terminal {
                                                ui.ctx().copy_text(terminal.output());
                                            }
                                        }
                                        PaneMenuAction::Clear => {
                                            if let Some(terminal) = &mut session.terminal {
                                                terminal.clear_screen();
                                            }
                                        }
                                        PaneMenuAction::ResetTerminal => {
                                            if let Some(terminal) = &mut session.terminal {
                                                terminal.reset_modes();
                                            }
                                        }
                                        PaneMenuAction::Find => {
                                            if let Some(terminal) = &mut session.terminal {
                                                terminal.open_find();
                                            }
                                        }
                                        PaneMenuAction::VoiceTyping => {
                                            crate::terminal::trigger_voice_typing();
                                        }
                                        PaneMenuAction::SendSelectionTo(target_id) => {
                                            if let Some(text) = session
                                                .terminal
                                                .as_mut()
                                                .and_then(Terminal::take_selection)
                                            {
                                                send_selection = Some((target_id, text));
                                            }
                                        }
                                        PaneMenuAction::OpenOrSwitchChat => {
                                            switch_chat_mode = Some(id);
                                        }
                                        PaneMenuAction::OpenHere => {
                                            open_here = Some(id);
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
                                        PaneMenuAction::Rename => {
                                            rename = Some((id, session.label().to_owned()))
                                        }
                                        PaneMenuAction::ToggleTeamLead(tid) => {
                                            team_membership_changed |=
                                                crate::teams_relay::TeamsRelayHub::toggle_team_lead(
                                                    &mut self.state,
                                                    &tid,
                                                    id,
                                                );
                                        }
                                        PaneMenuAction::LeaveTeam(tid) => {
                                            team_membership_changed |=
                                                crate::teams_relay::TeamsRelayHub::leave_team(
                                                    &mut self.state,
                                                    &tid,
                                                    id,
                                                );
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
                                                    if let Some(path) = chat.show(ui) { chat_citation = Some((id, path)); }
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
                                                    if let Some(path) = chat.show(ui) { chat_citation = Some((id, path)); }
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
                                if let Some(banner) = self.teams_hub.wake_banners.get(&id).cloned()
                                    && let Some(wake_act) =
                                        crate::teams_ui::show_unconfirmed_wake_banner(
                                            ui, &spec.ui, &banner, french,
                                        )
                                {
                                    deferred_teams_action = Some(wake_act);
                                }
                                let agent_kind = match &session.launch {
                                    Launch::Agent(agent) => Some(agent.as_str()),
                                    _ => None,
                                };
                                terminal.set_agent_kind(agent_kind);
                                ui.add_enabled_ui(!picker_was_open && !session.model_picker.is_open(), |ui| terminal.show_with_prefs(ui, self.state.font_size, &self.state.terminal_prefs, None));
                                if let Some(pos) = terminal.take_context_menu_request() {
                                    self.pane_context_menu = Some((id, pos));
                                    self.pane_context_menu_just_opened = true;
                                }
                                if let Some(url) = terminal.take_pending_tessel_url() {
                                    pending_tessel_browser_url = Some(url);
                                }
                                let batch = terminal.take_broadcast_inputs();
                                if self.state.broadcast
                                    && session.broadcast
                                    && session.launch.native_chat().is_none()
                                    && !batch.is_empty()
                                {
                                    broadcast_batches.push((id, batch));
                                }
                                let exited_code = terminal.exited().then(|| terminal.exit_code()).flatten();
                                let is_exited = terminal.exited();
                                if is_exited {
                                    let (msg, restart_lbl, close_lbl) =
                                        session.exit_banner(exited_code, french);
                                    let mut do_restart = false;
                                    let mut do_close = false;
                                    let wrap_actions =
                                        crate::terminal::exit_overlay_actions_wrap_together(
                                            ui.available_width() - 28.0,
                                            170.0,
                                            170.0,
                                            10.0,
                                        );
                                    ui.add_space(6.0);
                                    ui.vertical_centered(|ui| {
                                        egui::Frame::new()
                                            .fill(spec.ui.surface_2)
                                            .stroke(egui::Stroke::new(1.0_f32, spec.ui.border))
                                            .corner_radius(egui::CornerRadius::same(8))
                                            .inner_margin(egui::Margin::symmetric(12, 8))
                                            .show(ui, |ui| {
                                                if wrap_actions {
                                                    ui.vertical(|ui| {
                                                        ui.label(&msg);
                                                        ui.horizontal(|ui| {
                                                            if ui.button(restart_lbl).clicked() {
                                                                do_restart = true;
                                                            }
                                                            if ui.button(close_lbl).clicked() {
                                                                do_close = true;
                                                            }
                                                        });
                                                    });
                                                } else {
                                                    ui.horizontal(|ui| {
                                                        ui.label(&msg);
                                                        ui.add_space(8.0);
                                                        ui.horizontal(|ui| {
                                                            if ui.button(restart_lbl).clicked() {
                                                                do_restart = true;
                                                            }
                                                            if ui.button(close_lbl).clicked() {
                                                                do_close = true;
                                                            }
                                                        });
                                                    });
                                                }
                                            });
                                    });
                                    if do_restart {
                                        let prefs = match &session.launch {
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
                                        session.retry_terminal(prefs, self.state.launcher_prefs.preferred_shell.clone());
                                    }
                                    if do_close {
                                        close = Some(id);
                                    }
                                }
                            } else if session.restarting.is_some() {
                                ui.spinner();
                                ui.label(tr("Stopping terminal processes…"));
                            } else if session.codex_restore.is_some() {
                                ui.spinner();
                                ui.label(tr("Finding the previous conversation…"));
                            } else if session.loading.is_some() || session.profile_migration.is_some() {
                                ui.spinner();
                                ui.label(tr("Starting…"));
                            } else if !session.error.is_empty() {
                                let (msg, retry_lbl, close_lbl) =
                                    crate::terminal::terminal_exit_banner_copy(true, None, french);
                                let mut wake = false;
                                let mut fresh = false;
                                let mut do_close = false;
                                let wrap_actions =
                                    crate::terminal::exit_overlay_actions_wrap_together(
                                        ui.available_width() - 28.0,
                                        170.0,
                                        170.0,
                                        10.0,
                                    );
                                ui.add_space(6.0);
                                ui.vertical_centered(|ui| {
                                    egui::Frame::new()
                                        .fill(spec.ui.surface_2)
                                        .stroke(egui::Stroke::new(1.0_f32, spec.ui.border))
                                        .corner_radius(egui::CornerRadius::same(8))
                                        .inner_margin(egui::Margin::symmetric(12, 8))
                                        .show(ui, |ui| {
                                            if wrap_actions {
                                                ui.vertical(|ui| {
                                                    ui.label(&msg);
                                                    ui.horizontal(|ui| {
                                                        if ui.button(retry_lbl).clicked() {
                                                            wake = true;
                                                        }
                                                        if session.codex_seed.is_some()
                                                            && ui
                                                                .button(tr(
                                                                    "Start a new conversation",
                                                                ))
                                                                .clicked()
                                                        {
                                                            fresh = true;
                                                        }
                                                        if ui.button(close_lbl).clicked() {
                                                            do_close = true;
                                                        }
                                                    });
                                                });
                                            } else {
                                                ui.horizontal(|ui| {
                                                    ui.label(&msg);
                                                    ui.add_space(8.0);
                                                    ui.horizontal(|ui| {
                                                        if ui.button(retry_lbl).clicked() {
                                                            wake = true;
                                                        }
                                                        if session.codex_seed.is_some()
                                                            && ui
                                                                .button(tr(
                                                                    "Start a new conversation",
                                                                ))
                                                                .clicked()
                                                        {
                                                            fresh = true;
                                                        }
                                                        if ui.button(close_lbl).clicked() {
                                                            do_close = true;
                                                        }
                                                    });
                                                });
                                            }
                                        });
                                });
                                if fresh {
                                    session.codex_seed = None;
                                }
                                if do_close {
                                    close = Some(id);
                                } else if wake || fresh {
                                    session.error.clear();
                                    #[cfg(windows)]
                                    if session.native_host.is_some() {
                                        native_wake = Some(session.id);
                                    }
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
                            } else {
                                let wake = ui.button(tr("Wake / reconnect")).clicked();
                                if wake {
                                    #[cfg(windows)]
                                    if session.native_host.is_some() {
                                        native_wake = Some(session.id);
                                    }
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
        if team_membership_changed {
            self.save();
        }
        if let Some((pane, path)) = chat_citation {
            self.open_chat_citation(pane, &path);
        }
        if let Some(t_act) = deferred_teams_action {
            self.handle_teams_ui_action(t_act);
        }
        #[cfg(windows)]
        if let Some(id) = native_wake {
            self.wake_native_terminal(id, ui.ctx());
        }
        if let Some(target_pane) = open_here {
            self.active_session = Some(target_pane);
            self.open_launcher();
        }
        if let Some(pane_id) = switch_chat_mode {
            self.switch_session_chat_mode(pane_id);
        }
        if let Some(url) = pending_tessel_browser_url {
            self.open_url_in_tessel_browser(ui.ctx(), url);
        }
        if let Some((target_id_str, text)) = send_selection
            && let Ok(target_id) = target_id_str.parse::<u64>()
            && let Some(peer) = self.sessions.iter_mut().find(|s| s.id == target_id)
        {
            if let Some(terminal) = &mut peer.terminal {
                terminal.paste_text(&text);
                terminal.request_focus(ui.ctx());
                self.active_session = Some(target_id);
            } else {
                match peer.launch {
                    Launch::CodexChat => {
                        if let Some(chat) = self.chats.iter_mut().find(|c| c.pane == target_id) {
                            chat.append_composer(&text);
                            self.active_session = Some(target_id);
                        }
                    }
                    Launch::ClaudeChat => {
                        if let Some(chat) =
                            self.claude_chats.iter_mut().find(|c| c.pane == target_id)
                        {
                            chat.append_composer(&text);
                            self.active_session = Some(target_id);
                        }
                    }
                    _ => {}
                }
            }
        }
        if self.state.broadcast && !broadcast_batches.is_empty() {
            for (source_id, batch) in &broadcast_batches {
                for peer in &mut self.sessions {
                    if peer.project == self.state.active_project
                        && peer.id != *source_id
                        && peer.broadcast
                        && peer.launch.native_chat().is_none()
                        && let Some(target_terminal) = &mut peer.terminal
                    {
                        for item in batch {
                            target_terminal.send_broadcast(item);
                        }
                    }
                }
            }
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
        if native_options_changed || broadcast_membership_changed {
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
            self.split_shell_beside(placement.0, placement.1);
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
        let readable = d.diff_title.is_none()
            && matches!(
                extension.as_str(),
                "html"
                    | "htm"
                    | "md"
                    | "markdown"
                    | "csv"
                    | "tsv"
                    | "json"
                    | "jsonc"
                    | "jsonl"
                    | "ndjson"
            );
        let (close_clicked, prefs_changed, open_external) = document_toolbar(
            ui,
            d,
            readable,
            &mut self.state.editor_prefs,
            self.store.as_ref().map(|s| s.directory()),
        );
        if close_clicked {
            close = Some(self.active_document);
        }
        let mut save_prefs = prefs_changed;
        if !d.error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &d.error);
        }
        let mut link = None;
        if d.diff_title.is_some() {
            show_diff_document(
                ui,
                d,
                f32::from(self.state.editor_prefs.font_size),
                self.state.editor_prefs.diff_word_wrap,
            );
        } else if let Some(source) = &d.playback {
            source.show(ui);
        } else if let Some(office) = &mut d.office {
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
            let french = self.state.language == crate::i18n::Language::French;
            if !d.tools.image_download_notice.is_empty() {
                ui.label(&d.tools.image_download_notice);
            }
            if let Some(texture) = &d.texture {
                crate::files::show_fitted_image(ui, texture).context_menu(|ui| {
                    if ui
                        .button(if french {
                            "Télécharger…"
                        } else {
                            "Download…"
                        })
                        .clicked()
                    {
                        ui.close();
                        d.choose_image_copy(french);
                    }
                });
            }
        } else if readable && d.preview_mode {
            if matches!(extension.as_str(), "csv" | "tsv") {
                let explicit = (extension == "tsv").then_some(b'\t');
                let text_ref = &d.text;
                let sheet = d.sheet_cache.get_or_insert_with(|| {
                    std::sync::Arc::new(crate::rich_previews::parse_delimited_sheet(
                        text_ref, explicit,
                    ))
                });
                crate::rich_previews::show_sheet_grid(ui, &d.path, sheet);
            } else if matches!(extension.as_str(), "json" | "jsonc" | "jsonl" | "ndjson") {
                crate::rich_previews::show_json_preview(ui, &d.path, &d.text, &extension);
            } else if matches!(extension.as_str(), "md" | "markdown") {
                match crate::rich_previews::classify_markdown_size(d.text.len()) {
                    crate::rich_previews::MarkdownSizeDecision::SourceOnly { byte_len } => {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            format!(
                                "Markdown file exceeds 1 MiB ({} KiB) — opening in Source view.",
                                byte_len / 1024
                            ),
                        );
                        if ui.button(tr("Source")).clicked() {
                            d.preview_mode = false;
                        }
                    }
                    crate::rich_previews::MarkdownSizeDecision::PromptUser { byte_len }
                        if !d.force_large_markdown_render =>
                    {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            format!(
                                "Large Markdown document ({} KiB). Rendering may take a moment.",
                                byte_len / 1024
                            ),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Render anyway").clicked() {
                                d.force_large_markdown_render = true;
                            }
                            if ui.button("Show source").clicked() {
                                d.preview_mode = false;
                            }
                        });
                    }
                    _ => {
                        ui.small(tr(
                            "Native readable view · scripts, CSS and external resources are not loaded",
                        ));
                        let segments = crate::rich_previews::extract_markdown_segments(&d.text);
                        let has_mermaid = segments.iter().any(|s| {
                            matches!(s, crate::rich_previews::MarkdownSegment::Mermaid(_))
                        });
                        if !has_mermaid {
                            let preview = d.preview.get_or_insert_with(|| {
                                crate::preview::Preview::parse(&d.text, true)
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
                            egui::ScrollArea::vertical()
                                .id_salt((&d.path, "preview-mermaid"))
                                .show(ui, |ui| {
                                    for seg in &segments {
                                        match seg {
                                            crate::rich_previews::MarkdownSegment::Prose(p) => {
                                                let parsed =
                                                    crate::preview::Preview::parse(p, true);
                                                if let Some(clicked) = parsed.show(ui) {
                                                    link = Some(clicked);
                                                }
                                            }
                                            crate::rich_previews::MarkdownSegment::Mermaid(m) => {
                                                crate::rich_previews::show_mermaid_block(ui, m);
                                                ui.add_space(8.0_f32);
                                            }
                                        }
                                    }
                                });
                        }
                    }
                }
            } else {
                ui.small(tr(
                    "Native readable view · scripts, CSS and external resources are not loaded",
                ));
                let preview = d
                    .preview
                    .get_or_insert_with(|| crate::preview::Preview::parse(&d.text, false));
                if ui.button(tr("Copy plain text")).clicked() {
                    ui.ctx().copy_text(preview.plain());
                }
                egui::ScrollArea::vertical()
                    .id_salt((&d.path, "preview"))
                    .show(ui, |ui| {
                        link = preview.show(ui);
                    });
            }
        } else {
            show_editor_bars(ui, d);
            let font = egui::FontId::monospace(f32::from(self.state.editor_prefs.font_size));
            let row_height = ui.fonts_mut(|f| f.row_height(&font)).max(14.0_f32);
            let lang = crate::syntax::SyntaxLang::from_path(&d.path);
            let theme_id = crate::theme::active(ui.ctx());
            let spec = theme_id.spec();
            let dark = self.state.dark;
            let syntax_enabled = self.state.editor_prefs.syntax_highlighting;
            let show_line_numbers = self.state.editor_prefs.line_numbers;
            let changed_flags = d
                .original_text()
                .map(|orig| crate::syntax::changed_line_flags(orig, &d.text))
                .unwrap_or_default();
            let has_changes = changed_flags.iter().any(|&c| c);
            let line_count = 1 + d.text.as_bytes().iter().filter(|&&b| b == b'\n').count();
            let word_wrap = self.state.editor_prefs.word_wrap;
            let mut scroll = if word_wrap {
                egui::ScrollArea::vertical()
            } else {
                egui::ScrollArea::both()
            }
            .id_salt((&d.path, "source"));
            if let Some(target_line) = d.tools.pending_scroll_line.take() {
                scroll = scroll.vertical_scroll_offset(
                    ((target_line as f32) * row_height - 60.0_f32).max(0.0_f32),
                );
            }
            scroll.show(ui, |ui| {
                let gutter_w = if show_line_numbers || has_changes {
                    crate::syntax::editor_gutter_width(ui, &font, line_count)
                } else {
                    0.0_f32
                };
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0_f32;
                    let gutter_bounds = if gutter_w > 0.0_f32 {
                        let (r, _) = ui.allocate_exact_size(
                            egui::vec2(gutter_w, 18.0_f32),
                            egui::Sense::hover(),
                        );
                        Some((r.min.x, r.max.x))
                    } else {
                        None
                    };
                    let wrap_width = if word_wrap {
                        ui.available_width().max(80.0_f32)
                    } else {
                        f32::INFINITY
                    };
                    let Document {
                        text,
                        tools,
                        syntax_cache,
                        preview,
                        sheet_cache,
                        unpinned_preview,
                        last_edited_at,
                        ..
                    } = d;
                    let mut layouter =
                        |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _wrap_width: f32| {
                            syntax_cache.layout(
                                ui,
                                buf.as_str(),
                                lang,
                                theme_id,
                                dark,
                                &font,
                                wrap_width,
                                word_wrap,
                                syntax_enabled,
                            )
                        };
                    let mut out = egui::TextEdit::multiline(text)
                        .code_editor()
                        .font(font.clone())
                        .desired_width(f32::INFINITY)
                        .desired_rows(30)
                        .lock_focus(true)
                        .layouter(&mut layouter)
                        .show(ui);
                    if let Some((gx_min, gx_max)) = gutter_bounds {
                        let gutter_font = egui::FontId::monospace(
                            (f32::from(self.state.editor_prefs.font_size) - 1.0).max(6.0),
                        );
                        let _ = crate::syntax::paint_editor_gutter(
                            ui.painter(),
                            gx_min,
                            gx_max,
                            out.galley_pos,
                            &out.galley,
                            &gutter_font,
                            &changed_flags,
                            show_line_numbers,
                            spec.ui.text_dim,
                            spec.ui.border,
                            spec.ui.accent,
                        );
                    }
                    if let Some(offset) = tools
                        .pending_char_range
                        .map(|(_, end)| end)
                        .or_else(|| out.cursor_range.map(|r| r.primary.index))
                    {
                        let next = Some(crate::files::OutlineCursor::Chars(offset));
                        if tools.outline_cursor != next {
                            tools.outline_cursor = next;
                            ui.ctx().request_repaint();
                        }
                    }
                    if let Some((sc, ec)) = tools.pending_char_range.take() {
                        let range = egui::text::CCursorRange::two(
                            egui::text::CCursor::new(sc),
                            egui::text::CCursor::new(ec),
                        );
                        out.state.cursor.set_char_range(Some(range));
                        out.state.store(ui.ctx(), out.response.id);
                        if tools
                            .outline_reveal_focus
                            .take()
                            .unwrap_or(!tools.find_open && !tools.goto_open)
                        {
                            out.response.request_focus();
                        }
                    }
                    if out.response.changed() {
                        *preview = None;
                        *sheet_cache = None;
                        *unpinned_preview = false;
                        *last_edited_at = Some(std::time::Instant::now());
                        syntax_cache.invalidate();
                    }
                });
            });
        }
        let resolved = link.map(|href| {
            crate::preview::resolve(&d.root, &d.path, &href).map(|link| (d.root.clone(), link))
        });
        let external_id = open_external.then(|| d.instance_id());
        if let Some(id) = external_id {
            self.open_active_file_externally(id);
        }
        let _ = &mut save_prefs;
        if save_prefs {
            self.save();
        }
        if let Some(result) = resolved {
            match result {
                Ok((root, crate::preview::Link::File(path))) => {
                    self.open_linked_document(root, path)
                }
                Ok((_, crate::preview::Link::Web(url))) => self.open_web_link(ui.ctx(), url),
                Err(error) => self.error = error,
            }
        }
        if let Some(i) = close {
            self.request_document_close(i);
        }
    }
    fn open_url_in_tessel_browser(&mut self, ctx: &egui::Context, url: String) {
        #[cfg(windows)]
        {
            let _ = ctx;
            self.overlay = None;
            self.show_agent_browser = false;
            self.browser_tab_open = true;
            self.browser.open_address(url);
            self.view = View::Browser;
        }
        #[cfg(not(windows))]
        {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }

    fn prepare_native_profile(
        &mut self,
        pane: u64,
        agent: &str,
        saved: Option<&str>,
    ) -> Result<Option<crate::session_profile::NativeLaunch>, String> {
        let mut prefs = self
            .state
            .agent_prefs
            .get(agent)
            .cloned()
            .unwrap_or_default();
        let data = self
            .store
            .as_ref()
            .map(|store| store.directory().to_owned());
        let session = self
            .sessions
            .iter_mut()
            .find(|s| s.id == pane)
            .ok_or("Native pane is unavailable.")?;
        if session.profile_migration.is_some() {
            return Ok(None);
        }
        if !session.error.is_empty() {
            return Err(session.error.clone());
        }
        let result: Result<Option<crate::session_profile::NativeLaunch>, String> = (|| {
            if session.profile_binding.is_none()
                && let Some(saved) = saved
            {
                let home =
                    directories::BaseDirs::new().ok_or("Cannot locate the local home folder.")?;
                session.profile_migration =
                    Some(crate::session_profile_migration::Migration::start(
                        agent.into(),
                        saved.into(),
                        session.cwd.clone(),
                        data.clone(),
                        home.home_dir().to_owned(),
                        prefs,
                        None,
                        None,
                    )?);
                return Ok(None);
            }
            if session.profile_binding.is_none()
                && let Some(data) = data.as_deref()
                && let Some(line) =
                    crate::settings_extra_model::active_managed_env_line(data, agent)
                && crate::agent_env::parse(&prefs.env).is_ok_and(|env| {
                    crate::agent_env::effective_value(
                        &env,
                        crate::session_profile::env_key(agent).expect("native provider"),
                    )
                    .is_none()
                })
            {
                if !prefs.env.is_empty() {
                    prefs.env.push('\n');
                }
                prefs.env.push_str(&line);
            }
            session.profile_data_dir = data.clone();
            session.bind_profile(agent, &prefs)?;
            Ok(Some(crate::session_profile::NativeLaunch::new(
                agent,
                session.profile_binding.clone().expect("bound account"),
                session.profile_root.clone().expect("resolved account"),
                data,
            )?))
        })();
        if let Err(error) = &result {
            session.error = error.clone();
        }
        result
    }

    fn switch_session_chat_mode(&mut self, pane_id: u64) {
        if self.chats.iter().any(|c| c.pane == pane_id && c.starting())
            || self
                .claude_chats
                .iter()
                .any(|c| c.pane == pane_id && c.starting())
        {
            self.error = "Wait for native chat startup to finish before switching modes.".into();
            return;
        }
        if self
            .sessions
            .iter()
            .any(|s| s.id == pane_id && (s.loading.is_some() || s.restarting.is_some()))
        {
            self.error =
                "Wait for the terminal operation to finish before switching chat mode.".into();
            return;
        }
        self.ensure_native_chat_instances();
        if self.sessions.iter().any(|session| {
            session.id == pane_id
                && (session.profile_binding.is_none() || session.profile_migration.is_some())
        }) {
            self.error = "Identify the original account in the current mode before switching; the session was kept.".into();
            return;
        }

        // Do not expose a second transport until the old CLI has acknowledged exit.
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == pane_id)
            && let Some(mut terminal) = session.terminal.take()
        {
            let (sender, result) = mpsc::channel();
            session.restarting = Some(Restart {
                preferred_shell: None,
                switch_mode: true,
                result,
                preferences: None,
            });
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
            return;
        }

        // Capture any active native-chat thread ID before tearing down its runtime view:
        let live_chat_thread = self
            .chats
            .iter()
            .find(|c| c.pane == pane_id)
            .and_then(|c| c.thread_id.clone())
            .or_else(|| {
                self.claude_chats
                    .iter()
                    .find(|c| c.pane == pane_id)
                    .and_then(|c| c.saved_session.clone())
            });

        // Flush live `self.sessions` into `self.state.sessions` before running the transition:
        self.save();
        match self
            .state
            .switch_saved_session_chat_mode(pane_id, live_chat_thread.as_deref())
        {
            Ok(true) => {
                let Some(saved) = self
                    .state
                    .sessions
                    .iter()
                    .find(|s| s.id == pane_id)
                    .cloned()
                else {
                    return;
                };
                self.chats.retain(|c| c.pane != pane_id);
                self.claude_chats.retain(|c| c.pane != pane_id);
                if let Some(session) = self.sessions.iter_mut().find(|s| s.id == pane_id) {
                    session.loading = None;
                    session.restarting = None;
                    session.codex_discovery = None;
                    session.codex_restore = None;
                    session.codex_seed = saved.codex_seed;
                    session.last_model = saved.last_model;
                    session.launch_fingerprint = saved.launch_fingerprint;
                    session.conversation_id = saved.conversation_id;
                    session.profile_binding = saved.profile_binding;
                    session.profile_root = None;
                    session.resume_required = session.conversation_id.is_some();
                    session.session_options = if saved.launch.native_chat().is_some() {
                        saved.native_options
                    } else {
                        saved.session_options
                    };
                    session.launch = saved.launch.clone();
                    session.title = saved.title;
                    session.settings_changed = false;
                    session.error.clear();

                    if let Launch::Agent(agent) = &saved.launch {
                        let prefs = crate::agent_preferences::for_pane_launch(
                            &self.state,
                            agent,
                            self.model_discovery.as_ref().and_then(|d| d.models(agent)),
                            session.session_options.as_ref(),
                        );
                        let preferred_shell = self.state.launcher_prefs.preferred_shell.clone();
                        session.start_with_shell(Some(prefs), preferred_shell);
                    }
                }
                self.save();
            }
            Ok(false) => {}
            Err(err) => self.error = err,
        }
    }

    fn open_web_link(&mut self, ctx: &egui::Context, url: String) {
        if self.state.browser_prefs.open_links_in == workspace::OpenLinksTarget::System {
            ctx.open_url(egui::OpenUrl::new_tab(url));
            return;
        }
        #[cfg(windows)]
        {
            self.overlay = None;
            self.show_agent_browser = false;
            self.browser.open_address(url);
            self.view = View::Browser;
        }
        #[cfg(not(windows))]
        {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }
    fn open_embedded_notice(&mut self, name: &'static str) {
        let (title, content, markdown) = match name {
            "THIRD_PARTY_NOTICES.md" => (
                "THIRD_PARTY_NOTICES.md".to_owned(),
                include_str!("../THIRD_PARTY_NOTICES.md").to_owned(),
                true,
            ),
            _ => (
                "LICENSE".to_owned(),
                include_str!("../LICENSE").to_owned(),
                false,
            ),
        };
        let vpath = PathBuf::from(format!("tessel-notice://{title}"));
        if let Some(idx) = self.documents.iter().position(|d| d.path == vpath) {
            self.active_document = idx;
            self.overlay = None;
            self.view = View::Files;
            return;
        }
        if self.documents.len() >= 24 {
            if let Some(idx) = self
                .documents
                .iter()
                .position(|d| d.unpinned_preview && !d.dirty())
            {
                self.documents[idx] =
                    Document::from_readonly_text(PathBuf::new(), vpath, title, content, markdown);
                self.active_document = idx;
                self.overlay = None;
                self.view = View::Files;
            } else {
                self.error = "Close a document before opening more (24-tab limit)".into();
            }
            return;
        }
        self.documents.push(Document::from_readonly_text(
            PathBuf::new(),
            vpath,
            title,
            content,
            markdown,
        ));
        self.active_document = self.documents.len() - 1;
        self.overlay = None;
        self.view = View::Files;
    }
    fn task_agent_choices(&self, project: u64) -> Vec<crate::task_agents_model::Choice> {
        crate::task_agents_model::from_sessions(
            project,
            self.sessions.iter().map(|s| {
                (
                    s.id,
                    s.project,
                    &s.launch,
                    s.pane_name.as_deref(),
                    s.title.as_str(),
                )
            }),
        )
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
        let cards: Vec<_> =
            self.state
                .tasks
                .iter()
                .filter(|task| Some(task.project) == project)
                .map(|task| {
                    let timing = crate::task_card_timing::cached(
                        &task.timeline,
                        task.column,
                        self.state.language == crate::i18n::Language::French,
                    );
                    TaskCard {
                        id: task.id,
                        title: task.title.clone(),
                        column: task.column,
                        pane: task.timeline.pane_id,
                        timing: timing.text,
                        timing_title: timing.tooltip,
                        report: task.report.as_ref().map(|r| {
                            crate::tasks_board_panel::CardReport {
                                failed: r.outcome == crate::task_report::Outcome::Failed,
                                summary: r.summary.clone(),
                                files: r.files.clone(),
                            }
                        }),
                    }
                })
                .collect();
        if project.is_some() {
            ui.horizontal(|ui| {
                if ui.small_button(tr("Review task branch…")).clicked() {
                    self.open_review_modal();
                }
                if ui.small_button(tr("Project notes…")).clicked() {
                    self.open_notes_modal();
                }
            });
            ui.add_space(2.0_f32);
        }
        let agents: Vec<_> = project
            .map(|p| self.task_agent_choices(p))
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.id, p.label, p.provider))
            .collect();
        let decision_action = project.and_then(|project| {
            crate::task_decision_panel::show(
                ui,
                &self.state.tasks,
                project,
                self.state.language == crate::i18n::Language::French,
            )
        });
        let action = self.tasks_board.show(
            ui,
            TasksBoardInput {
                tasks: &cards,
                agents: &agents,
                french: self.state.language == crate::i18n::Language::French,
                project_selected: project.is_some(),
            },
        );
        let Some(project) = project else {
            return;
        };
        if let Some(action) = decision_action.or(action) {
            let showing = matches!(
                action,
                crate::tasks_board_panel::TasksBoardAction::ShowAgent { .. }
            );
            if self.apply_tasks_board_action(project, action) && showing {
                ui.memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
            }
        }
    }

    pub(crate) fn apply_tasks_board_action(
        &mut self,
        project: u64,
        action: crate::tasks_board_panel::TasksBoardAction,
    ) -> bool {
        self.capture_runtime_state();
        if let crate::tasks_board_panel::TasksBoardAction::AnswerDecision {
            id,
            request,
            ref answer,
        } = action
        {
            return match crate::task_decision_actions::answer(
                &mut self.state,
                self.store.as_ref(),
                &mut self.teams_hub,
                project,
                id,
                request,
                answer,
            ) {
                Ok(changed) => changed,
                Err(error) => {
                    self.error = error;
                    false
                }
            };
        }
        if let crate::tasks_board_panel::TasksBoardAction::ShowAgent { id, pane } = action {
            if crate::task_agents_model::can_focus(&self.state, project, id, pane) {
                self.active_session = Some(pane);
                self.view = View::Work;
                self.floating.ui.yield_keyboard();
                return true;
            }
            return false;
        }
        match crate::task_actions::commit(&mut self.state, self.store.as_ref(), project, action) {
            Ok(true) => true,
            Ok(false) => false,
            Err(error) => {
                self.error = tr(&error).into();
                false
            }
        }
    }
    fn can_restore(&self) -> bool {
        self.host_import.is_none()
            && self.store.is_some()
            && !self.file_action_pending
            && !self.transcript_trash_busy
            && !self.sessions.iter().any(|s| {
                s.terminal.is_some()
                    || (s.loading.is_some() || s.profile_migration.is_some())
                    || s.restarting.is_some()
                    || s.codex_restore.is_some()
            })
            && !self.chats.iter().any(|chat| chat.busy())
            && !self.claude_chats.iter().any(|chat| chat.busy())
            && !self.remote.needs_close_confirmation()
            && !self.documents.iter().any(Document::dirty)
            && !self.git_result.busy()
            && self.owner_task_launches.is_empty()
            && self.owner_task_turns.is_empty()
            && self.owner_task_reviews.is_empty()
            && !self.worktree_state.is_busy()
            && !self.scm_state.busy()
            && !self.account_busy()
            && !self.native_exit_pending()
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
        crate::task_history::preserve_for_restore(&self.state, &mut state)?;
        state.active_project = None;
        state.migrate_native_threads();
        state.prepare_task_change_from(state.task_cards_size()?)?;
        self.save();
        let backup = self
            .store
            .as_ref()
            .ok_or("Settings store unavailable")?
            .restore(&self.state, &state)?;
        self.sessions.clear();
        #[cfg(windows)]
        {
            self.native_terminals.picker = None;
        }
        self.chats.clear();
        self.claude_chats.clear();
        self.documents.clear();
        self.document_pending.clear();
        self.active_session = None;
        self.active_document = 0;
        self.pending_close = None;
        self.pending_scm_discard = None;
        self.rename_project_modal = None;
        self.right_fullscreen = false;
        self.commit_in_flight = false;
        self.share = Default::default();
        self.remote = Default::default();
        #[cfg(windows)]
        {
            let _ = self.agent_browsers.sync_presentation(None, None, false);
            self.agent_browsers = Default::default();
            self.show_agent_browser = false;
        }
        let (sender, receiver) = mpsc::channel();
        self.sender = sender;
        self.receiver = receiver;
        self.tasks_board.reset();
        self.tasks_board_project = None;
        self.gpui_extra.task_board = Default::default();
        self.new_task.open = false;
        self.gpui_extra.new_task_title.clear();
        self.state = state;
        self.refresh_custom_agents(None);
        self.remember_next_project_launch = self.state.launcher_prefs.remember_default.is_some();
        self.agents_panel = Default::default();
        self.agents_notice = None;
        self.host_panel.reset();
        self.host_notice = None;
        self.rename_agent = None;
        self.pane_context_menu = None;
        self.pane_context_menu_just_opened = false;
        self.next_session = self.state.next_pane_id();
        let restored_scrollback = self.state.terminal_prefs.scrollback;
        for saved in &self.state.sessions {
            self.next_session = self.next_session.max(saved.id + 1);
            self.sessions.push(Session {
                profile_data_dir: self
                    .store
                    .as_ref()
                    .map(|store| store.directory().to_owned()),
                profile_root: None,
                profile_migration: None,
                resume_required: saved.conversation_id.is_some(),
                ssh_retry: Default::default(),
                ssh_tmux_id: saved.ssh_tmux_id.clone(),
                native_host: saved.native_host.clone(),
                restarting: None,
                codex_discovery: None,
                codex_restore: None,
                model_picker: Default::default(),
                last_model: saved.last_model.clone(),
                model_save_attempt: None,
                settings_changed: false,
                codex_seed: saved.codex_seed.clone(),
                conversation_id: saved.conversation_id.clone(),
                profile_binding: saved.profile_binding.clone(),
                launch_fingerprint: saved.launch_fingerprint.clone(),
                session_options: if saved.launch.native_chat().is_some() {
                    saved.native_options.clone()
                } else {
                    saved.session_options.clone()
                },
                worktree: saved.worktree.clone(),
                pane_name: saved.pane_name.clone(),
                broadcast: saved.broadcast,
                id: saved.id,
                project: saved.project,
                title: saved.title.clone(),
                launch: saved.launch.clone(),
                cwd: saved.cwd.clone(),
                scrollback: restored_scrollback,
                terminal: None,
                loading: None,
                error: String::new(),
            });
        }
        self.explorer_tree
            .set_show_dotfiles(self.state.editor_prefs.show_git_ignored);
        self.explorer_tree
            .set_compact_folders(self.state.editor_prefs.compact_folders);
        self.scm_panel
            .set_group_order(self.state.git_prefs.scm_group_order.into_panel());
        self.scm_panel
            .set_view_mode(self.state.git_prefs.scm_view_mode.into_panel());
        self.refresh_language();
        crate::theme::apply(ctx, self.state.dark, self.state.theme);
        self.worktree_cleanup = None;
        self.pending_delete_worktree = None;
        self.pending_transcript_trash = None;
        self.refresh_project();
        self.backup_status = format!(
            "{} {}",
            tr("Settings restored. Previous settings saved at:"),
            backup.display()
        );
        Ok(())
    }
    fn restore_preview_modal(&mut self, ctx: &egui::Context) {
        if let Some(state) = &self.restore_preview {
            let mut apply = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("restore-settings")).show(ctx, |ui| {
                ui.heading(tr("Replace current settings?"));
                ui.label(format!(
                    "{}: {} · {}: {} · {}: {}",
                    tr("Projects"),
                    state.projects.len(),
                    tr("Tasks"),
                    state.tasks.len(),
                    tr("Sessions"),
                    state.sessions.len()
                ));
                ui.label(tr("Project folders are checked when opened, not during restore."));
                ui.collapsing(tr("Projects"), |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .show_rows(ui, 22.0, state.projects.len(), |ui, rows| {
                            for index in rows {
                                let project = &state.projects[index];
                                ui.label(format!("{} — {}", project.name, project.path.display()));
                            }
                        });
                });
                ui.label(tr("Current settings are backed up first. Sharing and agent-browser access stop. Restored sessions stay asleep."));
                if !self.can_restore() {
                    ui.label(tr(
                        "Stop active sessions and finish or save pending work before restoring settings",
                    ));
                }
                apply = ui
                    .add_enabled(self.can_restore(), egui::Button::new(tr("Replace settings")))
                    .clicked();
                cancel = ui.button(tr("Cancel")).clicked();
            });
            if apply
                && let Some(state) = self.restore_preview.take()
                && let Err(e) = self.restore_settings(state, ctx)
            {
                self.backup_status = e;
            }
            if cancel {
                self.restore_preview = None;
            }
        }
    }
    fn handle_settings_page_action(
        &mut self,
        ctx: &egui::Context,
        action: crate::settings_pages_panel::SettingsPageAction,
    ) {
        use crate::settings_pages_panel::SettingsPageAction;
        match action {
            SettingsPageAction::ExportSettings => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name("tessel-rust-settings.json")
                    .save_file()
                {
                    self.save();
                    self.backup_status = match crate::backup::export(&path, &self.state) {
                        Ok(()) => format!("{} {}", tr("Settings exported:"), path.display()),
                        Err(e) => e,
                    };
                }
            }
            SettingsPageAction::ImportSettings => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("JSON", &["json"])
                    .pick_file()
                {
                    match crate::backup::import(&path) {
                        Ok(state) => self.restore_preview = Some(state),
                        Err(e) => self.backup_status = e,
                    }
                }
            }
            SettingsPageAction::OpenDataFolder => {
                if let Some(store) = &self.store {
                    let dir = store.directory().to_path_buf();
                    #[cfg(windows)]
                    {
                        let mut command = std::process::Command::new("explorer.exe");
                        crate::agent_env::clean_command(&mut command);
                        let _ = command.arg(dir).spawn();
                    }
                    #[cfg(not(windows))]
                    {
                        ctx.open_url(egui::OpenUrl::new_tab(format!("file://{}", dir.display())));
                    }
                }
            }
            SettingsPageAction::CopyDiagnostics => {
                let data_dir = self
                    .store
                    .as_ref()
                    .map(|s| s.directory().display().to_string())
                    .unwrap_or_else(|| "<in-memory>".into());
                let diag = format!(
                    "Tessel Rust v{} ({})\nOS: {} ({})\nTheme: {} (dark={})\nLanguage: {:?}\nProjects: {}\nOpen panes: {}\nData directory: {}",
                    env!("CARGO_PKG_VERSION"),
                    env!("TESSEL_BUILD"),
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                    crate::theme::active(ctx).as_str(),
                    self.state.dark,
                    self.state.language,
                    self.state.projects.len(),
                    self.sessions.len(),
                    data_dir,
                );
                ctx.copy_text(diag);
            }
            SettingsPageAction::ResetAllSettings => {
                let defaults = Workspace::default();
                self.state.theme = defaults.theme;
                self.state.dark = defaults.dark;
                self.state.language = defaults.language;
                self.state.font_size = defaults.font_size;
                self.state.panels = defaults.panels;
                self.state.launcher_prefs = defaults.launcher_prefs;
                self.state.sidebar = defaults.sidebar;
                self.state.status_bar = defaults.status_bar;
                self.state.editor_prefs = defaults.editor_prefs;
                self.state.general_prefs = defaults.general_prefs;
                self.state.terminal_prefs = defaults.terminal_prefs;
                self.state.browser_prefs = defaults.browser_prefs;
                self.state.git_prefs = defaults.git_prefs;
                self.state.quick_commands = defaults.quick_commands;
                self.state.stats_prefs = defaults.stats_prefs;
                self.state.agent_prefs = defaults.agent_prefs;
                self.state.agent_model_defaults = defaults.agent_model_defaults;
                self.state.custom_agents = defaults.custom_agents;
                self.refresh_custom_agents(None);
                self.state.orchestration_prefs = defaults.orchestration_prefs;
                self.state.notification_prefs = defaults.notification_prefs;
                self.state.voice_prefs = defaults.voice_prefs;
                self.state.provider_accounts_prefs = defaults.provider_accounts_prefs;
                self.state.context_prefs = defaults.context_prefs;
                for sess in &mut self.sessions {
                    sess.scrollback = self.state.terminal_prefs.scrollback;
                }
                self.explorer_tree
                    .set_show_dotfiles(self.state.editor_prefs.show_git_ignored);
                self.explorer_tree
                    .set_compact_folders(self.state.editor_prefs.compact_folders);
                self.scm_panel
                    .set_group_order(self.state.git_prefs.scm_group_order.into_panel());
                self.scm_panel
                    .set_view_mode(self.state.git_prefs.scm_view_mode.into_panel());
                self.refresh_language();
                crate::theme::apply(ctx, self.state.dark, self.state.theme);
                self.refresh_launch_status();
                self.save();
                self.backup_status = tr("Settings reset to defaults.").into();
            }
            SettingsPageAction::SelectPreferredShell(shell) => {
                if self.state.launcher_prefs.preferred_shell != shell {
                    self.state.launcher_prefs.preferred_shell = shell;
                    self.save();
                }
            }
            SettingsPageAction::SetFontSize(size) => {
                if size.is_finite() {
                    let clamped = size.clamp(10.0, 28.0);
                    if (self.state.font_size - clamped).abs() > f32::EPSILON {
                        self.state.font_size = clamped;
                        self.save();
                    }
                }
            }
            SettingsPageAction::OpenPasswordsVault => {
                #[cfg(windows)]
                {
                    self.settings_section = SettingsSection::Passwords;
                }
            }
            SettingsPageAction::AddQuickCommand {
                name,
                text,
                press_enter,
            } => {
                if self.state.quick_commands.len() < 200 {
                    let next_id = self
                        .state
                        .quick_commands
                        .iter()
                        .map(|c| c.id)
                        .max()
                        .unwrap_or(0)
                        .saturating_add(1);
                    self.state.quick_commands.push(workspace::QuickCommand {
                        id: next_id,
                        name,
                        text,
                        press_enter,
                    });
                    self.save();
                }
            }
            SettingsPageAction::UpdateQuickCommand(updated) => {
                if let Some(existing) = self
                    .state
                    .quick_commands
                    .iter_mut()
                    .find(|c| c.id == updated.id)
                {
                    *existing = updated;
                    self.save();
                }
            }
            SettingsPageAction::RemoveQuickCommand(id) => {
                let before = self.state.quick_commands.len();
                self.state.quick_commands.retain(|c| c.id != id);
                if self.state.quick_commands.len() != before {
                    self.save();
                }
            }
            SettingsPageAction::RunQuickCommand(id) => {
                self.run_quick_command_by_id(ctx, id);
            }
            SettingsPageAction::OpenGeneralPage => {
                self.settings_section = SettingsSection::General;
            }
            SettingsPageAction::OpenExternalUrl(url) => {
                self.open_web_link(ctx, url);
            }
            SettingsPageAction::OpenNoticeFile(name) => {
                self.open_embedded_notice(name);
            }
        }
    }
    fn general_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let has_store = self.store.is_some();
        let (changed, action) = self.settings_pages.show_general(
            ui,
            &mut self.state.general_prefs,
            &self.backup_status,
            has_store,
            french,
        );
        if changed {
            self.save();
        }
        if let Some(act) = action {
            self.handle_settings_page_action(ui.ctx(), act);
        }
        self.restore_preview_modal(ui.ctx());
    }
    fn terminal_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let preferred = self.state.launcher_prefs.preferred_shell.clone();
        let font_size = self.state.font_size;
        let (changed, action) = self.settings_pages.show_terminal(
            ui,
            &mut self.state.terminal_prefs,
            preferred.as_deref(),
            font_size,
            self.wsl_available,
            french,
        );
        if changed {
            let sb = self.state.terminal_prefs.scrollback;
            for sess in &mut self.sessions {
                sess.scrollback = sb;
            }
            self.save();
        }
        if let Some(act) = action {
            self.handle_settings_page_action(ui.ctx(), act);
        }
    }
    fn editor_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        if self
            .settings_pages
            .show_editor(ui, &mut self.state.editor_prefs, french)
        {
            self.explorer_tree
                .set_show_dotfiles(self.state.editor_prefs.show_git_ignored);
            self.explorer_tree
                .set_compact_folders(self.state.editor_prefs.compact_folders);
            self.save();
        }
    }
    fn browser_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let (changed, action) = self.settings_pages.show_browser(
            ui,
            &mut self.state.browser_prefs,
            cfg!(windows),
            french,
        );
        if changed {
            #[cfg(windows)]
            if !self.state.browser_prefs.separate_agent_session {
                self.show_agent_browser = false;
            }
            self.save();
        }
        if let Some(act) = action {
            self.handle_settings_page_action(ui.ctx(), act);
        }
    }
    fn git_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        if self.settings_pages.show_git(
            ui,
            &mut self.state.git_prefs,
            self.git_username.as_deref(),
            french,
        ) {
            self.scm_panel
                .set_group_order(self.state.git_prefs.scm_group_order.into_panel());
            self.scm_panel
                .set_view_mode(self.state.git_prefs.scm_view_mode.into_panel());
            self.save();
        }
    }
    fn quick_commands_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let can_run = self.sessions.len() < 100
            || self.sessions.iter().any(|s| {
                s.project == self.state.active_project
                    && s.terminal.as_ref().is_some_and(|t| !t.exited())
            });
        if let Some(act) =
            self.settings_pages
                .show_quick_commands(ui, &self.state.quick_commands, can_run, french)
        {
            self.handle_settings_page_action(ui.ctx(), act);
        }
    }
    fn stats_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let report = self.status_telemetry.latest();
        if report.resources.is_none() {
            self.status_telemetry.request_stats_refresh();
        }
        let (app_rss_bytes, panes_rss_bytes, panes_cpu_pct) = match &report.resources {
            Some(res) => {
                let app_rss = res.app.iter().map(|a| a.memory_bytes as u64).sum::<u64>();
                let pane_rss = res
                    .terminals
                    .values()
                    .map(|t| t.memory_bytes as u64)
                    .sum::<u64>();
                let pane_cpu = res
                    .terminals
                    .values()
                    .map(|t| t.cpu_percent as f32)
                    .sum::<f32>();
                (app_rss, pane_rss, pane_cpu)
            }
            None => (0, 0, 0.0),
        };
        let running_agents = self
            .sessions
            .iter()
            .filter(|s| self.session_is_running_agent(s.id))
            .count();
        let sleeping_sessions = self
            .sessions
            .iter()
            .filter(|s| s.terminal.is_none() && s.launch.native_chat().is_none())
            .count();
        let mut tasks_todo = 0usize;
        let mut tasks_doing = 0usize;
        let mut tasks_review = 0usize;
        let mut tasks_done = 0usize;
        for t in &self.state.tasks {
            match t.column {
                0 => tasks_todo += 1,
                1 => tasks_doing += 1,
                2 => tasks_review += 1,
                _ => tasks_done += 1,
            }
        }
        let snapshot = crate::settings_pages_panel::StatsSnapshot {
            project_count: self.state.projects.len(),
            session_count: self.sessions.len(),
            running_agents,
            sleeping_sessions,
            open_documents: self.documents.len(),
            tasks_todo,
            tasks_doing,
            tasks_review,
            tasks_done,
            app_rss_bytes,
            panes_rss_bytes,
            panes_cpu_pct,
            listening_ports: report.ports.len(),
        };
        if self
            .settings_pages
            .show_stats(ui, &mut self.state.stats_prefs, &snapshot, french)
        {
            self.save();
        }
    }
    fn updates_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let has_store = self.store.is_some();
        if let Some(act) = self.settings_pages.show_updates(ui, has_store, french) {
            self.handle_settings_page_action(ui.ctx(), act);
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
        let mut changed = match action {
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
                self.refresh_language();
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
        ui.add_space(16.0);
        let spec = crate::theme::active(ui.ctx()).spec();
        egui::Frame::new()
            .fill(spec.ui.surface)
            .stroke(egui::Stroke::new(1.0_f32, spec.ui.border))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width().min(680.0));
                if ui
                    .checkbox(&mut self.state.status_bar.visible, tr("Show status bar"))
                    .changed()
                {
                    changed = true;
                }
                ui.add_space(2.0);
                ui.small(tr("Show or hide the bottom status bar."));
            });
        ui.add_space(12.0);
        let french = self.state.language == crate::i18n::Language::French;
        egui::Frame::new()
            .fill(spec.ui.surface)
            .stroke(egui::Stroke::new(1.0_f32, spec.ui.border))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width().min(680.0));
                ui.strong(if french {
                    "Moteur de rendu UI"
                } else {
                    "UI Renderer"
                });
                ui.add_space(6.0);
                for pref in [
                    workspace::RendererPreference::AutoGpui,
                    workspace::RendererPreference::CompatEgui,
                ] {
                    if ui
                        .radio_value(&mut self.state.renderer, pref, pref.label(french))
                        .changed()
                    {
                        changed = true;
                    }
                    ui.small(pref.description(french));
                    ui.add_space(4.0);
                }
                ui.small(if french {
                    "Prend effet au prochain démarrage (ou via --renderer gpui|compat)."
                } else {
                    "Takes effect on next launch (or override per run with --renderer gpui|compat)."
                });
            });
        if changed {
            self.save();
        }
    }

    fn render_status_bar(&mut self, ctx: &egui::Context) {
        use crate::native_host_manager::Failure;
        use crate::sidebar_model::TerminalMeta;
        use crate::status_metrics::{
            TelemetryRequest, TerminalPidProbe, WorkspacePortProbe, should_keep_awake,
        };
        use crate::status_strip::{
            RemoteHostRowInput, RemoteHostStatusKind, StatusBarAction, StatusBarInput,
            StatusBarItemId, Target,
        };
        use tessel_host_client::ConnectionState;

        let french = self.state.language == crate::i18n::Language::French;
        let mut any_agent_working = false;
        let mut active_pane_count = 0usize;
        let mut active_working_count = 0usize;
        let mut active_waiting_count = 0usize;

        for s in &self.sessions {
            let is_agent = matches!(
                s.launch,
                Launch::Agent(_) | Launch::CodexChat | Launch::ClaudeChat
            );
            let is_working = match &s.launch {
                Launch::CodexChat => self.chats.iter().any(|c| c.pane == s.id && c.working()),
                Launch::ClaudeChat => self
                    .claude_chats
                    .iter()
                    .any(|c| c.pane == s.id && c.working()),
                Launch::Agent(_) => {
                    s.loading.is_some()
                        || s.profile_migration.is_some()
                        || s.restarting.is_some()
                        || s.terminal.as_ref().is_some_and(|t| {
                            !t.exited() && t.active_within(Duration::from_millis(2500))
                        })
                }
                _ => false,
            };
            let is_waiting = match &s.launch {
                Launch::CodexChat => self
                    .chats
                    .iter()
                    .any(|c| c.pane == s.id && c.waiting_for_user()),
                Launch::ClaudeChat => self
                    .claude_chats
                    .iter()
                    .any(|c| c.pane == s.id && c.waiting_for_user()),
                _ => false,
            };
            if is_agent && is_working {
                any_agent_working = true;
            }
            if s.project == self.state.active_project {
                active_pane_count += 1;
                if is_working {
                    active_working_count += 1;
                }
                if is_waiting {
                    active_waiting_count += 1;
                }
            }
        }

        let keep_awake_mode = self.state.status_bar.keep_awake.into_mode();
        let keep_awake_active = self
            .keep_awake
            .sync(should_keep_awake(keep_awake_mode, any_agent_working));

        if !self.state.status_bar.visible {
            self.status_bar.close_popovers();
            return;
        }

        let ws_meta =
            |project_id: Option<u64>, projects: &[workspace::Project]| -> (String, String) {
                if let Some(pid) = project_id
                    && let Some(p) = projects.iter().find(|p| p.id == pid)
                {
                    return (format!("project:{pid}"), p.name.clone());
                }
                (
                    "__scratch__".to_owned(),
                    if french {
                        "Terminaux".to_owned()
                    } else {
                        "Terminals".to_owned()
                    },
                )
            };

        let mut terminals = Vec::with_capacity(self.sessions.len() + 1);
        let mut bound_terminals = std::collections::HashSet::with_capacity(self.sessions.len() + 1);
        let mut pid_probes = Vec::with_capacity(self.sessions.len() + 1);

        for s in &self.sessions {
            let (ws_key, ws_name) = ws_meta(s.project, &self.state.projects);
            let pid = match &s.launch {
                Launch::CodexChat => self
                    .chats
                    .iter()
                    .find(|c| c.pane == s.id)
                    .and_then(crate::chat::Chat::process_id),
                Launch::ClaudeChat => self
                    .claude_chats
                    .iter()
                    .find(|c| c.pane == s.id)
                    .and_then(crate::claude::Chat::process_id),
                _ => s
                    .terminal
                    .as_ref()
                    .and_then(|t| (!t.exited()).then(|| t.process_id()).flatten()),
            };
            let sid = s.id.to_string();
            if s.project == self.state.active_project {
                bound_terminals.insert(sid.clone());
            }
            let is_agent = matches!(
                s.launch,
                Launch::Agent(_) | Launch::CodexChat | Launch::ClaudeChat
            );
            terminals.push(TerminalMeta {
                id: sid.clone(),
                workspace_id: ws_key.clone(),
                workspace_name: ws_name,
                name: s.label().to_owned(),
                is_agent,
            });
            pid_probes.push(TerminalPidProbe {
                id: sid,
                workspace_key: ws_key,
                pid,
            });
        }

        if let Some(fpid) = self.floating.process_id() {
            let (f_key, f_name) = ws_meta(self.state.active_project, &self.state.projects);
            bound_terminals.insert("floating".to_owned());
            terminals.push(TerminalMeta {
                id: "floating".to_owned(),
                workspace_id: f_key.clone(),
                workspace_name: f_name,
                name: if french {
                    "Terminal flottant".to_owned()
                } else {
                    "Floating Terminal".to_owned()
                },
                is_agent: false,
            });
            pid_probes.push(TerminalPidProbe {
                id: "floating".to_owned(),
                workspace_key: f_key,
                pid: Some(fpid),
            });
        }

        let ws_probes: Vec<WorkspacePortProbe> = self
            .state
            .projects
            .iter()
            .map(|p| {
                let key = format!("project:{}", p.id);
                let pids = pid_probes
                    .iter()
                    .filter(|tp| tp.workspace_key == key)
                    .filter_map(|tp| tp.pid)
                    .collect();
                WorkspacePortProbe {
                    key,
                    name: p.name.clone(),
                    path: p.path.clone(),
                    pids,
                }
            })
            .collect();

        let (window_focused, window_minimized) =
            ctx.input(|i| (i.focused, i.viewport().minimized.unwrap_or(false)));
        let telemetry = self.status_telemetry.sync_and_read(&TelemetryRequest {
            window_focused,
            window_minimized,
            show_resources: self.state.status_bar.show_resources,
            resources_popover_open: self.status_bar.is_resources_open(),
            show_ports: self.state.status_bar.show_ports,
            ports_popover_open: self.status_bar.is_ports_open(),
            terminals: pid_probes,
            workspaces: ws_probes,
        });

        let mut seen_aliases = std::collections::HashSet::new();
        let mut remote_hosts = Vec::with_capacity(self.state.hosts.len());
        let mut push_alias = |alias: &str| {
            if alias.is_empty() || !seen_aliases.insert(alias.to_owned()) {
                return;
            }
            let status = if self.native_hosts.pending(alias) {
                RemoteHostStatusKind::Connecting
            } else if let Some(conn) = self.native_hosts.connection(alias) {
                match conn.state() {
                    ConnectionState::Connected { .. } | ConnectionState::Restarted { .. } => {
                        RemoteHostStatusKind::Connected
                    }
                    ConnectionState::Reconnecting { .. } => RemoteHostStatusKind::Reconnecting,
                    ConnectionState::Closed => RemoteHostStatusKind::Disconnected,
                }
            } else {
                match self.native_hosts.failure(alias) {
                    None | Some(Failure::Cancelled) => RemoteHostStatusKind::Disconnected,
                    Some(Failure::Connect) => RemoteHostStatusKind::AuthFailed,
                    Some(_) => RemoteHostStatusKind::Error,
                }
            };
            remote_hosts.push(RemoteHostRowInput {
                alias: alias.to_owned(),
                label: alias.to_owned(),
                status,
            });
        };
        for h in &self.state.hosts {
            push_alias(&h.alias);
        }
        let extra_aliases: Vec<String> = self.native_hosts.aliases().map(str::to_owned).collect();
        for alias in &extra_aliases {
            push_alias(alias);
        }

        let target_label = if self.view == View::Work {
            self.sessions
                .iter()
                .find(|s| {
                    Some(s.id) == self.active_session && s.project == self.state.active_project
                })
                .map_or("", Session::label)
        } else {
            ""
        };
        let target = if self.state.broadcast {
            Target::Broadcast(
                self.sessions
                    .iter()
                    .filter(|s| {
                        s.project == self.state.active_project
                            && s.broadcast
                            && s.launch.native_chat().is_none()
                    })
                    .count(),
            )
        } else if target_label.is_empty() {
            Target::None
        } else {
            Target::Input(target_label)
        };
        let project_path = self
            .state
            .active()
            .map(|p| p.path.display().to_string())
            .unwrap_or_default();
        let spec = crate::theme::active(ctx).spec();
        let error_banner = if !self.error.is_empty() {
            self.error.clone()
        } else {
            telemetry.last_stop_feedback.clone().unwrap_or_default()
        };

        let mut bar_action = None;
        egui::TopBottomPanel::bottom("status")
            .exact_height(24.0)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::chrome(ctx))
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(ctx, |ui| {
                let room = ui.available_rect_before_wrap();
                ui.painter().hline(
                    egui::Rangef::new(room.left() - 12.0, room.right() + 12.0),
                    room.top(),
                    ui.visuals().window_stroke,
                );
                bar_action = crate::status_strip::show_full(
                    ui,
                    &mut self.status_bar,
                    &StatusBarInput {
                        ui_colors: &spec.ui,
                        french,
                        target,
                        pane_count: active_pane_count,
                        working_count: active_working_count,
                        waiting_count: active_waiting_count,
                        project_path: &project_path,
                        keep_awake_mode,
                        keep_awake_active,
                        show_ssh: self.state.status_bar.show_ssh,
                        show_resources: self.state.status_bar.show_resources,
                        show_ports: self.state.status_bar.show_ports,
                        terminals: &terminals,
                        bound_terminals: &bound_terminals,
                        telemetry: &telemetry,
                        remote_hosts: &remote_hosts,
                        agent_updating: &[],
                        error_banner: &error_banner,
                        persistence_unavailable: self.store.is_none(),
                    },
                );
            });

        if let Some(act) = bar_action {
            match act {
                StatusBarAction::SetKeepAwake(mode) => {
                    self.state.status_bar.keep_awake = workspace::KeepAwakeSetting::from_mode(mode);
                    self.save();
                }
                StatusBarAction::ToggleItem(item) => {
                    match item {
                        StatusBarItemId::Ssh => {
                            self.state.status_bar.show_ssh = !self.state.status_bar.show_ssh;
                        }
                        StatusBarItemId::Resources => {
                            self.state.status_bar.show_resources =
                                !self.state.status_bar.show_resources;
                        }
                        StatusBarItemId::Ports => {
                            self.state.status_bar.show_ports = !self.state.status_bar.show_ports;
                        }
                    }
                    self.save();
                }
                StatusBarAction::FocusSession(sid) => {
                    if sid == "floating" {
                        if !self.floating.ui.is_open() {
                            self.toggle_floating(ctx);
                        }
                    } else if let Ok(id) = sid.parse::<u64>()
                        && let Some(proj) =
                            self.sessions.iter().find(|s| s.id == id).map(|s| s.project)
                    {
                        if proj != self.state.active_project {
                            self.state.active_project = proj;
                            self.refresh_project();
                        }
                        self.active_session = Some(id);
                        self.view = View::Work;
                        self.save();
                    }
                }
                StatusBarAction::ActivateWorkspace(ws_key) => {
                    if let Some(raw_id) = ws_key.strip_prefix("project:")
                        && let Ok(pid) = raw_id.parse::<u64>()
                        && self.state.projects.iter().any(|p| p.id == pid)
                        && self.state.active_project != Some(pid)
                    {
                        self.state.active_project = Some(pid);
                        self.refresh_project();
                        self.save();
                    }
                }
                StatusBarAction::OpenPortInBrowser { url, .. } => {
                    #[cfg(windows)]
                    {
                        self.browser.open_url(&url);
                        self.show_agent_browser = false;
                        self.view = View::Browser;
                    }
                    #[cfg(not(windows))]
                    {
                        let _ = url;
                    }
                }
                StatusBarAction::CopyPortAddress { address, .. } => {
                    ctx.copy_text(address);
                }
                StatusBarAction::StopPort(port) => {
                    self.status_telemetry.request_stop_port(port);
                }
                StatusBarAction::RefreshPorts => {
                    self.status_telemetry.refresh_ports_now();
                }
                StatusBarAction::ConnectRemoteHost(alias) => {
                    if self.native_hosts.failure(&alias).is_some()
                        || self
                            .native_hosts
                            .connection(&alias)
                            .is_some_and(|c| matches!(c.state(), ConnectionState::Closed))
                    {
                        self.native_hosts.cancel(&alias);
                    }
                    if let Err(err) = self.native_hosts.start(&alias) {
                        self.host_notice = Some((format!("{err:?}"), true));
                    }
                }
                StatusBarAction::DisconnectRemoteHost(alias) => {
                    self.native_hosts.cancel(&alias);
                }
                StatusBarAction::ManageRemoteHosts => {
                    self.overlay = Some(Overlay::Settings);
                    self.settings_section = SettingsSection::Hosts;
                }
                StatusBarAction::OpenAgentUpdates => {
                    self.overlay = Some(Overlay::Settings);
                    self.settings_section = SettingsSection::Agents;
                }
                StatusBarAction::DismissError => {
                    self.error.clear();
                    self.status_telemetry.clear_stop_feedback();
                }
            }
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
                    persistent_sessions: host.persistent_sessions,
                    host_daemon: host.host_daemon,
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
        let native_action =
            self.native_host_panel
                .show(ui, &self.state.hosts, &mut self.native_hosts);
        #[cfg(windows)]
        match native_action {
            Some(crate::native_host_panel::Action::NewTerminal(alias)) => {
                self.open_native_terminal(&alias, None, ui.ctx())
            }
            Some(crate::native_host_panel::Action::ExistingTerminals(alias)) => {
                self.open_native_picker(&alias, ui.ctx())
            }
            None => {}
        }
        #[cfg(not(windows))]
        let _ = native_action;
        let to_host = |host: SshHostSpec| Host {
            alias: host.alias,
            folder: host.folder,
            persistent_sessions: host.persistent_sessions,
            host_daemon: host.host_daemon,
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
                agent_folder_trust: self.state.agent_folder_trust,
            },
        );
        if !self.save_agent_draft() {
            return;
        }
        match action {
            Some(Action::SetAgentFolderTrust(on)) => {
                self.state.agent_folder_trust = on;
                self.save();
            }
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
            Some(Action::Install(id)) => {
                self.request_agent_install(&id);
            }
            _ => {}
        }
        let french = self.state.language == crate::i18n::Language::French;
        let custom_snapshot = self.state.custom_agents.clone();
        if let Some(extra_act) =
            self.settings_extra
                .show_custom_agents_section(ui, &custom_snapshot, french)
        {
            self.handle_settings_extra_action(ui.ctx(), extra_act);
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
                        name: crate::agent_preferences::display_name(&self.state, name).into(),
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
        let is_en = self.state.language != crate::i18n::Language::French;
        match self.state.active_project {
            Some(pid) => match self.worktree_state.summary(pid) {
                Some(summary) => {
                    let (eligible, reason) = summary.launch_availability(is_en);
                    self.launcher.set_worktree_availability(eligible, reason);
                }
                None => {
                    let msg = if is_en {
                        "Not a Git repository."
                    } else {
                        "Ce dossier n'est pas un dépôt Git."
                    };
                    self.launcher
                        .set_worktree_availability(false, Some(msg.to_owned()));
                }
            },
            None => self.launcher.clear_worktree_availability(),
        }
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
            Some(LaunchMenuAction::OpenAgentSettings) => {
                self.settings_section = SettingsSection::Agents;
                self.agents_panel.reveal_missing();
                self.overlay = Some(Overlay::Settings);
                true
            }
            Some(LaunchMenuAction::Launch {
                item,
                placement,
                in_worktree,
            }) => {
                if !entries
                    .iter()
                    .any(|entry| entry.available && entry.item == item)
                {
                    return false;
                }
                if let LaunchMenuItem::InstallAgent { id, .. } = item {
                    self.request_agent_install(&id);
                    return true;
                }
                if in_worktree {
                    let Some(pid) = self.state.active_project else {
                        return false;
                    };
                    let label = match &item {
                        LaunchMenuItem::Agent { id, .. } | LaunchMenuItem::ChatAgent { id, .. } => {
                            id.clone()
                        }
                        LaunchMenuItem::Shell { .. } => "shell".into(),
                        _ => "worktree".into(),
                    };
                    self.start_new_worktree_pane(
                        pid,
                        &label,
                        crate::worktree_state::WorktreeLaunchTarget::Menu { item, placement },
                    );
                    return true;
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
        let is_agent = self
            .sessions
            .iter()
            .find(|s| s.id == draft.pane)
            .is_some_and(|s| {
                matches!(
                    s.launch,
                    Launch::Agent(_) | Launch::CodexChat | Launch::ClaudeChat
                )
            });
        let mut submit = false;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("rename-agent")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(220.0, 420.0));
            ui.heading(tr(if is_agent {
                "Rename agent"
            } else {
                "Rename pane"
            }));
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
        let Some(target) = self.sessions.iter().find(|s| s.id == pane) else {
            return Err(tr("Agent changed; reopen its menu.").into());
        };
        if target.label() != expected {
            return Err(tr("Agent changed; reopen its menu.").into());
        }
        let is_agent = matches!(
            target.launch,
            Launch::Agent(_) | Launch::CodexChat | Launch::ClaudeChat
        );
        if !is_agent {
            let trimmed = name.trim();
            if trimmed.is_empty()
                || trimmed.chars().count() > 60
                || trimmed.chars().any(char::is_control)
            {
                return Err(tr("Choose a valid pane name (1–60 characters).").into());
            }
            let new_title = trimmed.to_owned();
            if let Some(store) = &self.store {
                let mut next = self.state.clone();
                if let Some(saved) = next.sessions.iter_mut().find(|s| s.id == pane) {
                    saved.title = new_title.clone();
                }
                store.save(&next)?;
                self.state.sessions = next.sessions;
            }
            if let Some(session) = self.sessions.iter_mut().find(|s| s.id == pane) {
                session.title = new_title;
            }
            return Ok(());
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

    fn live_share_project_list(&self) -> Vec<crate::share::SharedProject> {
        self.state
            .projects
            .iter()
            .map(|p| crate::share::SharedProject {
                id: p.id,
                name: p.name.clone(),
                path: p.path.clone(),
            })
            .collect()
    }

    fn sync_live_share(&mut self) {
        if self.share.history_store.is_none() {
            self.share.history_store =
                self.store.as_ref().map(|store| crate::share::HistoryStore {
                    profile: store.directory().to_owned(),
                    read: crate::local_file_history::shared_read,
                });
        }
        let all_projects = self.live_share_project_list();
        let (panes, pages) = if self.share.hosting() {
            let panes = self
                .sessions
                .iter()
                .filter_map(|s| {
                    let project_id = s.project?;
                    let is_agent = matches!(
                        s.launch,
                        Launch::Agent(_) | Launch::CodexChat | Launch::ClaudeChat
                    );
                    let output = s
                        .terminal
                        .as_ref()
                        .map_or_else(String::new, Terminal::output);
                    Some(crate::share::SharedPaneSnapshot {
                        id: s.id,
                        project_id,
                        title: s.label().to_owned(),
                        is_agent,
                        output,
                    })
                })
                .collect();
            #[cfg(windows)]
            let pages = self
                .browser
                .active_url()
                .map(|url| {
                    vec![crate::share::SharedBrowserPage {
                        project_id: self.state.active_project,
                        title: tr("Browser").to_owned(),
                        url: url.to_owned(),
                    }]
                })
                .unwrap_or_default();
            #[cfg(not(windows))]
            let pages = Vec::new();
            (panes, pages)
        } else {
            (Vec::new(), Vec::new())
        };

        let cmds = self.share.sync_host_workspace(
            &all_projects,
            panes,
            pages,
            self.state.active_project,
            self.active_session,
        );
        if let Some(cards) = self
            .share_tasks
            .update(&self.state, self.share.task_scope())
        {
            self.share.publish_tasks(cards);
        }
        for cmd in cmds {
            match cmd {
                crate::share::HostCommand::TaskMutation(command) => {
                    self.capture_runtime_state();
                    let state = &mut self.state;
                    let store = self.store.as_ref();
                    self.share.complete_task(command, |project, actor, op| {
                        crate::shared_task_apply::commit(state, store, project, actor, op)
                    });
                }
                crate::share::HostCommand::WriteTerminal { pane_id, bytes } => {
                    if let Some(session) = self.sessions.iter_mut().find(|s| s.id == pane_id)
                        && let Some(terminal) = &mut session.terminal
                    {
                        terminal.send_broadcast(&crate::terminal::BroadcastInput::Raw(bytes));
                    }
                }
                crate::share::HostCommand::ResizeTerminal { .. } => {}
                crate::share::HostCommand::OpenPane {
                    project_id,
                    agent_name,
                } => {
                    if self.state.projects.iter().any(|p| p.id == project_id) {
                        if self.state.active_project != Some(project_id) {
                            self.state.active_project = Some(project_id);
                            self.refresh_project();
                        }
                        let launch = match agent_name {
                            Some(agent)
                                if crate::agent_preferences::enabled(&self.state, &agent) =>
                            {
                                Launch::Agent(agent)
                            }
                            _ => Launch::Shell,
                        };
                        self.new_session(launch);
                    }
                }
                crate::share::HostCommand::ClosePane { pane_id } => {
                    self.close_session(pane_id);
                }
            }
        }
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
        if overlay == Overlay::Share {
            let all_projects = self.live_share_project_list();
            let french = self.state.language == crate::i18n::Language::French;
            if self.share.show_dialog_modal(ctx, &all_projects, french) {
                self.overlay = None;
            }
            if let Some(key) = self.share.workspace_file_request.take()
                && self.share.select_workspace_file(key)
            {
                self.overlay = None;
                self.view = View::GuestFiles;
            }
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
                    SUPPORTED_HELP_SHORTCUT_KEYS,
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
        if !self.native_modal_open()
            && self.restore_preview.is_none()
            && (close || response.should_close())
        {
            self.overlay = None;
        }
    }
    fn open_settings_by_page_id(&mut self, page_id: &str) {
        self.overlay = Some(Overlay::Settings);
        let next = match page_id {
            "agents" => SettingsSection::Agents,
            "accounts" => SettingsSection::Accounts,
            "skills" => SettingsSection::Skills,
            "mcp" => SettingsSection::Mcp,
            "connections" => SettingsSection::Connections,
            "context" => SettingsSection::Context,
            "orchestration" => SettingsSection::Orchestration,
            #[cfg(windows)]
            "tessel-account" => SettingsSection::Account,
            #[cfg(not(windows))]
            "tessel-account" => SettingsSection::Accounts,
            "general" => SettingsSection::General,
            "appearance" => SettingsSection::Appearance,
            "terminal" => SettingsSection::Terminal,
            "editor" => SettingsSection::Editor,
            "browser" => SettingsSection::Browser,
            "alerts" => SettingsSection::Alerts,
            "voice" => SettingsSection::Voice,
            "git" => SettingsSection::Git,
            "devtools" => SettingsSection::Devtools,
            "automations" => SettingsSection::Automations,
            "quick-commands" => SettingsSection::QuickCommands,
            "ssh" => SettingsSection::Hosts,
            "stats" => SettingsSection::Stats,
            "updates" => SettingsSection::Updates,
            #[cfg(windows)]
            "passwords" => SettingsSection::Passwords,
            _ => SettingsSection::General,
        };
        self.on_enter_settings_section(next);
        self.settings_section = next;
    }

    fn on_enter_settings_section(&mut self, section: SettingsSection) {
        match section {
            SettingsSection::Accounts => {
                if let Some(base) = directories::BaseDirs::new() {
                    self.cli_sign_ins =
                        crate::settings_extra_model::probe_cli_sign_ins(base.home_dir());
                    self.claude_system_email =
                        crate::settings_extra_model::probe_system_provider_email(
                            base.home_dir(),
                            "claude",
                        );
                    self.codex_system_email =
                        crate::settings_extra_model::probe_system_provider_email(
                            base.home_dir(),
                            "codex",
                        );
                } else {
                    self.cli_sign_ins = Default::default();
                    self.claude_system_email = None;
                    self.codex_system_email = None;
                }
                if let Some(store) = &self.store {
                    self.claude_managed_accounts =
                        crate::settings_extra_model::load_managed_accounts(
                            store.directory(),
                            "claude",
                        );
                    self.codex_managed_accounts =
                        crate::settings_extra_model::load_managed_accounts(
                            store.directory(),
                            "codex",
                        );
                    self.provider_secret_flags =
                        crate::settings_extra_model::load_provider_secret_flags(store.directory());
                }
            }
            SettingsSection::Skills => {
                self.refresh_scanned_skills();
            }
            SettingsSection::Mcp | SettingsSection::Connections => {
                self.refresh_installed_mcp_servers();
            }
            SettingsSection::Context => {
                self.refresh_context_scan();
            }
            SettingsSection::Devtools => {
                if self.devtools_statuses.is_empty() && self.devtools_rx.is_none() {
                    self.spawn_devtools_probe();
                }
            }
            SettingsSection::Automations => {
                if let Some(store) = &self.store {
                    self.automations_store =
                        crate::automations::AutomationsFile::load(store.directory());
                }
            }
            _ => {}
        }
    }

    fn refresh_scanned_skills(&mut self) {
        let Some(base) = directories::BaseDirs::new() else {
            return;
        };
        let proj = self.state.active().map(|p| p.path.clone());
        self.scanned_skills =
            crate::settings_extra_model::scan_skills(base.home_dir(), proj.as_deref());
    }

    fn refresh_installed_mcp_servers(&mut self) {
        let Some(base) = directories::BaseDirs::new() else {
            return;
        };
        let proj = self.state.active().map(|p| p.path.clone());
        let mut servers = crate::settings_extra_model::scan_installed_mcp_servers(
            base.home_dir(),
            proj.as_deref(),
        );
        #[cfg(windows)]
        let google_connected = self.account.is_signed_in();
        #[cfg(not(windows))]
        let google_connected = false;
        if let Some(store) = &self.store
            && self
                .retire_old_google
                .sync(store.directory(), google_connected, &servers)
        {
            servers = crate::settings_extra_model::scan_installed_mcp_servers(
                base.home_dir(),
                proj.as_deref(),
            );
        }
        self.installed_mcp_servers = servers;
    }

    fn refresh_context_scan(&mut self) {
        let Some(base) = directories::BaseDirs::new() else {
            return;
        };
        if self.scanned_skills.is_empty() {
            self.refresh_scanned_skills();
        }
        if self.installed_mcp_servers.is_empty() {
            self.refresh_installed_mcp_servers();
        }
        let proj = self.state.active().map(|p| p.path.clone());
        self.context_scan = crate::settings_extra_model::scan_project_context(
            base.home_dir(),
            proj.as_deref(),
            &self.scanned_skills,
            &self.installed_mcp_servers,
            self.state.context_prefs.model_family,
        );
    }

    fn spawn_devtools_probe(&mut self) {
        if self.devtools_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let res = crate::settings_extra_model::probe_devtools();
            let _ = tx.send(res);
        });
        self.devtools_rx = Some(rx);
    }

    fn accounts_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let ssh_aliases: Vec<String> = self.state.hosts.iter().map(|h| h.alias.clone()).collect();
        let (changed, act) = {
            let input = crate::settings_extra_panel::AccountsViewInput {
                claude_roster: &self.claude_managed_accounts,
                codex_roster: &self.codex_managed_accounts,
                claude_system_email: self.claude_system_email.as_deref(),
                codex_system_email: self.codex_system_email.as_deref(),
                secret_saved_flags: &self.provider_secret_flags,
                cli_sign_ins: &self.cli_sign_ins,
                ssh_host_aliases: &ssh_aliases,
                french,
            };
            self.settings_extra
                .show_accounts(ui, &mut self.state.provider_accounts_prefs, &input)
        };
        if changed {
            self.save();
        }
        if let Some(a) = act {
            self.handle_settings_extra_action(ui.ctx(), a);
        }
    }

    fn skills_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let has_proj = self.state.active_project.is_some();
        if let Some(a) = self
            .settings_extra
            .show_skills(ui, &self.scanned_skills, has_proj, french)
        {
            self.handle_settings_extra_action(ui.ctx(), a);
        }
    }

    fn mcp_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let targets = directories::BaseDirs::new()
            .map(|b| {
                let proj = self.state.active().map(|p| p.path.clone());
                crate::settings_extra_model::mcp_agent_config_targets(b.home_dir(), proj.as_deref())
            })
            .unwrap_or_default();
        let target_labels: Vec<String> = targets
            .iter()
            .map(|(_, label, p)| format!("{label} ({})", p.display()))
            .collect();
        if let Some(a) =
            self.settings_extra
                .show_mcp(ui, &self.installed_mcp_servers, &target_labels, french)
        {
            self.handle_settings_extra_action(ui.ctx(), a);
        }
    }

    fn connections_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        #[cfg(windows)]
        let signed_in = self.account.is_signed_in();
        #[cfg(not(windows))]
        let signed_in = false;
        if let Some(a) =
            self.settings_extra
                .show_connections(ui, signed_in, &self.installed_mcp_servers, french)
        {
            self.handle_settings_extra_action(ui.ctx(), a);
        }
    }

    fn context_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let prev_family = self.state.context_prefs.model_family;
        let (changed, act) = self.settings_extra.show_context(
            ui,
            &mut self.state.context_prefs,
            &self.context_scan,
            french,
        );
        if changed {
            if self.state.context_prefs.model_family != prev_family {
                self.refresh_context_scan();
            }
            self.save();
        }
        if let Some(a) = act {
            self.handle_settings_extra_action(ui.ctx(), a);
        }
    }

    fn orchestration_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        if self
            .settings_extra
            .show_orchestration(ui, &mut self.state.orchestration_prefs, french)
        {
            self.save();
        }
    }

    fn alerts_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let (changed, act) =
            self.settings_extra
                .show_alerts(ui, &mut self.state.notification_prefs, french);
        if changed {
            self.save();
        }
        if let Some(a) = act {
            self.handle_settings_extra_action(ui.ctx(), a);
        }
    }

    fn voice_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let (changed, act) =
            self.settings_extra
                .show_voice(ui, &mut self.state.voice_prefs, french);
        if changed {
            self.save();
        }
        if let Some(a) = act {
            self.handle_settings_extra_action(ui.ctx(), a);
        }
    }

    fn devtools_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let probing = self.devtools_rx.is_some();
        if let Some(a) =
            self.settings_extra
                .show_devtools(ui, &self.devtools_statuses, probing, french)
        {
            self.handle_settings_extra_action(ui.ctx(), a);
        }
    }

    fn automations_settings(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let projects: Vec<(u64, String, PathBuf)> = self
            .state
            .projects
            .iter()
            .map(|p| (p.id, p.name.clone(), p.path.clone()))
            .collect();
        if let Some(a) =
            self.settings_extra
                .show_automations(ui, &self.automations_store, &projects, french)
        {
            self.handle_settings_extra_action(ui.ctx(), a);
        }
    }

    fn run_shell_setup_command(
        &mut self,
        title: String,
        command: String,
        extra_env: Option<String>,
    ) {
        if self.sessions.len() >= 100 {
            self.error = "Session limit: 100".into();
            return;
        }
        let cwd = self
            .state
            .active()
            .map(|p| p.path.clone())
            .unwrap_or_else(launch::default_directory);
        let id = self.next_session;
        let Some(next) = self.next_session.checked_add(1) else {
            self.error = "Session identifiers exhausted".into();
            return;
        };
        self.next_session = next;
        let mut s = Session {
            profile_data_dir: None,
            profile_root: None,
            profile_migration: None,
            resume_required: false,
            ssh_retry: Default::default(),
            ssh_tmux_id: None,
            native_host: None,
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            settings_changed: false,
            codex_seed: None,
            conversation_id: None,
            profile_binding: None,
            launch_fingerprint: None,
            session_options: None,
            worktree: None,
            pane_name: None,
            broadcast: false,
            id,
            project: self.state.active_project,
            title: title.clone(),
            launch: Launch::Shell,
            cwd,
            scrollback: self.state.terminal_prefs.scrollback,
            terminal: None,
            loading: None,
            error: String::new(),
        };
        let prefs = extra_env.map(|env| crate::agent_preferences::Preferences {
            enabled: true,
            command: String::new(),
            args: String::new(),
            env,
        });
        let preferred_shell = self.state.launcher_prefs.preferred_shell.clone();
        s.start_with_shell(prefs, preferred_shell);
        self.sessions.push(s);
        self.active_session = Some(id);
        self.overlay = None;
        self.view = View::Work;
        self.active_agent_installs.push(ActiveAgentInstall {
            session_id: id,
            agent_name: title,
            pending_command: Some(command),
            attached_at: None,
            finished: false,
        });
        self.save();
    }

    fn save_custom_agents(
        &mut self,
        agents: Vec<crate::settings_extra_model::CustomAgent>,
    ) -> Result<(), String> {
        self.capture_runtime_state();
        let store = self.store.as_ref().ok_or("Settings store unavailable")?;
        let mut next = self.state.clone();
        next.custom_agents = agents;
        store.save(&next)?;
        self.state.custom_agents = next.custom_agents;
        Ok(())
    }

    fn save_custom_agent(
        &mut self,
        mut agent: crate::settings_extra_model::CustomAgent,
    ) -> Result<(), String> {
        let id = agent.id.clone();
        let mut next = self.state.custom_agents.clone();
        if let Some(existing) = next.iter_mut().find(|existing| existing.id == id) {
            agent.enabled = existing.enabled;
            *existing = agent;
        } else {
            next.push(agent);
        }
        self.save_custom_agents(next)?;
        self.refresh_custom_agents(Some(&id));
        Ok(())
    }

    fn handle_settings_extra_action(
        &mut self,
        ctx: &egui::Context,
        action: crate::settings_extra_panel::SettingsExtraAction,
    ) {
        use crate::settings_extra_panel::SettingsExtraAction as Act;
        let french = self.state.language == crate::i18n::Language::French;
        match action {
            Act::AddManagedAccount { provider, label } => {
                if let Some(store) = &self.store {
                    match crate::settings_extra_model::add_managed_account(
                        store.directory(),
                        &provider,
                        &label,
                    ) {
                        Ok(_) => {
                            let roster = crate::settings_extra_model::load_managed_accounts(
                                store.directory(),
                                &provider,
                            );
                            if provider == "claude" {
                                self.claude_managed_accounts = roster;
                            } else {
                                self.codex_managed_accounts = roster;
                            }
                            self.settings_extra.accounts_notice = if french {
                                "Profil ajouté.".into()
                            } else {
                                "Managed profile added.".into()
                            };
                        }
                        Err(e) => self.settings_extra.accounts_notice = e,
                    }
                }
            }
            Act::SelectManagedAccount {
                provider,
                account_id,
            } => {
                if let Some(store) = &self.store {
                    let mut roster = crate::settings_extra_model::load_managed_accounts(
                        store.directory(),
                        &provider,
                    );
                    roster.selected_id = account_id;
                    if let Err(e) = crate::settings_extra_model::save_managed_accounts(
                        store.directory(),
                        &provider,
                        &roster,
                    ) {
                        self.settings_extra.accounts_notice = e;
                    } else if provider == "claude" {
                        self.claude_managed_accounts = roster;
                    } else {
                        self.codex_managed_accounts = roster;
                    }
                }
            }
            Act::RemoveManagedAccount {
                provider,
                account_id,
            } => {
                if let Some(store) = &self.store {
                    let mut roster = crate::settings_extra_model::load_managed_accounts(
                        store.directory(),
                        &provider,
                    );
                    roster.accounts.retain(|a| a.id != account_id);
                    if roster.selected_id.as_deref() == Some(account_id.as_str()) {
                        roster.selected_id = None;
                    }
                    if let Some(dir) = crate::settings_extra_model::managed_account_profile_dir(
                        store.directory(),
                        &provider,
                        &account_id,
                    ) {
                        let _ = std::fs::remove_dir_all(dir);
                    }
                    let _ = crate::settings_extra_model::save_managed_accounts(
                        store.directory(),
                        &provider,
                        &roster,
                    );
                    if provider == "claude" {
                        self.claude_managed_accounts = roster;
                    } else {
                        self.codex_managed_accounts = roster;
                    }
                }
            }
            Act::LaunchManagedAccountSignIn {
                provider,
                account_id,
            } => {
                if let Some(store) = &self.store
                    && let Some(dir) = crate::settings_extra_model::managed_account_profile_dir(
                        store.directory(),
                        &provider,
                        &account_id,
                    )
                {
                    let _ = std::fs::create_dir_all(&dir);
                    let (env_key, cmd, title) = if provider == "claude" {
                        ("CLAUDE_CONFIG_DIR", "claude", "Claude sign-in")
                    } else {
                        ("CODEX_HOME", "codex login", "Codex sign-in")
                    };
                    let env_line = format!("{env_key}={}", dir.display());
                    self.run_shell_setup_command(title.into(), cmd.into(), Some(env_line));
                }
            }
            Act::SaveProviderSecret { key, value } => {
                if let Some(store) = &self.store {
                    match crate::settings_extra_model::set_provider_secret(
                        store.directory(),
                        &key,
                        &value,
                    ) {
                        Ok(()) => {
                            self.provider_secret_flags =
                                crate::settings_extra_model::load_provider_secret_flags(
                                    store.directory(),
                                );
                            self.settings_extra.accounts_notice = if french {
                                "Jeton chiffré localement.".into()
                            } else {
                                "Credential encrypted locally.".into()
                            };
                        }
                        Err(e) => self.settings_extra.accounts_notice = e,
                    }
                }
            }
            Act::ClearProviderSecret { key } => {
                if let Some(store) = &self.store
                    && crate::settings_extra_model::set_provider_secret(store.directory(), &key, "")
                        .is_ok()
                {
                    self.provider_secret_flags =
                        crate::settings_extra_model::load_provider_secret_flags(store.directory());
                }
            }
            Act::RefreshSkills => {
                self.refresh_scanned_skills();
            }
            Act::PickSkillFolder { project_scope } => {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    let dest_root = if project_scope {
                        self.state
                            .active()
                            .map(|p| p.path.join(".agents").join("skills"))
                    } else {
                        directories::BaseDirs::new()
                            .map(|b| b.home_dir().join(".agents").join("skills"))
                    };
                    if let Some(dest) = dest_root {
                        match crate::settings_extra_model::install_skill_from_folder(&folder, &dest)
                        {
                            Ok(installed) => {
                                self.settings_extra.skills_status = format!(
                                    "{}: {}",
                                    if french { "Installé" } else { "Installed" },
                                    installed.display()
                                );
                                self.refresh_scanned_skills();
                            }
                            Err(e) => self.settings_extra.skills_status = e,
                        }
                    }
                }
            }
            Act::CloneSkillFromGit {
                repo_url,
                project_scope,
            } => {
                let url_trimmed = repo_url.trim();
                let valid_scheme =
                    url_trimmed.starts_with("https://") || url_trimmed.starts_with("git@");
                if !valid_scheme || url_trimmed.starts_with('-') {
                    self.settings_extra.skills_status = if french {
                        "URL Git refusée (utilisez https:// ou git@).".into()
                    } else {
                        "Git URL rejected (use https:// or git@).".into()
                    };
                    return;
                }
                let dest_root = if project_scope {
                    self.state
                        .active()
                        .map(|p| p.path.join(".agents").join("skills"))
                } else {
                    directories::BaseDirs::new()
                        .map(|b| b.home_dir().join(".agents").join("skills"))
                };
                if let Some(dest) = dest_root {
                    let tmp_dir = std::env::temp_dir()
                        .join(format!("tessel-skill-clone-{}", std::process::id()));
                    let _ = std::fs::remove_dir_all(&tmp_dir);
                    let mut cmd = std::process::Command::new("git");
                    crate::agent_env::clean_command(&mut cmd);
                    cmd.args([
                        "-c",
                        "protocol.ext.allow=never",
                        "-c",
                        "protocol.fd.allow=never",
                        "clone",
                        "--depth",
                        "1",
                        "--",
                        url_trimmed,
                    ])
                    .arg(&tmp_dir);
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        cmd.creation_flags(0x0800_0000);
                    }
                    match cmd.output() {
                        Ok(out) if out.status.success() => {
                            let repo_name = url_trimmed
                                .trim_end_matches('/')
                                .trim_end_matches(".git")
                                .rsplit(['/', ':'])
                                .next()
                                .unwrap_or("skill");
                            let named_src = tmp_dir.parent().unwrap_or(&tmp_dir).join(repo_name);
                            let _ = std::fs::remove_dir_all(&named_src);
                            let src_to_copy = if std::fs::rename(&tmp_dir, &named_src).is_ok() {
                                named_src.clone()
                            } else {
                                tmp_dir.clone()
                            };
                            let res = crate::settings_extra_model::install_skill_from_folder(
                                &src_to_copy,
                                &dest,
                            );
                            let _ = std::fs::remove_dir_all(&tmp_dir);
                            let _ = std::fs::remove_dir_all(&named_src);
                            match res {
                                Ok(installed) => {
                                    self.settings_extra.skills_status =
                                        format!("Installed: {}", installed.display());
                                    self.refresh_scanned_skills();
                                }
                                Err(e) => self.settings_extra.skills_status = e,
                            }
                        }
                        Ok(out) => {
                            let _ = std::fs::remove_dir_all(&tmp_dir);
                            self.settings_extra.skills_status =
                                String::from_utf8_lossy(&out.stderr).trim().to_owned();
                        }
                        Err(e) => {
                            self.settings_extra.skills_status = e.to_string();
                        }
                    }
                }
            }
            Act::OpenSkillMarkdown(md_path) => {
                if let Some(parent) = md_path.parent().map(PathBuf::from) {
                    self.overlay = None;
                    self.open_document_at(parent, md_path);
                }
            }
            Act::RemoveSkillDir(skill_dir) => {
                match crate::settings_extra_model::remove_skill_dir(&skill_dir) {
                    Ok(()) => {
                        self.settings_extra.skills_status = if french {
                            "Compétence supprimée.".into()
                        } else {
                            "Skill removed.".into()
                        };
                        self.refresh_scanned_skills();
                    }
                    Err(e) => self.settings_extra.skills_status = e,
                }
            }
            Act::RefreshMcp => {
                self.refresh_installed_mcp_servers();
            }
            Act::InstallCatalogMcp {
                catalog_id,
                target_agent_idx,
                env_values,
            } => {
                if let Some(entry) = crate::settings_extra_model::MCP_CATALOG
                    .iter()
                    .find(|e| e.id == catalog_id)
                    && let Some(base) = directories::BaseDirs::new()
                {
                    let proj = self.state.active().map(|p| p.path.clone());
                    let targets = crate::settings_extra_model::mcp_agent_config_targets(
                        base.home_dir(),
                        proj.as_deref(),
                    );
                    if let Some((agent_id, _, cfg_path)) = targets.get(target_agent_idx) {
                        let args: Vec<String> =
                            entry.args.iter().map(|s| (*s).to_owned()).collect();
                        match crate::settings_extra_model::upsert_json_mcp_server(
                            cfg_path,
                            agent_id,
                            entry.id,
                            entry.transport,
                            entry.command_or_url,
                            &args,
                            &env_values,
                        ) {
                            Ok(()) => {
                                self.settings_extra.mcp_test_status = format!(
                                    "{}: {}",
                                    if french { "Ajouté" } else { "Added" },
                                    entry.name
                                );
                                self.refresh_installed_mcp_servers();
                            }
                            Err(e) => self.settings_extra.mcp_test_status = e,
                        }
                    }
                }
            }
            Act::AddCustomMcp {
                target_agent_idx,
                name,
                transport,
                endpoint,
                args,
                env_values,
            } => {
                if let Some(base) = directories::BaseDirs::new() {
                    let proj = self.state.active().map(|p| p.path.clone());
                    let targets = crate::settings_extra_model::mcp_agent_config_targets(
                        base.home_dir(),
                        proj.as_deref(),
                    );
                    if let Some((agent_id, _, cfg_path)) = targets.get(target_agent_idx) {
                        match crate::settings_extra_model::upsert_json_mcp_server(
                            cfg_path,
                            agent_id,
                            &name,
                            transport,
                            &endpoint,
                            &args,
                            &env_values,
                        ) {
                            Ok(()) => {
                                self.settings_extra.mcp_test_status = format!("Saved {name}.");
                                self.refresh_installed_mcp_servers();
                            }
                            Err(e) => self.settings_extra.mcp_error = e,
                        }
                    }
                }
            }
            Act::RemoveInstalledMcp {
                config_path,
                agent_id,
                name,
            } => {
                match crate::settings_extra_model::remove_json_mcp_server(
                    &config_path,
                    &agent_id,
                    &name,
                ) {
                    Ok(()) => self.refresh_installed_mcp_servers(),
                    Err(e) => self.settings_extra.mcp_test_status = e,
                }
            }
            Act::TestMcpServer {
                name,
                transport,
                endpoint,
            } => {
                if self.mcp_test_rx.is_none() {
                    self.settings_extra.mcp_test_status = format!("Testing {name}…");
                    let (tx, rx) = mpsc::channel();
                    thread::spawn(move || {
                        let res = match transport {
                            crate::settings_extra_model::McpTransport::Stdio => {
                                let parts: Vec<String> =
                                    endpoint.split_whitespace().map(str::to_owned).collect();
                                if let Some((cmd, rest)) = parts.split_first() {
                                    crate::settings_extra_model::probe_mcp_stdio(cmd, rest).map(
                                        crate::settings_extra_model::McpProbeOutcome::Connected,
                                    )
                                } else {
                                    Err("Empty command.".into())
                                }
                            }
                            crate::settings_extra_model::McpTransport::Http => {
                                crate::settings_extra_model::probe_mcp_http(&name, &endpoint)
                            }
                        };
                        let _ = tx.send((name, res));
                    });
                    self.mcp_test_rx = Some(rx);
                }
            }
            Act::SignInMcpAgent {
                name,
                agent_id,
                agent_label,
            } => {
                let french = self.state.language == crate::i18n::Language::French;
                let agents = vec![(agent_id, agent_label)];
                match crate::settings_extra_model::plan_mcp_sign_in(&agents, &name, french) {
                    Ok(plan) => {
                        if let Some(msg) = plan.card_message.clone() {
                            self.settings_extra
                                .mcp_card_messages
                                .insert(name.clone(), msg);
                        } else {
                            self.settings_extra.mcp_card_messages.remove(&name);
                        }
                        if let Some(cmd) = plan.chained_command(cfg!(windows)) {
                            self.run_shell_setup_command(plan.pane_title, cmd, None);
                        }
                    }
                    Err(err) => {
                        self.settings_extra.mcp_card_messages.insert(name, err);
                    }
                }
            }
            Act::SignInAllMcpServer { name } => {
                let french = self.state.language == crate::i18n::Language::French;
                let mut seen = std::collections::BTreeSet::new();
                let mut agents = Vec::new();
                for srv in &self.installed_mcp_servers {
                    if srv.name == name
                        && srv.transport == crate::settings_extra_model::McpTransport::Http
                        && seen.insert(srv.agent_id.clone())
                    {
                        agents.push((srv.agent_id.clone(), srv.agent_label.clone()));
                    }
                }
                match crate::settings_extra_model::plan_mcp_sign_in(&agents, &name, french) {
                    Ok(plan) => {
                        if let Some(msg) = plan.card_message.clone() {
                            self.settings_extra
                                .mcp_card_messages
                                .insert(name.clone(), msg);
                        } else {
                            self.settings_extra.mcp_card_messages.remove(&name);
                        }
                        if let Some(cmd) = plan.chained_command(cfg!(windows)) {
                            self.run_shell_setup_command(plan.pane_title, cmd, None);
                        }
                    }
                    Err(err) => {
                        self.settings_extra.mcp_card_messages.insert(name, err);
                    }
                }
            }
            Act::OpenTesselAccountSettings => {
                #[cfg(windows)]
                {
                    self.settings_section = SettingsSection::Account;
                }
            }
            Act::QuickAddProductivityMcp(catalog_id) => {
                self.settings_section = SettingsSection::Mcp;
                self.settings_extra.mcp_tab = crate::settings_extra_panel::McpTab::Catalog;
                self.settings_extra.mcp_catalog_filter = catalog_id.to_owned();
                self.refresh_installed_mcp_servers();
            }
            Act::RefreshContext => {
                self.refresh_context_scan();
            }
            Act::SendTestNotification => {
                #[cfg(windows)]
                if self.state.notification_prefs.alert_sound
                    != crate::settings_extra_model::AlertSound::None
                {
                    unsafe extern "system" {
                        fn MessageBeep(utype: u32) -> i32;
                    }
                    // MB_ICONASTERISK = 0x00000040
                    unsafe {
                        let _ = MessageBeep(0x0000_0040);
                    }
                }
                ctx.request_repaint();
            }
            Act::TriggerWindowsVoiceTyping => {
                #[cfg(windows)]
                {
                    // Synthesize Win+H to invoke native Windows Voice Typing.
                    use windows::Win32::UI::Input::KeyboardAndMouse::{
                        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
                        VIRTUAL_KEY, VK_LWIN,
                    };
                    let vk_h = VIRTUAL_KEY(0x48);
                    let inputs = [
                        INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: VK_LWIN,
                                    ..Default::default()
                                },
                            },
                        },
                        INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: vk_h,
                                    ..Default::default()
                                },
                            },
                        },
                        INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: vk_h,
                                    dwFlags: KEYEVENTF_KEYUP,
                                    ..Default::default()
                                },
                            },
                        },
                        INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: VK_LWIN,
                                    dwFlags: KEYEVENTF_KEYUP,
                                    ..Default::default()
                                },
                            },
                        },
                    ];
                    unsafe {
                        let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
                    }
                }
            }
            Act::RefreshDevtools => {
                self.spawn_devtools_probe();
            }
            Act::RunTerminalSetupCommand { title, command } => {
                self.run_shell_setup_command(title, command, None);
            }
            Act::OpenExternalUrl(url) => {
                self.open_web_link(ctx, url);
            }
            Act::UpsertAutomation(auto) => {
                if let Some(existing) = self
                    .automations_store
                    .automations
                    .iter_mut()
                    .find(|a| a.id == auto.id)
                {
                    *existing = auto;
                } else if self.automations_store.automations.len()
                    < crate::automations::MAX_AUTOMATIONS
                {
                    self.automations_store.automations.push(auto);
                }
                if let Some(store) = &self.store
                    && let Err(e) = self.automations_store.save(store.directory())
                {
                    self.settings_extra.automations_status = e;
                }
            }
            Act::DeleteAutomation(id) => {
                self.automations_store.automations.retain(|a| a.id != id);
                if let Some(store) = &self.store {
                    let _ = self.automations_store.save(store.directory());
                }
            }
            Act::ConfirmAndEnableAutomation(id) => {
                if let Some(auto) = self
                    .automations_store
                    .automations
                    .iter_mut()
                    .find(|a| a.id == id)
                {
                    auto.mark_confirmed();
                    auto.enabled = true;
                }
                if let Some(store) = &self.store {
                    let _ = self.automations_store.save(store.directory());
                }
            }
            Act::RunAutomationNow(id) => {
                self.trigger_automation_by_id(&id);
            }
            Act::SetAutomationsMaxConcurrent(val) => {
                self.automations_store.max_concurrent = val.clamp(1, 8);
                if let Some(store) = &self.store {
                    let _ = self.automations_store.save(store.directory());
                }
            }
            Act::UpsertCustomAgent(agent) => match self.save_custom_agent(agent) {
                Ok(()) => {
                    self.settings_extra.custom_agent_form_open = false;
                    self.settings_extra.editing_custom_id = None;
                    self.settings_extra.custom_error.clear();
                }
                Err(error) => self.settings_extra.custom_error = error,
            },
            Act::DeleteCustomAgent(id) => {
                let mut next = self.state.custom_agents.clone();
                next.retain(|agent| agent.id != id);
                match self.save_custom_agents(next) {
                    Ok(()) => self.refresh_custom_agents(Some(&id)),
                    Err(error) => self.error = error,
                }
            }
            Act::ToggleCustomAgent { id, enabled } => {
                let mut next = self.state.custom_agents.clone();
                if let Some(agent) = next.iter_mut().find(|agent| agent.id == id) {
                    agent.enabled = enabled;
                    if let Err(error) = self.save_custom_agents(next) {
                        self.error = error;
                    }
                }
            }
        }
    }

    fn trigger_automation_by_id(&mut self, automation_id: &str) {
        let Some(store_dir) = self.store.as_ref().map(|s| s.directory().to_path_buf()) else {
            return;
        };
        let Some(idx) = self
            .automations_store
            .automations
            .iter()
            .position(|a| a.id == automation_id)
        else {
            return;
        };
        let auto = self.automations_store.automations[idx].clone();
        if !auto.is_confirmed() {
            self.settings_extra.pending_confirm_automation = Some(auto.id.clone());
            return;
        }
        let Some(project) = self
            .state
            .projects
            .iter()
            .find(|p| p.id == auto.project_id)
            .cloned()
        else {
            return;
        };
        let open_for_this = self
            .sessions
            .iter()
            .filter(|s| {
                s.pane_name
                    .as_deref()
                    .is_some_and(|n| n.starts_with(&format!("auto:{}:", auto.id)))
            })
            .count();
        if open_for_this >= crate::automations::MAX_OPEN_PANES_PER_AUTOMATION {
            return;
        }
        let (run_id, prompt_file) = match crate::automations::generate_automation_id("run")
            .and_then(|run_id| {
                crate::automations::write_automation_prompt_file(&store_dir, &run_id, &auto.prompt)
                    .map(|file| (run_id, file))
            }) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.error = error.clone();
                self.settings_extra.automations_status = error;
                return;
            }
        };
        let instruction = crate::automations::safe_prompt_instruction(&prompt_file);
        let agent_id = auto.agent.as_id().to_owned();
        let before_next = self.next_session;
        self.new_session_in_worktree(Some(project.id), Launch::Agent(agent_id), None, false, None);
        let spawned_pane_id =
            self.sessions
                .iter_mut()
                .find(|s| s.id == before_next)
                .map(|created| {
                    created.pane_name = Some(format!("auto:{}:{}", auto.id, auto.name));
                    self.active_agent_installs.push(ActiveAgentInstall {
                        session_id: created.id,
                        agent_name: auto.name.clone(),
                        pending_command: Some(instruction),
                        attached_at: None,
                        finished: false,
                    });
                    created.id
                });
        let started_at_unix = chrono::Local::now().timestamp().max(0) as u64;
        if let Some(stored) = self.automations_store.automations.get_mut(idx) {
            stored.runs.push(crate::automations::AutomationRunRecord {
                run_id,
                started_at_unix,
                trigger: "run".into(),
                pane_id: spawned_pane_id,
                prompt_file,
            });
            if stored.runs.len() > crate::automations::MAX_RUNS_PER_AUTOMATION {
                let drop_n = stored.runs.len() - crate::automations::MAX_RUNS_PER_AUTOMATION;
                stored.runs.drain(0..drop_n);
            }
        }
        if self.state.notification_prefs.notifications_enabled
            && self.state.notification_prefs.in_app_alerts
        {
            self.push_notification(
                crate::dialogs_toolbar_model::NotificationKind::AutomationRan,
                format!("Automation started: {}", auto.name),
                project.name.clone(),
                spawned_pane_id,
            );
        }
        let _ = self.automations_store.save(&store_dir);
    }

    fn poll_automations(&mut self) {
        let now = chrono::Local::now();
        let minute_key = now.timestamp() / 60;
        if self.last_automation_minute == Some(minute_key) {
            return;
        }
        self.last_automation_minute = Some(minute_key);
        let max_conc = usize::from(self.automations_store.max_concurrent.clamp(1, 8));
        let running_auto_panes = self
            .sessions
            .iter()
            .filter(|s| {
                s.pane_name
                    .as_deref()
                    .is_some_and(|n| n.starts_with("auto:"))
                    && s.terminal.as_ref().is_some_and(|t| !t.exited())
            })
            .count();
        if running_auto_panes >= max_conc {
            return;
        }
        let slots = max_conc - running_auto_panes;
        let due_ids: Vec<String> = self
            .automations_store
            .automations
            .iter()
            .filter(|a| a.enabled && a.is_confirmed() && a.schedule.matches(&now))
            .take(slots)
            .map(|a| a.id.clone())
            .collect();
        for id in due_ids {
            self.trigger_automation_by_id(&id);
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
                "agent codex claude custom cli install detection visibility command model",
            ),
            (
                SettingsSection::Accounts,
                "accounts",
                "AI provider accounts",
                Group::Ai,
                Icon::Account,
                "accounts comptes provider claude codex gemini opencode minimax zcode quota oauth api key",
            ),
            (
                SettingsSection::Skills,
                "skills",
                "Skills",
                Group::Ai,
                Icon::FolderPlus,
                "skills competences skill.md instructions folder git install",
            ),
            (
                SettingsSection::Mcp,
                "mcp",
                "MCP servers",
                Group::Ai,
                Icon::Plug,
                "mcp servers serveurs model context protocol catalog stdio http sse tools",
            ),
            (
                SettingsSection::Connections,
                "connections",
                "Connections",
                Group::Ai,
                Icon::Cable,
                "connections connexions gmail calendar drive outlook dropbox github notion linear sentry",
            ),
            (
                SettingsSection::Context,
                "context",
                "Context",
                Group::Ai,
                Icon::Files,
                "context contexte tokens claude.md agents.md instructions rules budget",
            ),
            (
                SettingsSection::Orchestration,
                "orchestration",
                "Orchestration",
                Group::Ai,
                Icon::Layout,
                "orchestration teams equipes workers subagents relay inter-project",
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
                SettingsSection::General,
                "general",
                "General",
                Group::Workspace,
                Icon::SlidersHorizontal,
                "general startup restore quit close confirm workspace worktree backup export import diagnostics reset sauvegarde",
            ),
            (
                SettingsSection::Appearance,
                "appearance",
                "Appearance",
                Group::Workspace,
                Icon::Appearance,
                "theme language langue police font status bar",
            ),
            (
                SettingsSection::Terminal,
                "terminal",
                "Terminal",
                Group::Workspace,
                Icon::Terminal,
                "terminal shell powershell cmd wsl scrollback cursor line height paste focus tile",
            ),
            (
                SettingsSection::Editor,
                "editor",
                "Editor",
                Group::Workspace,
                Icon::Files,
                "editor editeur auto save syntax line numbers wrap preview tabs tab size dotfiles compact diff",
            ),
            (
                SettingsSection::Browser,
                "browser",
                "Browser",
                Group::Workspace,
                Icon::MonitorSmartphone,
                "browser navigateur links webview session agent isolate cookies",
            ),
            (
                SettingsSection::Alerts,
                "alerts",
                "Notifications",
                Group::Workspace,
                Icon::Bell,
                "notifications alertes sound chime ping desktop toast bell volume",
            ),
            (
                SettingsSection::Voice,
                "voice",
                "Voice typing",
                Group::Workspace,
                Icon::MonitorSmartphone,
                "voice dictee vocale dictation speech microphone language",
            ),
            #[cfg(windows)]
            (
                SettingsSection::Passwords,
                "passwords",
                "Passwords",
                Group::Workspace,
                Icon::ShieldCheck,
                "password mot passe vault coffre csv import dpapi",
            ),
            (
                SettingsSection::Git,
                "git",
                "Git",
                Group::Workflows,
                Icon::GitBranch,
                "git branch prefix worktree changes tree list staged unstaged",
            ),
            (
                SettingsSection::Devtools,
                "devtools",
                "Developer tools",
                Group::Workflows,
                Icon::Terminal,
                "developer tools outils developpement git gh node python rustup go docker rg fd jq winget",
            ),
            (
                SettingsSection::Automations,
                "automations",
                "Automations",
                Group::Workflows,
                Icon::RefreshCw,
                "automations automatisations cron schedule planifie templates background runs",
            ),
            (
                SettingsSection::QuickCommands,
                "quick-commands",
                "Quick commands",
                Group::Workflows,
                Icon::ListChecks,
                "quick commands commandes rapides snippet macro run",
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
                SettingsSection::Stats,
                "stats",
                "Stats",
                Group::Insights,
                Icon::Activity,
                "stats statistiques usage telemetry memory cpu ports sessions tasks",
            ),
            (
                SettingsSection::Updates,
                "updates",
                "Updates",
                Group::About,
                Icon::RefreshCw,
                "updates mises a jour version build license notices feedback",
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
            .unwrap_or("general");
        let host_dialog_open = self.settings_section == SettingsSection::Hosts
            && (self.host_panel.is_form_open() || self.host_panel.is_confirm_remove_open());
        let agents_escape_consumed = self.settings_section == SettingsSection::Agents
            && self.restore_preview.is_none()
            && self.agents_panel.handle_escape(ctx);
        #[cfg(windows)]
        let vault_escape_consumed = self.settings_section == SettingsSection::Passwords
            && self.restore_preview.is_none()
            && self
                .vault
                .handle_escape(ctx, self.settings_nav.owns_focus(ctx));
        #[cfg(not(windows))]
        let vault_escape_consumed = false;
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
                            if self.native_modal_open()
                                || self.restore_preview.is_some()
                                || self.pending_agent_install.is_some()
                                || self.pending_transcript_trash.is_some()
                                || host_dialog_open
                            {
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
                                            self.settings_pages.reset_transient();
                                            self.on_enter_settings_section(*section);
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
                        if self.native_modal_open()
                            || self.pending_agent_install.is_some()
                            || self.pending_transcript_trash.is_some()
                        {
                            ui.disable();
                        }
                        crate::settings_surface::body(ui, |ui| match self.settings_section {
                            SettingsSection::General => self.general_settings(ui),
                            SettingsSection::Appearance => self.appearance_settings(ui),
                            SettingsSection::Terminal => self.terminal_settings(ui),
                            SettingsSection::Editor => self.editor_settings(ui),
                            SettingsSection::Browser => self.browser_settings(ui),
                            SettingsSection::Git => self.git_settings(ui),
                            SettingsSection::QuickCommands => self.quick_commands_settings(ui),
                            SettingsSection::Hosts => self.host_settings(ui),
                            SettingsSection::Agents => self.agent_settings(ui),
                            SettingsSection::Accounts => self.accounts_settings(ui),
                            SettingsSection::Skills => self.skills_settings(ui),
                            SettingsSection::Mcp => self.mcp_settings(ui),
                            SettingsSection::Connections => self.connections_settings(ui),
                            SettingsSection::Context => self.context_settings(ui),
                            SettingsSection::Orchestration => self.orchestration_settings(ui),
                            SettingsSection::Alerts => self.alerts_settings(ui),
                            SettingsSection::Voice => self.voice_settings(ui),
                            SettingsSection::Devtools => self.devtools_settings(ui),
                            SettingsSection::Automations => self.automations_settings(ui),
                            SettingsSection::Stats => self.stats_settings(ui),
                            SettingsSection::Updates => self.updates_settings(ui),
                            #[cfg(windows)]
                            SettingsSection::Passwords => self.vault.show(
                                ui,
                                &mut self.browser,
                                self.state.language == crate::i18n::Language::French,
                            ),
                            #[cfg(windows)]
                            SettingsSection::Account => {
                                self.account.show_with_workspace(ui, &self.state)
                            }
                        })
                    },
                );
            });
        });
        if self.restore_preview.is_none()
            && !self.native_modal_open()
            && self.pending_agent_install.is_none()
            && !host_dialog_open
            && !agents_escape_consumed
            && !vault_escape_consumed
            && (close || response.should_close())
        {
            if self.settings_section == SettingsSection::Agents {
                self.agents_panel.save_open_draft();
            }
            if self.save_agent_draft() {
                self.overlay = None;
                self.settings_nav.reset();
                self.settings_pages.reset_transient();
            }
        }
    }

    fn transcript_source_for_trash(
        &self,
        entry: &crate::side_panels_model::SessionHistoryEntry,
    ) -> Result<crate::side_panels_model::SessionSource, String> {
        if self.sessions.iter().any(|s| {
            let terminal_match = s.conversation_id.as_ref() == Some(&entry.id)
                && (s.profile_binding.is_none() || s.profile_binding == entry.profile_binding)
                && matches!(&s.launch, Launch::Agent(agent) if agent == &entry.agent_id);
            let native_match = self.state.native_threads.get(&s.id).is_some_and(|thread| {
                thread.thread == entry.id && thread.provider.eq_ignore_ascii_case(&entry.agent_id)
            });
            terminal_match || native_match
        }) {
            return Err("Close this conversation's pane before recycling its transcript.".into());
        }
        crate::transcript_trash::source_for(
            entry,
            &self.past_sessions_cache,
            &self.session_history_sources(),
        )
    }

    fn confirm_transcript_trash(&mut self) {
        if self.transcript_trash_busy {
            return;
        }
        let Some(entry) = self.pending_transcript_trash.take() else {
            return;
        };
        let source = match self.transcript_source_for_trash(&entry) {
            Ok(source) => source,
            Err(error) => {
                self.error = error;
                return;
            }
        };
        self.transcript_trash_busy = true;
        let sender = self.sender.clone();
        let data = self.store.as_ref().map(|s| s.directory().to_owned());
        thread::spawn(move || {
            let result = crate::session_profile::check_account_available(
                &source.binding,
                &source.agent_id,
                data.as_deref(),
            )
            .and_then(|()| crate::transcript_trash::recycle(&source, &entry));
            let _ = sender.send(Message::TranscriptTrash(entry, result));
        });
    }

    fn confirmations(&mut self, ctx: &egui::Context) {
        if let Some(entry) = self.pending_transcript_trash.clone() {
            let mut confirm = false;
            let mut cancel = false;
            let response = egui::Modal::new(egui::Id::new("recycle-conversation")).show(ctx, |ui| {
                ui.heading(tr("Move conversation to Recycle Bin?"));
                ui.label(&entry.title);
                ui.label(tr("The local transcript will be removed from history. Close its pane before continuing."));
                confirm = ui.button(tr("Move to Recycle Bin")).clicked();
                cancel = ui.button(tr("Cancel")).clicked();
            });
            if cancel || response.should_close() {
                self.pending_transcript_trash = None;
            } else if confirm {
                self.confirm_transcript_trash();
            }
        }

        if let Some(modal) = &mut self.worktree_cleanup {
            let is_en = self.state.language != crate::i18n::Language::French;
            let confirm_before_remove = self.state.general_prefs.confirm_delete_workspace;
            let pid = modal.project_id;
            let cleanup_action = modal.ui(ctx, is_en, confirm_before_remove);
            match cleanup_action {
                Some(crate::worktree_cleanup_panel::WorktreeCleanupAction::Close) => {
                    self.worktree_cleanup = None;
                }
                Some(crate::worktree_cleanup_panel::WorktreeCleanupAction::Refresh) => {
                    if let Some(proj) = self.state.projects.iter().find(|p| p.id == pid).cloned() {
                        let counts = self.worktree_open_pane_counts(pid);
                        self.worktree_state
                            .request_cleanup_scan(pid, proj.path, counts);
                    }
                }
                Some(crate::worktree_cleanup_panel::WorktreeCleanupAction::RemoveSelected(
                    targets,
                )) => {
                    if let Some(proj) = self.state.projects.iter().find(|p| p.id == pid).cloned() {
                        let paths: Vec<PathBuf> =
                            targets.iter().map(|(p, _, _)| p.clone()).collect();
                        self.close_sessions_for_worktree_paths(pid, &paths);
                        self.worktree_state
                            .request_remove_batch(pid, proj.path, targets);
                    }
                }
                None => {}
            }
        }
        if let Some((pid, wt_path, branch)) = self.pending_delete_worktree.clone() {
            let mut confirm = false;
            let mut cancel = false;
            let resp =
                egui::Modal::new(egui::Id::new("confirm-delete-worktree-modal")).show(ctx, |ui| {
                    ui.set_width((ctx.content_rect().width() - 48.0).clamp(240.0, 440.0));
                    ui.heading(tr("Delete worktree?"));
                    ui.add_space(4.0);
                    if !branch.is_empty() {
                        ui.monospace(&branch);
                    }
                    ui.monospace(wt_path.display().to_string());
                    ui.add_space(4.0);
                    ui.label(tr(
                        "This deletes the worktree folder and its local branch. Open panes in this worktree will be closed.",
                    ));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("Cancel")).clicked() {
                            cancel = true;
                        }
                        let danger = ui.visuals().error_fg_color;
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(tr("Delete worktree"))
                                        .color(egui::Color32::WHITE)
                                        .strong(),
                                )
                                .fill(danger),
                            )
                            .clicked()
                        {
                            confirm = true;
                        }
                    });
                });
            if cancel || resp.should_close() {
                self.pending_delete_worktree = None;
            } else if confirm {
                self.pending_delete_worktree = None;
                if let Some(proj) = self.state.projects.iter().find(|p| p.id == pid).cloned() {
                    self.close_sessions_for_worktree_paths(pid, std::slice::from_ref(&wt_path));
                    self.worktree_state.request_remove_batch(
                        pid,
                        proj.path,
                        vec![(wt_path, branch, true)],
                    );
                }
            }
        }
        if let Some(index) = self.pending_close {
            egui::Modal::new(egui::Id::new("unsaved-file")).show(ctx, |ui| {
                ui.label(tr("Save changes before closing this file?"));
                ui.label(self.documents[index].path.display().to_string());
                ui.horizontal(|ui| {
                    if ui.button(tr("Cancel")).clicked() {
                        self.pending_close = None;
                    }
                    if ui.button(tr("Save and close")).clicked() {
                        match self.documents[index].save(self.store.as_ref().map(|s| s.directory()))
                        {
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
        if let Some((area, paths, copy)) = self.pending_scm_discard.clone() {
            let mut confirm = false;
            let mut cancel = false;
            let resp = egui::Modal::new(egui::Id::new("scm-discard-confirm")).show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 48.0).clamp(240.0, 440.0));
                ui.heading(&copy.title);
                ui.add_space(4.0);
                ui.label(&copy.description);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(tr("Cancel")).clicked() {
                        cancel = true;
                    }
                    let danger = ui.visuals().error_fg_color;
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(&copy.confirm_label)
                                    .color(egui::Color32::WHITE)
                                    .strong(),
                            )
                            .fill(danger),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
            if cancel || resp.should_close() {
                self.pending_scm_discard = None;
            } else if confirm {
                self.pending_scm_discard = None;
                match self.scm_state.discard_selection(area, &paths) {
                    Ok((tracked, untracked)) => {
                        self.git_start_work(move |root| {
                            crate::git::discard_worktree(&root, &tracked, &untracked)
                        });
                    }
                    Err(error) => self.error = error,
                }
            }
        }
        if let Some(spec) = self.pending_agent_install.clone() {
            let french = self.state.language == crate::i18n::Language::French;
            let (decision, open_url) =
                crate::agents_panel::show_install_confirm_modal(ctx, &spec, french);
            if let Some(url) = open_url {
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
            match decision {
                Some(true) => {
                    self.pending_agent_install = None;
                    self.start_agent_install(spec.execution);
                }
                Some(false) => {
                    self.pending_agent_install = None;
                }
                None => {}
            }
        }
        if let Some((project_id, mut draft, mut focused)) = self.rename_project_modal.clone() {
            let mut submit = false;
            let mut cancel = false;
            let resp = egui::Modal::new(egui::Id::new("rename-project-modal")).show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 48.0).clamp(220.0, 400.0));
                ui.heading(tr("Rename project"));
                let field = ui.add(
                    egui::TextEdit::singleline(&mut draft)
                        .char_limit(80)
                        .desired_width(ui.available_width()),
                );
                if !focused {
                    field.request_focus();
                    focused = true;
                }
                if (field.has_focus() || field.lost_focus())
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                {
                    submit = true;
                }
                ui.horizontal(|ui| {
                    if ui.button(tr("Cancel")).clicked() {
                        cancel = true;
                    }
                    if ui
                        .add_enabled(!draft.trim().is_empty(), egui::Button::new(tr("Save")))
                        .clicked()
                    {
                        submit = true;
                    }
                });
            });
            if cancel || resp.should_close() {
                self.rename_project_modal = None;
            } else if submit {
                let trimmed = draft.trim();
                if !trimmed.is_empty()
                    && !trimmed.chars().any(char::is_control)
                    && let Some(proj) = self.state.projects.iter_mut().find(|p| p.id == project_id)
                {
                    proj.name = trimmed.to_owned();
                    self.save();
                }
                self.rename_project_modal = None;
            } else {
                self.rename_project_modal = Some((project_id, draft, focused));
            }
        }
        if let Some(session_id) = self.pending_close_session {
            let label = self
                .sessions
                .iter()
                .find(|s| s.id == session_id)
                .map(|s| s.label().to_owned())
                .unwrap_or_default();
            if label.is_empty() {
                self.pending_close_session = None;
            } else {
                let mut confirm = false;
                let mut cancel = false;
                let resp =
                    egui::Modal::new(egui::Id::new("confirm-close-agent-modal")).show(ctx, |ui| {
                        ui.set_width((ctx.content_rect().width() - 48.0).clamp(240.0, 420.0));
                        ui.heading(tr("Close running agent?"));
                        ui.add_space(4.0);
                        ui.label(label);
                        ui.label(tr(
                            "This pane is still running an agent session. Closing it will stop the session.",
                        ));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button(tr("Cancel")).clicked() {
                                cancel = true;
                            }
                            let danger = ui.visuals().error_fg_color;
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(tr("Close pane"))
                                            .color(egui::Color32::WHITE)
                                            .strong(),
                                    )
                                    .fill(danger),
                                )
                                .clicked()
                            {
                                confirm = true;
                            }
                        });
                    });
                if cancel || resp.should_close() {
                    self.pending_close_session = None;
                } else if confirm {
                    self.pending_close_session = None;
                    self.close_session_immediate(session_id);
                }
            }
        }
        if let Some(project_id) = self.pending_remove_project {
            let info = self
                .state
                .projects
                .iter()
                .find(|p| p.id == project_id)
                .map(|p| format!("{} — {}", p.name, p.path.display()));
            if let Some(info_line) = info {
                let mut confirm = false;
                let mut cancel = false;
                let resp = egui::Modal::new(egui::Id::new("confirm-remove-project-modal"))
                    .show(ctx, |ui| {
                        ui.set_width((ctx.content_rect().width() - 48.0).clamp(240.0, 440.0));
                        ui.heading(tr("Remove workspace from sidebar?"));
                        ui.add_space(4.0);
                        ui.label(info_line);
                        ui.label(tr(
                            "Removes this workspace and closes its open panes in Tessel. Files on disk are untouched.",
                        ));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button(tr("Cancel")).clicked() {
                                cancel = true;
                            }
                            let danger = ui.visuals().error_fg_color;
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(tr("Remove workspace"))
                                            .color(egui::Color32::WHITE)
                                            .strong(),
                                    )
                                    .fill(danger),
                                )
                                .clicked()
                            {
                                confirm = true;
                            }
                        });
                    });
                if cancel || resp.should_close() {
                    self.pending_remove_project = None;
                } else if confirm {
                    self.pending_remove_project = None;
                    self.remove_project_immediate(project_id);
                }
            } else {
                self.pending_remove_project = None;
            }
        }
        if self.pending_exit {
            let model_busy = self.model_discovery.as_ref().is_some_and(|d| d.active());
            egui::Modal::new(egui::Id::new("close-app")).show(ctx, |ui| {
                ui.label(tr(if self.native_exit_pending() {
                    "There are unsaved files or sessions. Local sessions end; native remote terminals remain on the host."
                } else {
                    "There are unsaved files or sessions. Closing ends sessions."
                }));
                if ui.button(tr("Keep working")).clicked() {
                    self.pending_exit = false;
                    self.floating.cancel_exit();
                }
                if self.floating.exit_pending { ui.label(tr("Closing the floating terminal…")); }
                if self.floating.exit_failed {
                    ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
                    ui.label(tr("Quit without waiting for floating terminal shutdown."));
                    if ui.add_enabled(!self.exit_background_busy(), egui::Button::new(tr("Quit anyway"))).clicked() {
                        self.floating.force_exit = true;
                        self.finish_exit(ctx);
                    }
                }
                if self.file_action_pending { ui.label(tr("File operation in progress…")); }
                if self.account_busy() { ui.label(tr("Wait for the account request to finish before closing.")); }
                if model_busy { ui.label(if self.state.language == crate::i18n::Language::French { "Attendez la fin de la recherche et de la sauvegarde des modèles avant de fermer." } else { "Wait for model discovery and saving to finish before closing." }); }
                if self.remote.needs_close_confirmation() {ui.label(tr("Finish remote requests and save remote edits in Remote files before saving and closing."));}
                if ui.add_enabled(!self.floating.exit_pending && !model_busy && !self.remote.needs_close_confirmation() && !self.account_busy() && !self.file_action_pending && !self.transcript_trash_busy,egui::Button::new(tr("Save all and close"))).clicked() {
                    match self
                        .documents
                        .iter_mut()
                        .filter(|d| d.dirty())
                        .try_for_each(|doc| doc.save(self.store.as_ref().map(|s| s.directory())))
                    {
                        Ok(()) => {
                            self.finish_exit(ctx);
                        }
                        Err(e) => self.error = e,
                    }
                }
                if ui.add_enabled(!self.floating.exit_pending && !model_busy && !self.remote.busy() && !self.account_busy() && !self.file_action_pending && !self.transcript_trash_busy,egui::Button::new(tr("Discard edits and close"))).clicked() {
                    self.finish_exit(ctx);
                }
            });
        }
    }
    fn finish_exit(&mut self, ctx: &egui::Context) {
        if !self.drain_floating_for_exit() {
            return;
        }
        #[cfg(windows)]
        self.begin_native_exit(ctx);
        #[cfg(not(windows))]
        {
            self.save();
            self.allow_exit = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    fn exit_background_busy(&self) -> bool {
        self.model_discovery.as_ref().is_some_and(|d| d.active())
            || self.remote.busy()
            || self.account_busy()
            || self.file_action_pending
            || self.transcript_trash_busy
    }
    #[cfg(not(windows))]
    fn native_modal_open(&self) -> bool {
        false
    }

    fn native_exit_pending(&self) -> bool {
        #[cfg(windows)]
        {
            self.native_terminals
                .runtime
                .as_ref()
                .is_some_and(|runtime| {
                    runtime.closing()
                        || self
                            .sessions
                            .iter()
                            .any(|session| session.native_host.is_some())
                        || runtime.openings().count() > 0
                        || !runtime.keeper().is_idle()
                        || !runtime.desk().unsettled().is_empty()
                })
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
    fn native_terminals_not_closing(&self) -> bool {
        #[cfg(windows)]
        {
            !self
                .native_terminals
                .runtime
                .as_ref()
                .is_some_and(crate::terminal::host_runtime::Runtime::closing)
        }
        #[cfg(not(windows))]
        {
            true
        }
    }
    fn sync_git_project(&mut self) {
        self.gpui_extra.scm_drafts.retain(|owner| {
            self.state
                .projects
                .iter()
                .any(|p| p.id == owner.project && p.path == owner.root)
        });
        let owner = self.state.active().map(|project| crate::git_result::Owner {
            project: project.id,
            root: project.path.clone(),
        });
        if let Some(owner) = &owner {
            self.gpui_extra.scm_drafts.ensure(owner);
        }
        if self.scm_state.select(owner.clone()) {
            self.scm_panel.reset();
            self.pending_scm_discard = None;
            self.commit_in_flight = false;
        }
        let _ = self.git_result.select(owner);
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
        use crate::scm_changes_panel::{
            ScmArea, ScmCapabilities, ScmChangesAction, ScmChangesInput, ScmStatusSnapshot,
            discard_area_copy,
        };
        let french = self.state.language == crate::i18n::Language::French;
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
        let active_rel_entry: Option<(ScmArea, String)> = if self.view == View::Files
            && let Some(doc) = self.documents.get(self.active_document)
            && let Some(project) = self.state.active()
            && let Ok(rel) = doc.path.strip_prefix(&project.path)
        {
            let rel_norm = rel.to_string_lossy().replace('\\', "/");
            let area = if doc.diff_area == Some(DiffTabArea::Staged) {
                ScmArea::Staged
            } else if status.is_some_and(|st| {
                st.entries
                    .iter()
                    .any(|e| e.area == ScmArea::Untracked && e.path == rel_norm)
            }) {
                ScmArea::Untracked
            } else {
                ScmArea::Unstaged
            };
            Some((area, rel_norm))
        } else {
            None
        };
        self.scm_panel
            .set_group_order(self.state.git_prefs.scm_group_order.into_panel());
        self.scm_panel
            .set_view_mode(self.state.git_prefs.scm_view_mode.into_panel());
        let action = self.scm_panel.show(
            ui,
            ScmChangesInput {
                project_selected: self.state.active().is_some(),
                loaded: self.scm_state.result.is_some(),
                busy: self.scm_state.busy() || self.git_result.busy(),
                read_only: false,
                french,
                error: error.map(String::as_str),
                status: status.map(|parsed| ScmStatusSnapshot::from_parsed(true, parsed)),
                active_entry: active_rel_entry
                    .as_ref()
                    .map(|(area, rel)| (*area, rel.as_str())),
                capabilities: ScmCapabilities {
                    can_refresh: true,
                    can_open_diff: true,
                    can_open_file: true,
                    can_stage: true,
                    can_unstage: true,
                    can_discard: true,
                    can_commit: true,
                    can_remote: true,
                },
            },
        );
        match action {
            Some(ScmChangesAction::Refresh) if !self.git_result.busy() => self.scm_state.refresh(),
            Some(ScmChangesAction::OpenDiff {
                area,
                path,
                old_path,
                pinned,
            }) => {
                self.open_scm_diff(area, path, old_path, pinned);
            }
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
            Some(ScmChangesAction::RequestDiscard { area, paths }) if !self.git_result.busy() => {
                if !paths.is_empty() {
                    let copy = if paths.len() == 1
                        && let Some(st) = status
                        && let Some(entry) = st
                            .entries
                            .iter()
                            .find(|e| e.area == area && e.path == paths[0])
                    {
                        entry.discard_copy(false, french)
                    } else {
                        discard_area_copy(area, paths.len(), false, french)
                    };
                    self.pending_scm_discard = Some((area, paths, copy));
                }
            }
            Some(ScmChangesAction::Commit { message }) if !self.git_result.busy() => {
                if let Err(error) = self.scm_state.validate_commit() {
                    self.error = error;
                } else {
                    self.commit_in_flight = true;
                    self.git_start_work(move |root| crate::git::commit_staged(&root, &message));
                }
            }
            Some(ScmChangesAction::Remote(op)) if !self.git_result.busy() => {
                let branch = self.scm_state.branch_name();
                self.git_start_work(move |root| {
                    crate::git::remote_op(&root, op, branch.as_deref())
                });
            }
            Some(ScmChangesAction::OpenFile(path)) => {
                if let Some(path) = self.scm_state.file_path(&path) {
                    self.open_document_preview(path);
                }
            }
            _ => {}
        }
        let new_order = workspace::ScmOrderSetting::from_panel(self.scm_panel.group_order());
        let new_mode = workspace::ScmViewModeSetting::from_panel(self.scm_panel.view_mode());
        if self.state.git_prefs.scm_group_order != new_order
            || self.state.git_prefs.scm_view_mode != new_mode
        {
            self.state.git_prefs.scm_group_order = new_order;
            self.state.git_prefs.scm_view_mode = new_mode;
            self.save();
        }
        if self.scm_commits_project != self.state.active_project {
            self.scm_commits_project = self.state.active_project;
            self.refresh_scm_commits();
        }
        if self.state.active().is_some()
            && let Some(hist_act) = crate::side_panels_ui::show_scm_history_section(
                ui,
                &mut self.side_panels_ui,
                &self.scm_commits_cache,
                french,
            )
        {
            self.handle_side_panels_action(ui.ctx(), hist_act);
        }
    }

    fn refresh_scm_commits(&mut self) {
        self.side_panels_ui.expanded_commits.clear();
        self.side_panels_ui.commit_files_cache.clear();
        if let Some(proj) = self.state.active() {
            self.scm_commits_cache = crate::side_panels_model::load_git_commit_history(
                &proj.path,
                crate::side_panels_model::DEFAULT_COMMIT_LIMIT,
            )
            .unwrap_or_default();
        } else {
            self.scm_commits_cache.clear();
        }
    }

    fn build_dashboard_items(&self) -> Vec<crate::side_panels_model::DashboardItem> {
        use crate::side_panels_model::{DashboardItem, DashboardState};
        let mut out = Vec::with_capacity(self.sessions.len());
        for s in &self.sessions {
            let project_name = self
                .state
                .projects
                .iter()
                .find(|p| Some(p.id) == s.project)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            let is_chat = s.launch.native_chat().is_some();
            let (agent_kind, state) = match &s.launch {
                Launch::CodexChat => {
                    let chat = self.chats.iter().find(|c| c.pane == s.id);
                    let st = if chat.is_some_and(crate::chat::Chat::waiting_for_user) {
                        DashboardState::Waiting
                    } else if chat.is_some_and(crate::chat::Chat::working) {
                        DashboardState::Working
                    } else {
                        DashboardState::Idle
                    };
                    ("Codex Chat".to_owned(), st)
                }
                Launch::ClaudeChat => {
                    let chat = self.claude_chats.iter().find(|c| c.pane == s.id);
                    let st = if chat.is_some_and(crate::claude::Chat::waiting_for_user) {
                        DashboardState::Waiting
                    } else if chat.is_some_and(crate::claude::Chat::working) {
                        DashboardState::Working
                    } else {
                        DashboardState::Idle
                    };
                    ("Claude Chat".to_owned(), st)
                }
                Launch::Agent(agent) => {
                    let sleeping = s.terminal.is_none()
                        && s.loading.is_none()
                        && s.profile_migration.is_none()
                        && s.restarting.is_none();
                    let st = if sleeping {
                        DashboardState::Sleeping
                    } else if s.terminal.as_ref().is_some_and(Terminal::exited) {
                        DashboardState::Done
                    } else {
                        DashboardState::Idle
                    };
                    (agent.clone(), st)
                }
                Launch::Shell => {
                    let sleeping = s.terminal.is_none()
                        && s.loading.is_none()
                        && s.profile_migration.is_none()
                        && s.restarting.is_none();
                    let st = if sleeping {
                        DashboardState::Sleeping
                    } else if s.terminal.as_ref().is_some_and(Terminal::exited) {
                        DashboardState::Done
                    } else {
                        DashboardState::Idle
                    };
                    ("Terminal".to_owned(), st)
                }
                Launch::Ssh(host) => {
                    let sleeping = s.terminal.is_none()
                        && s.loading.is_none()
                        && s.profile_migration.is_none()
                        && s.restarting.is_none();
                    let st = if sleeping {
                        DashboardState::Sleeping
                    } else {
                        DashboardState::Idle
                    };
                    (format!("SSH ({})", host.alias), st)
                }
            };
            let branch = s
                .project
                .and_then(|pid| self.worktree_state.summary(pid))
                .map(|sum| sum.branch.clone());
            out.push(DashboardItem {
                pane_id: s.id,
                project_id: s.project,
                project_name,
                title: s.label().to_owned(),
                agent_kind,
                branch,
                task_line: None,
                state,
                is_chat,
                is_focused: self.active_session == Some(s.id),
            });
        }
        out
    }

    fn dashboard_view(&mut self, ui: &mut egui::Ui) {
        let items = self.build_dashboard_items();
        let french = self.state.language == crate::i18n::Language::French;
        if let Some(act) = crate::side_panels_ui::show_dashboard_panel(
            ui,
            &mut self.side_panels_ui,
            &items,
            french,
        ) {
            self.handle_side_panels_action(ui.ctx(), act);
        }
    }

    fn task_history_view(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        if let Some(act) = crate::side_panels_ui::show_task_history_panel(
            ui,
            &mut self.side_panels_ui,
            &self.state.task_history.records,
            self.state.active_project,
            french,
        ) {
            self.handle_side_panels_action(ui.ctx(), act);
        }
    }

    fn session_history_sources(&self) -> Vec<crate::side_panels_model::SessionSource> {
        use crate::session_profile::{Binding, Context};
        let Some(home) = directories::BaseDirs::new() else {
            return Vec::new();
        };
        let data_dir = self.store.as_ref().map(|store| store.directory());
        let mut sources = Vec::new();
        for agent in ["claude", "codex"] {
            let prefs = self
                .state
                .agent_prefs
                .get(agent)
                .cloned()
                .unwrap_or_default();
            let roster = if agent == "claude" {
                &self.claude_managed_accounts
            } else {
                &self.codex_managed_accounts
            };
            let inherited =
                std::env::var(crate::session_profile::env_key(agent).expect("provider")).ok();
            let cwd = self
                .state
                .active()
                .map(|project| project.path.as_path())
                .unwrap_or(home.home_dir());
            let context = Context {
                agent,
                prefs: &prefs,
                cwd,
                home: home.home_dir(),
                data_dir,
                roster,
                inherited: inherited.as_deref(),
            };
            let explicit = crate::agent_env::parse(&prefs.env).ok().is_some_and(|env| {
                crate::agent_env::effective_value(
                    &env,
                    crate::session_profile::env_key(agent).expect("provider"),
                )
                .is_some()
            });
            let current = if !explicit && let Some(account_id) = &roster.selected_id {
                Binding::Managed {
                    account_id: account_id.clone(),
                }
            } else {
                match context.capture() {
                    Ok(binding) => binding,
                    Err(_) => continue,
                }
            };
            let mut bindings = vec![current];
            if agent == "codex" {
                bindings.push(Binding::Default);
                bindings.extend(
                    roster
                        .accounts
                        .iter()
                        .take(100)
                        .map(|account| Binding::Managed {
                            account_id: account.id.clone(),
                        }),
                );
            }
            bindings.sort();
            bindings.dedup();
            for binding in bindings {
                if let Ok(root) = context.resolve(&binding) {
                    sources.push(crate::side_panels_model::SessionSource {
                        label: match &binding {
                            Binding::Managed { account_id } => roster
                                .accounts
                                .iter()
                                .find(|account| &account.id == account_id)
                                .map(|account| account.label.clone())
                                .unwrap_or_default(),
                            _ => String::new(),
                        },
                        agent_id: agent.into(),
                        root,
                        binding,
                    });
                }
            }
        }
        sources
    }

    fn refresh_past_sessions(&mut self) {
        self.gpui_extra.session_query.invalidate();
        let live: Vec<crate::side_panels_model::SessionHistoryEntry> = self
            .sessions
            .iter()
            .filter_map(|s| {
                let agent_id = match &s.launch {
                    Launch::Agent(a) => a.clone(),
                    Launch::CodexChat => "codex".into(),
                    Launch::ClaudeChat => "claude".into(),
                    _ => return None,
                };
                Some(crate::side_panels_model::SessionHistoryEntry {
                    profile_label: String::new(),
                    profile_binding: s.profile_binding.clone(),
                    id: s
                        .conversation_id
                        .clone()
                        .unwrap_or_else(|| format!("pane-{}", s.id)),
                    agent_id,
                    title: s.label().to_owned(),
                    folder: s.cwd.clone(),
                    updated_at_ms: crate::side_panels_model::now_unix_ms(),
                    file_path: None,
                    open_pane_id: Some(s.id),
                    transcript_snippet: None,
                })
            })
            .collect();
        let sources = self.session_history_sources();
        self.gpui_extra.session_discovery.request_sources(
            sources,
            live,
            self.side_panels_ui.sessions_limit,
        );
        self.side_panels_ui.sessions_loading = true;
    }

    fn session_history_view(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let active_root = self.state.active().map(|p| p.path.clone());
        let ws_roots: Vec<PathBuf> = self.state.projects.iter().map(|p| p.path.clone()).collect();
        if let Some(act) = crate::side_panels_ui::show_session_history_panel(
            ui,
            &mut self.side_panels_ui,
            &self.past_sessions_cache,
            active_root.as_deref(),
            &ws_roots,
            french,
        ) {
            self.handle_side_panels_action(ui.ctx(), act);
        }
    }

    pub(crate) fn conversation_preview(
        &mut self,
    ) -> std::sync::Arc<[crate::side_panels_model::ActivityMessage]> {
        let mut inputs = Vec::new();
        let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf());
        let inherited_cfg = std::env::var("CLAUDE_CONFIG_DIR").ok();
        if let Some(home) = home.as_deref() {
            for session in &self.sessions {
                let Launch::Agent(agent_id) = &session.launch else {
                    continue;
                };
                if !crate::claude_terminal::supported(agent_id) {
                    continue;
                }
                let Some(conversation) = session.conversation_id.as_ref() else {
                    continue;
                };
                let prefs = self
                    .state
                    .agent_prefs
                    .get(agent_id)
                    .cloned()
                    .unwrap_or_default();
                let root = session.profile_root.clone().or_else(|| {
                    let binding = session.profile_binding.as_ref()?;
                    let context = crate::session_profile::Context {
                        agent: agent_id,
                        prefs: &prefs,
                        cwd: &session.cwd,
                        home,
                        data_dir: session.profile_data_dir.as_deref(),
                        roster: &self.claude_managed_accounts,
                        inherited: inherited_cfg.as_deref(),
                    };
                    context.resolve(binding).ok()
                });
                let Some(root) = root else {
                    continue;
                };
                inputs.push(crate::conversation_loader::Input {
                    root,
                    conversation: conversation.clone(),
                    pane: session.id,
                    title: session.label().to_owned(),
                });
            }
        }
        self.gpui_extra
            .conversation_loader
            .update(inputs, std::time::Instant::now())
    }

    fn conversations_view(&mut self, ui: &mut egui::Ui) {
        let french = self.state.language == crate::i18n::Language::French;
        let preview = self.conversation_preview();
        let mut messages = self.activity_store.messages.clone();
        messages.extend(preview.iter().cloned());
        if self.gpui_extra.conversation_loader.loading() {
            ui.label(if french {
                "Chargement des messages récents…"
            } else {
                "Loading recent messages…"
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(2));
        }
        if self.gpui_extra.conversation_loader.limited {
            ui.label(if french {
                "Aperçu partiel : limite atteinte ou fichiers indisponibles."
            } else {
                "Partial preview: limit reached or unavailable files."
            });
        }

        if let Some(act) = crate::side_panels_ui::show_conversations_panel(
            ui,
            &mut self.side_panels_ui,
            &messages,
            french,
        ) {
            self.handle_side_panels_action(ui.ctx(), act);
        }
    }

    fn open_notes_modal(&mut self) {
        let root = self.state.active().map(|p| p.path.clone());
        if let Err(e) = self.side_panels_ui.notes.load_project(root.as_deref()) {
            self.error = e;
            return;
        }
        self.side_panels_ui.notes_modal_open = true;
    }

    fn review_candidate_branches(&self) -> Vec<(String, Option<PathBuf>)> {
        let Some(pid) = self.state.active_project else {
            return Vec::new();
        };
        let Some(summary) = self.worktree_state.summary(pid) else {
            return Vec::new();
        };
        summary
            .worktrees
            .iter()
            .filter(|w| !w.is_primary && !w.branch.is_empty())
            .map(|w| (w.branch.clone(), Some(PathBuf::from(&w.path))))
            .collect()
    }

    fn open_review_modal(&mut self) {
        let candidates = self.review_candidate_branches();
        self.side_panels_ui.review_modal_open = true;
        if let Some((first_br, first_wt)) = candidates.first().cloned() {
            self.load_review_for_branch(first_br, first_wt);
        } else {
            self.side_panels_ui.review_snapshot = None;
            self.side_panels_ui.review_error = None;
        }
    }

    fn load_review_for_branch(&mut self, head_branch: String, worktree_path: Option<PathBuf>) {
        let Some(proj) = self.state.active() else {
            return;
        };
        let base_branch = self
            .worktree_state
            .summary(proj.id)
            .map(|s| s.default_branch.clone())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "main".to_owned());
        self.side_panels_ui.review_head_branch = head_branch.clone();
        self.side_panels_ui.review_worktree_path = worktree_path.clone();
        match crate::side_panels_model::load_branch_review(
            &proj.path,
            &base_branch,
            &head_branch,
            worktree_path.as_deref(),
        ) {
            Ok(snap) => {
                let first_file = snap.files.first().map(|f| f.path.clone());
                let merge_base = snap.merge_base.clone();
                self.side_panels_ui.review_snapshot = Some(snap);
                self.side_panels_ui.review_error = None;
                if let Some(fp) = first_file {
                    self.side_panels_ui.review_selected_diff =
                        crate::side_panels_model::load_review_file_diff(
                            &proj.path,
                            &merge_base,
                            &head_branch,
                            &fp,
                        )
                        .unwrap_or_default();
                    self.side_panels_ui.review_selected_file = Some(fp);
                } else {
                    self.side_panels_ui.review_selected_file = None;
                    self.side_panels_ui.review_selected_diff.clear();
                }
            }
            Err(err) => {
                self.side_panels_ui.review_snapshot = None;
                self.side_panels_ui.review_error = Some(err);
            }
        }
    }

    fn handle_side_panels_action(
        &mut self,
        ctx: &egui::Context,
        action: crate::side_panels_ui::SidePanelsAction,
    ) {
        use crate::side_panels_ui::SidePanelsAction;
        match action {
            SidePanelsAction::FocusPane(pane_id) => {
                self.reveal_workspace_pane(pane_id);
                self.side_panels_ui.sessions_modal_open = false;
            }
            SidePanelsAction::SleepPanes(ids) => {
                for id in ids {
                    if let Some(sess) = self.sessions.iter_mut().find(|s| s.id == id)
                        && sess.launch.native_chat().is_none()
                    {
                        sess.profile_migration = None;
                        sess.loading = None;
                        sess.terminal = None;
                    }
                }
                self.save();
            }
            SidePanelsAction::RestartPane(pane_id) => {
                if let Some(session) = self.sessions.iter_mut().find(|s| s.id == pane_id) {
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
            }
            SidePanelsAction::ClosePane(pane_id) => {
                self.close_session(pane_id);
            }
            SidePanelsAction::ExportTaskHistoryCsv(csv) => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name("tessel-task-history.csv")
                    .add_filter("CSV", &["csv"])
                    .save_file()
                    && let Err(e) = std::fs::write(path, csv.as_bytes())
                {
                    self.error = e.to_string();
                }
            }
            SidePanelsAction::ClearTaskHistory => {
                self.capture_runtime_state();
                if let Err(error) = crate::task_history::clear(&mut self.state, self.store.as_ref())
                {
                    self.error = error;
                }
            }
            SidePanelsAction::RefreshSessions => {
                self.refresh_past_sessions();
            }
            SidePanelsAction::ResumeSession {
                agent_id,
                session_id,
                folder,
                profile_binding,
            } => {
                if self.transcript_trash_busy {
                    self.error = "Wait for the conversation recycling operation to finish.".into();
                    return;
                }
                let entry = self
                    .past_sessions_cache
                    .iter()
                    .find(|entry| {
                        entry.agent_id == agent_id
                            && entry.id == session_id
                            && entry.folder == folder
                            && entry.profile_binding == profile_binding
                            && entry.file_path.is_some()
                            && entry.open_pane_id.is_none()
                    })
                    .cloned();
                let Some(entry) = entry.filter(|entry| {
                    entry.profile_binding.is_some()
                        && crate::claude::valid_uuid(&entry.id)
                        && entry.folder.is_absolute()
                }) else {
                    self.error = "This saved conversation is no longer available; refresh the history before resuming.".into();
                    return;
                };
                let Some(project) = self.state.active_project else {
                    self.error = "Select a project before resuming a saved conversation.".into();
                    return;
                };
                self.side_panels_ui.sessions_modal_open = false;
                let _ = self.new_session_with_history(
                    Some(project),
                    Launch::Agent(agent_id),
                    None,
                    false,
                    None,
                    Some(&entry),
                );
            }
            SidePanelsAction::DeleteSession(entry) => {
                if self.transcript_trash_busy || self.pending_transcript_trash.is_some() {
                    return;
                }
                match self.transcript_source_for_trash(&entry) {
                    Ok(_) => self.pending_transcript_trash = Some(entry),
                    Err(error) => self.error = error,
                }
            }
            SidePanelsAction::CopyText(text) => {
                ctx.copy_text(text);
            }
            SidePanelsAction::RefreshCommits => {
                self.refresh_scm_commits();
            }
            SidePanelsAction::ToggleExpandCommit(hash) => {
                if !self.side_panels_ui.expanded_commits.remove(&hash) {
                    if !self.side_panels_ui.commit_files_cache.contains_key(&hash)
                        && let Some(proj) = self.state.active()
                        && let Ok(files) =
                            crate::side_panels_model::load_commit_files(&proj.path, &hash)
                    {
                        self.side_panels_ui
                            .commit_files_cache
                            .insert(hash.clone(), files);
                    }
                    self.side_panels_ui.expanded_commits.insert(hash);
                }
            }
            SidePanelsAction::OpenCommitDiff {
                commit_hash,
                path,
                pinned,
            } => {
                if let Some(proj) = self.state.active().cloned() {
                    let full_path = proj.path.join(&path);
                    let root = proj.path.clone();
                    let sender = self.sender.clone();
                    thread::spawn(move || {
                        let result = crate::side_panels_model::load_commit_file_diff(
                            &root,
                            &commit_hash,
                            &path,
                        );
                        let _ = sender.send(Message::DiffDocument {
                            source_doc: None,
                            root,
                            path: full_path,
                            area: DiffTabArea::Commit {
                                commit: commit_hash,
                            },
                            pinned,
                            result,
                        });
                    });
                }
            }
            SidePanelsAction::SaveNotes { force } => {
                if let Err(e) = self.side_panels_ui.notes.save(force) {
                    self.error = e;
                }
            }
            SidePanelsAction::OpenNotesInEditor => {
                if let Some(proj) = self.state.active().cloned() {
                    let _ = self.side_panels_ui.notes.save(false);
                    let notes_path = crate::side_panels_model::NotesModel::notes_path(&proj.path);
                    if !notes_path.exists() {
                        if let Some(parent) = notes_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::write(&notes_path, &self.side_panels_ui.notes.text);
                    }
                    self.side_panels_ui.notes_modal_open = false;
                    self.open_document_at(proj.path, notes_path);
                }
            }
            SidePanelsAction::SendPromptToActiveAgent(prompt) => {
                if let Some(pane_id) = self.active_project_work_pane() {
                    if let Some(sess) = self.sessions.iter_mut().find(|s| s.id == pane_id)
                        && let Some(term) = &mut sess.terminal
                    {
                        term.paste_text(&prompt);
                    }
                    self.activity_store
                        .push(crate::side_panels_model::ActivityMessage {
                            id: 0,
                            thread_key: format!("you:{pane_id}"),
                            thread_title: format!("You & Pane #{pane_id}"),
                            pane_id: Some(pane_id),
                            sender: "You".into(),
                            recipient: format!("Pane #{pane_id}"),
                            body: prompt,
                            timestamp_ms: crate::side_panels_model::now_unix_ms(),
                        });
                    if let Some(st) = &self.store {
                        let _ = self.activity_store.save(st.directory());
                    }
                    self.side_panels_ui.notes_modal_open = false;
                    self.side_panels_ui.review_modal_open = false;
                    self.view = View::Work;
                }
            }
            SidePanelsAction::CloseNotesModal => {
                self.side_panels_ui.notes_modal_open = false;
            }
            SidePanelsAction::CloseSessionsModal => {
                self.side_panels_ui.sessions_modal_open = false;
            }
            SidePanelsAction::SelectReviewBranch {
                head_branch,
                worktree_path,
            } => {
                self.load_review_for_branch(head_branch, worktree_path);
            }
            SidePanelsAction::RefreshReview => {
                let br = self.side_panels_ui.review_head_branch.clone();
                let wt = self.side_panels_ui.review_worktree_path.clone();
                if !br.is_empty() {
                    self.load_review_for_branch(br, wt);
                }
            }
            SidePanelsAction::SelectReviewFile(rel_path) => {
                if let (Some(proj), Some(snap)) =
                    (self.state.active(), &self.side_panels_ui.review_snapshot)
                {
                    self.side_panels_ui.review_selected_diff =
                        crate::side_panels_model::load_review_file_diff(
                            &proj.path,
                            &snap.merge_base,
                            &snap.head_branch,
                            &rel_path,
                        )
                        .unwrap_or_default();
                    self.side_panels_ui.review_selected_file = Some(rel_path);
                }
            }
            SidePanelsAction::OpenReviewDiffInTab(rel_path) => {
                if let (Some(proj), Some(snap)) = (
                    self.state.active().cloned(),
                    self.side_panels_ui.review_snapshot.clone(),
                ) {
                    let full_path = proj.path.join(&rel_path);
                    let area = DiffTabArea::Branch {
                        base: snap.merge_base.clone(),
                        commit: snap.head_oid.clone(),
                    };
                    let root = proj.path;
                    let sender = self.sender.clone();
                    thread::spawn(move || {
                        let result = crate::side_panels_model::load_review_file_diff(
                            &root,
                            &snap.merge_base,
                            &snap.head_oid,
                            &rel_path,
                        );
                        let _ = sender.send(Message::DiffDocument {
                            source_doc: None,
                            root,
                            path: full_path,
                            area,
                            pinned: true,
                            result,
                        });
                    });
                    self.side_panels_ui.review_modal_open = false;
                }
            }
            SidePanelsAction::PushReviewBranch(branch) => {
                self.git_start_work(move |root| {
                    crate::git::run(&root, &["push", "-u", "origin", &branch], None)
                });
            }
            SidePanelsAction::MergeReviewBranch {
                base_branch,
                head_branch,
                expect_head_oid,
            } => {
                if let Some(proj) = self.state.active().cloned() {
                    match crate::side_panels_model::merge_review_branch(
                        &proj.path,
                        &base_branch,
                        &head_branch,
                        &expect_head_oid,
                    ) {
                        Ok(_) => {
                            self.side_panels_ui.review_modal_open = false;
                            self.scm_state.refresh();
                            self.refresh_scm_commits();
                        }
                        Err(e) => {
                            self.side_panels_ui.review_error = Some(e);
                        }
                    }
                }
            }
            SidePanelsAction::CloseReviewModal => {
                self.side_panels_ui.review_modal_open = false;
            }
        }
    }

    fn open_add_project_modal(&mut self) {
        self.dialogs_toolbar_ui.add_project_open = true;
        self.dialogs_toolbar_ui.add_error.clear();
    }

    pub(crate) fn open_linear_modal(&mut self) {
        if self.linear.view.open {
            return;
        }
        self.gpui_extra.layout_menu_right = None;
        self.gpui_extra.issues_menu_open = false;
        self.command_palette.open = false;
        self.toolbar_menu_open = false;
        self.gpui_extra.toolbar_more_open = false;
        self.gpui_extra.launch_menu_open = false;
        let path = self
            .store
            .as_ref()
            .map(|store| store.directory().join("linear.dat"));
        self.linear.open(path);
        self.configure_linear_task();
    }

    fn open_github_modal(&mut self, project_id: Option<u64>) {
        self.gpui_extra.issues_menu_open = false;
        if let Some(pid) = project_id
            && self.state.projects.iter().any(|p| p.id == pid)
            && self.state.active_project != Some(pid)
        {
            self.state.active_project = Some(pid);
            self.refresh_project();
        }
        self.dialogs_toolbar_ui.github_open = true;
        // A freshly opened dialog must not show a previous project's data if no project
        // is selected or a new repository request fails.
        self.dialogs_toolbar_ui.github_prs.clear();
        self.dialogs_toolbar_ui.github_issues.clear();
        self.dialogs_toolbar_ui.github_query.clear();
        self.refresh_github_modal();
    }

    fn refresh_github_modal(&mut self) {
        let Some(proj) = self.state.active().cloned() else {
            self.dialogs_toolbar_ui.github_error = "Select a project first.".into();
            return;
        };
        self.dialogs_toolbar_ui.github_error.clear();
        match crate::dialogs_toolbar_model::load_gh_prs(&proj.path, 40) {
            Ok(prs) => self.dialogs_toolbar_ui.github_prs = prs,
            Err(e) => {
                self.dialogs_toolbar_ui.github_prs.clear();
                self.dialogs_toolbar_ui.github_error = e;
            }
        }
        if let Ok(issues) = crate::dialogs_toolbar_model::load_gh_issues(&proj.path, 40) {
            self.dialogs_toolbar_ui.github_issues = issues;
        }
    }

    fn put_session_to_sleep(&mut self, id: u64) {
        self.cancel_owner_task_pane(id);
        self.team_mcp.revoke(id);
        for worker in &mut self.teams_hub.workers {
            if worker.worker_pane_id == Some(id)
                && matches!(
                    worker.state,
                    crate::teams_relay::WorkerState::Starting
                        | crate::teams_relay::WorkerState::Running
                )
            {
                worker.state = crate::teams_relay::WorkerState::Stopped;
                if matches!(
                    worker.first_turn,
                    crate::teams_relay::WorkerFirstTurn::Pending
                        | crate::teams_relay::WorkerFirstTurn::Connecting
                ) {
                    worker.first_turn = crate::teams_relay::WorkerFirstTurn::Cancelled;
                }
            }
        }
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
            session.terminal = None;
            session.loading = None;
            session.restarting = None;
            session.profile_migration = None;
            session.codex_restore = None;
            session.codex_discovery = None;
        }
        if let Some(chat) = self.chats.iter_mut().find(|c| c.pane == id) {
            chat.gpui_stop();
        }
        if let Some(chat) = self.claude_chats.iter_mut().find(|c| c.pane == id) {
            chat.gpui_stop();
        }
    }

    fn add_multiple_projects(&mut self, paths: Vec<PathBuf>) {
        let mut last_id = None;
        for path in paths {
            match self.state.add_project(&path) {
                Ok(pid) => last_id = Some(pid),
                Err(e) => self.error = e,
            }
        }
        if let Some(project_id) = last_id {
            self.save();
            self.refresh_project();
            if !self.sessions.iter().any(|s| s.project == Some(project_id))
                && let Some(default_id) = self.state.launcher_prefs.remember_default.clone()
            {
                self.launch_empty_state_entry(&default_id);
            }
        }
    }

    fn handle_dialogs_toolbar_action(
        &mut self,
        ctx: &egui::Context,
        action: crate::dialogs_toolbar_ui::DialogsToolbarAction,
    ) {
        use crate::dialogs_toolbar_ui::DialogsToolbarAction;
        match action {
            DialogsToolbarAction::PickLocalProjectFolder => {
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let _ = sender.send(Message::AddProjectPickedFolder(
                        rfd::FileDialog::new()
                            .set_title("Open project or workspace folder")
                            .pick_folder(),
                    ));
                });
            }
            DialogsToolbarAction::PickCloneParentFolder => {
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let _ = sender.send(Message::CloneParentPicked(
                        rfd::FileDialog::new()
                            .set_title("Choose clone parent folder")
                            .pick_folder(),
                    ));
                });
            }
            DialogsToolbarAction::PickInitParentFolder => {
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let _ = sender.send(Message::InitParentPicked(
                        rfd::FileDialog::new()
                            .set_title("Choose parent folder for new project")
                            .pick_folder(),
                    ));
                });
            }
            DialogsToolbarAction::AddProjects(paths) => {
                self.dialogs_toolbar_ui.add_project_open = false;
                self.add_multiple_projects(paths);
            }
            DialogsToolbarAction::CloneRepository {
                url,
                parent_dir,
                folder_name,
            } => match crate::dialogs_toolbar_model::clone_git_repository(
                &url,
                &parent_dir,
                &folder_name,
            ) {
                Ok(dest) => {
                    self.dialogs_toolbar_ui.add_project_open = false;
                    self.dialogs_toolbar_ui.add_error.clear();
                    self.add_multiple_projects(vec![dest]);
                }
                Err(e) => {
                    self.dialogs_toolbar_ui.add_error = e;
                }
            },
            DialogsToolbarAction::InitNewProject {
                parent_dir,
                folder_name,
            } => {
                match crate::dialogs_toolbar_model::init_new_git_project(&parent_dir, &folder_name)
                {
                    Ok(dest) => {
                        self.dialogs_toolbar_ui.add_project_open = false;
                        self.dialogs_toolbar_ui.add_error.clear();
                        self.add_multiple_projects(vec![dest]);
                    }
                    Err(e) => {
                        self.dialogs_toolbar_ui.add_error = e;
                    }
                }
            }
            DialogsToolbarAction::BrowseRemoteDirs {
                host_alias,
                remote_dir,
            } => match crate::dialogs_toolbar_model::list_remote_directories(
                &host_alias,
                &remote_dir,
            ) {
                Ok(entries) => {
                    self.dialogs_toolbar_ui.ssh_remote_entries = entries;
                    self.dialogs_toolbar_ui.add_error.clear();
                }
                Err(e) => {
                    self.dialogs_toolbar_ui.add_error = e;
                }
            },
            DialogsToolbarAction::OpenSshWorkspace {
                host_alias,
                remote_dir,
            } => {
                self.dialogs_toolbar_ui.add_project_open = false;
                self.new_session(Launch::Ssh(workspace::Host {
                    alias: host_alias,
                    folder: remote_dir,
                    ..Default::default()
                }));
            }
            DialogsToolbarAction::CloseAddProjectModal => {
                self.dialogs_toolbar_ui.add_project_open = false;
            }
            DialogsToolbarAction::RefreshGitHub => {
                self.refresh_github_modal();
            }
            DialogsToolbarAction::StartWorktreeFromSlug(slug) => {
                if let Some(pid) = self.state.active_project {
                    self.dialogs_toolbar_ui.github_open = false;
                    self.start_new_worktree_pane(
                        pid,
                        &slug,
                        crate::worktree_state::WorktreeLaunchTarget::ShellNewPane,
                    );
                }
            }
            DialogsToolbarAction::SendPromptToActiveAgent(prompt) => {
                self.dialogs_toolbar_ui.github_open = false;
                let _ = self.paste_into_active_terminal(ctx, &prompt);
            }
            DialogsToolbarAction::OpenExternalUrl(url) => {
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
            DialogsToolbarAction::CloseGitHubModal => {
                self.dialogs_toolbar_ui.github_open = false;
            }
            DialogsToolbarAction::SaveProjectGroup { project_id, group } => {
                if let Some(g) = group {
                    self.state.project_groups.insert(project_id, g);
                } else {
                    self.state.project_groups.remove(&project_id);
                }
                self.dialogs_toolbar_ui.group_modal_project = None;
                self.save();
            }
            DialogsToolbarAction::CloseProjectGroupModal => {
                self.dialogs_toolbar_ui.group_modal_project = None;
            }
            DialogsToolbarAction::OpenStatsSettings => {
                self.settings_section = SettingsSection::Stats;
                self.overlay = Some(Overlay::Settings);
            }
            DialogsToolbarAction::OpenAlertsSettings => {
                self.settings_section = SettingsSection::Alerts;
                self.overlay = Some(Overlay::Settings);
            }
            DialogsToolbarAction::FocusPane(pane_id) => {
                self.reveal_workspace_pane(pane_id);
            }
            DialogsToolbarAction::OpenNotification(id) => {
                self.gpui_extra.notifications_popover_open = false;
                let pane = self
                    .notifications
                    .items
                    .iter()
                    .find(|n| n.id == id)
                    .and_then(|n| n.pane_id);
                self.set_notification_unread(id, false);
                if let Some(pane) = pane {
                    self.reveal_workspace_pane(pane);
                }
            }
            DialogsToolbarAction::SetNotificationUnread(id, unread) => {
                self.set_notification_unread(id, unread);
            }
            DialogsToolbarAction::MarkAllNotificationsRead => {
                self.notifications.mark_all_read();
                self.persist_notifications();
            }
            DialogsToolbarAction::ClearNotifications => {
                self.clear_notifications();
            }
        }
    }

    fn handle_teams_ui_action(&mut self, act: crate::teams_ui::TeamsUiAction) {
        use crate::teams_ui::TeamsUiAction;
        let now = crate::teams_relay::now_ms();
        match act {
            TeamsUiAction::ApproveWorker(wid) => {
                if let Err(e) = self.teams_hub.approve_worker_start(&self.state, &wid, now) {
                    self.error = e;
                }
            }
            TeamsUiAction::RefuseWorker(wid) => {
                let _ = self.teams_hub.refuse_worker_start(&wid);
            }
            TeamsUiAction::ApproveRelayPair { pair, hourly_limit } => {
                let _ = self
                    .teams_hub
                    .resolve_pair_approval(pair, true, hourly_limit, now);
            }
            TeamsUiAction::DeclineRelayPair(pair) => {
                let _ = self.teams_hub.resolve_pair_approval(pair, false, 30, now);
            }
            TeamsUiAction::SendWakeReminder(pane_id) => {
                self.teams_hub.trigger_banner_reminder(pane_id);
            }
            TeamsUiAction::DismissWakeBanner(pane_id) => {
                self.teams_hub.wake_banners.remove(&pane_id);
            }
            TeamsUiAction::CreateTeam { project, name } => {
                let id = match crate::teams_relay::generate_team_id(&self.state.teams) {
                    Ok(id) => id,
                    Err(error) => {
                        self.error = error;
                        return;
                    }
                };
                self.state.teams.push(crate::teams_relay::SavedTeam {
                    id,
                    project,
                    name,
                    lead_pane: None,
                    members: Vec::new(),
                });
                self.save();
            }
            TeamsUiAction::DeleteTeam(tid) => {
                self.state.teams.retain(|t| t.id != tid);
                self.save();
            }
            TeamsUiAction::ToggleMember { team_id, pane_id } => {
                if let Some(team) = self.state.teams.iter_mut().find(|t| t.id == team_id) {
                    if team.members.contains(&pane_id) {
                        team.members.retain(|&m| m != pane_id);
                        if team.lead_pane == Some(pane_id) {
                            team.lead_pane = team.members.first().copied();
                        }
                    } else {
                        team.members.push(pane_id);
                        if team.lead_pane.is_none() {
                            team.lead_pane = Some(pane_id);
                        }
                    }
                    self.save();
                }
            }
            TeamsUiAction::ToggleLead { team_id, pane_id } => {
                if crate::teams_relay::TeamsRelayHub::toggle_team_lead(
                    &mut self.state,
                    &team_id,
                    pane_id,
                ) {
                    self.save();
                }
            }
            TeamsUiAction::StopWorker(wid) => {
                let _ = self.teams_hub.stop_or_release_worker(&wid, false);
            }
            TeamsUiAction::ReleaseWorker(wid) => {
                let _ = self.teams_hub.stop_or_release_worker(&wid, true);
            }
            TeamsUiAction::FocusPane(pane_id) => {
                if let Some(s) = self.sessions.iter().find(|s| s.id == pane_id) {
                    let proj = s.project;
                    if self.state.active_project != proj {
                        self.state.active_project = proj;
                        self.refresh_project();
                    }
                    self.active_session = Some(pane_id);
                    self.view = View::Work;
                }
            }
        }
    }

    fn poll_worker_first_turns(&mut self) -> bool {
        use crate::teams_relay::{WorkerFirstTurn, WorkerState};
        let pending: Vec<_> = self
            .teams_hub
            .workers
            .iter()
            .filter(|w| {
                w.state == WorkerState::Running
                    && w.agent == "codex"
                    && w.mode == crate::settings_extra_model::WorkerMode::Chat
                    && matches!(
                        w.first_turn,
                        WorkerFirstTurn::Pending | WorkerFirstTurn::Connecting
                    )
            })
            .cloned()
            .collect();
        let mut changed = false;
        for worker in pending {
            let Some(pane) = worker.worker_pane_id else {
                continue;
            };
            let context_valid = self
                .state
                .projects
                .iter()
                .any(|p| p.id == worker.frozen.project_id)
                && self.state.teams.iter().any(|t| {
                    t.id == worker.frozen.team_id
                        && t.project == worker.frozen.project_id
                        && t.members.contains(&pane)
                        && t.members.contains(&worker.frozen.coordinator_pane_id)
                })
                && self.sessions.iter().any(|s| {
                    s.id == worker.frozen.coordinator_pane_id
                        && s.project == Some(worker.frozen.project_id)
                        && same_worker_directory(&s.cwd, &worker.frozen.cwd)
                })
                && self.sessions.iter().any(|s| {
                    s.id == pane
                        && matches!(s.launch, Launch::CodexChat)
                        && s.project == Some(worker.frozen.project_id)
                        && same_worker_directory(&s.cwd, &worker.frozen.cwd)
                        && s.worktree
                            .as_ref()
                            .map(|w| &w.path)
                            .or_else(|| {
                                self.state
                                    .projects
                                    .iter()
                                    .find(|p| p.id == worker.frozen.project_id)
                                    .map(|p| &p.path)
                            })
                            .is_some_and(|root| same_worker_directory(root, &worker.frozen.cwd))
                });
            let now = crate::teams_relay::now_ms();
            let started = worker.first_turn_started_at_ms.unwrap_or(now);
            let result = if !context_valid {
                Err("Worker context changed before its first turn.".to_owned())
            } else if now.saturating_sub(started) >= 30_000 {
                Err("Worker first turn timed out; it was not retried.".to_owned())
            } else {
                self.ensure_native_chat_instances();
                match self.chats.iter_mut().find(|c| {
                    c.pane == pane
                        && c.project == worker.frozen.project_id
                        && same_worker_directory(&c.cwd, &worker.frozen.cwd)
                }) {
                    Some(chat) => {
                        chat.begin_worker_turn(worker.first_turn == WorkerFirstTurn::Connecting)
                    }
                    None => Err("Worker chat is unavailable.".into()),
                }
            };
            if let Err(error) = &result
                && let Some(chat) = self.chats.iter_mut().find(|c| {
                    c.pane == pane
                        && c.project == worker.frozen.project_id
                        && same_worker_directory(&c.cwd, &worker.frozen.cwd)
                })
            {
                chat.gpui_stop();
                chat.error = error.clone();
            }
            if let Some(record) = self
                .teams_hub
                .workers
                .iter_mut()
                .find(|r| r.id == worker.id)
            {
                record.first_turn_started_at_ms = Some(started);
                match result {
                    Ok(phase) => {
                        changed |= record.first_turn != phase;
                        record.first_turn = phase;
                        if phase == WorkerFirstTurn::Cancelled {
                            record.summary = Some(
                                "Automatic first turn skipped because this chat was already used."
                                    .into(),
                            );
                        }
                    }
                    Err(error) => {
                        record.first_turn = WorkerFirstTurn::Failed;
                        record.state = WorkerState::Failed;
                        record.summary = Some(error.chars().take(300).collect());
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    fn poll_team_mcp(&mut self) -> bool {
        if !self.team_mcp.enabled() {
            return false;
        }
        let panes = self
            .sessions
            .iter()
            .filter_map(|session| {
                Some(crate::team_mcp_runtime::LivePane {
                    pane: session.id,
                    project: session.project?,
                    cwd: session.cwd.clone(),
                    local: session.native_host.is_none()
                        && matches!(
                            session.launch,
                            Launch::Agent(_) | Launch::CodexChat | Launch::ClaudeChat
                        ),
                    active: session.restarting.is_none()
                        && (session
                            .terminal
                            .as_ref()
                            .is_some_and(|terminal| !terminal.exited())
                            || session.loading.is_some()
                            || self.chats.iter().any(|chat| {
                                chat.pane == session.id
                                    && (chat.process_id().is_some() || chat.starting())
                            })
                            || self.claude_chats.iter().any(|chat| {
                                chat.pane == session.id
                                    && (chat.process_id().is_some() || chat.starting())
                            })),
                })
            })
            .collect::<Vec<_>>();
        // Persisted session data supplies tool presentation, not transport authority.
        self.capture_runtime_state();
        let mcp_changed = self.team_mcp.poll(
            &panes,
            &mut self.state,
            self.store.as_ref(),
            &mut self.teams_hub,
        );
        if mcp_changed {
            self.save();
        }
        mcp_changed
    }

    fn poll_teams_and_relay(&mut self) -> bool {
        // A stop queued before this tick takes precedence over queued MCP work.
        let had_stops = !self.teams_hub.pending_worker_stops.is_empty();
        while let Some(pane) = self.teams_hub.pending_worker_stops.pop_front() {
            self.put_session_to_sleep(pane);
        }
        let mcp_changed = self.poll_team_mcp();
        let readiness_changed = match crate::task_readiness::poll(
            &mut self.state,
            self.store.as_ref(),
            &mut self.teams_hub,
            crate::teams_relay::now_ms(),
        ) {
            Ok(changed) => changed,
            Err(error) => {
                self.error = error;
                false
            }
        };
        let changed = !self.teams_hub.pending_spawns.is_empty()
            || !self.teams_hub.pending_worker_stops.is_empty()
            || mcp_changed
            || readiness_changed
            || had_stops;
        while let Some(pane) = self.teams_hub.pending_worker_stops.pop_front() {
            self.put_session_to_sleep(pane);
        }
        // Bootstrap already-bound workers only. A new pane gets its own chat
        // instance and frozen mailbox before the next tick can send a turn.
        let changed = self.poll_worker_first_turns() || changed;
        let now = crate::teams_relay::now_ms();
        while let Some(spec) = self.teams_hub.pending_spawns.pop_front() {
            let coordinator = self.sessions.iter().find(|s| {
                s.id == spec.coordinator_pane_id
                    && s.project == Some(spec.project_id)
                    && same_worker_directory(&s.cwd, &spec.cwd)
            });
            let worktree = coordinator.and_then(|s| s.worktree.clone());
            let launch_root = worktree.as_ref().map(|wt| &wt.path).or_else(|| {
                self.state
                    .projects
                    .iter()
                    .find(|p| p.id == spec.project_id)
                    .map(|p| &p.path)
            });
            if !self.teams_hub.worker_spawn_is_current(&self.state, &spec)
                || coordinator.is_none()
                || !launch_root.is_some_and(|root| same_worker_directory(root, &spec.cwd))
            {
                self.teams_hub.fail_worker_start(
                    &spec.worker_id,
                    "Worker context changed before launch.",
                    now,
                );
                continue;
            }
            let launch = match spec.mode {
                crate::settings_extra_model::WorkerMode::Chat => match spec.agent.as_str() {
                    "claude" => Launch::ClaudeChat,
                    _ => Launch::CodexChat,
                },
                crate::settings_extra_model::WorkerMode::Terminal => {
                    Launch::Agent(spec.agent.clone())
                }
            };
            // Key creation must succeed before any child can start. The exact
            // pane id remains reserved within this synchronous launch step.
            let reserved_pane = self.next_session;
            if let Err(error) = self.teams_hub.auth.ensure_pane_secret_hex(reserved_pane) {
                self.teams_hub
                    .fail_worker_start(&spec.worker_id, &error, now);
                continue;
            }
            let created = self.new_session_with_history(
                Some(spec.project_id),
                launch,
                None,
                false,
                worktree,
                None,
            );
            if let Some(created_id) = created
                && let Some(spawned) = self.sessions.iter_mut().find(|s| s.id == created_id)
            {
                let spawned_id = spawned.id;
                spawned.pane_name = Some(spec.worker_name.clone());
                self.teams_hub.bind_spawned_worker_pane(
                    &mut self.state,
                    &spec.worker_id,
                    spawned_id,
                    now,
                );
                self.save();
            } else {
                self.teams_hub.auth.revoke_pane(reserved_pane);
                self.teams_hub.fail_worker_start(
                    &spec.worker_id,
                    "Worker pane could not be created.",
                    now,
                );
            }
        }

        let mut candidates = Vec::new();
        for sess in &self.sessions {
            if !matches!(sess.launch, Launch::Agent(_)) {
                continue;
            }
            let busy_or_inactive = match &sess.terminal {
                None => true,
                Some(t) => t.exited() || t.active_within(Duration::from_secs(5)),
            };
            let focused = self.active_session == Some(sess.id);
            if let Some(oldest_ms) = self.teams_hub.oldest_unread_team_ms(sess.id) {
                candidates.push(crate::teams_relay::WakeCandidate {
                    pane_id: sess.id,
                    channel: crate::teams_relay::WakeChannel::Team,
                    oldest_unread_ms: oldest_ms,
                    relay_sender: None,
                    pane_idle_confirmed: !busy_or_inactive && !focused,
                    pane_busy_or_sleeping_or_exited: busy_or_inactive,
                    user_focused_on_pane: focused,
                    last_user_interaction_ms: 0,
                });
            } else if let Some((oldest_ms, sender)) = self.teams_hub.oldest_unread_relay(sess.id) {
                candidates.push(crate::teams_relay::WakeCandidate {
                    pane_id: sess.id,
                    channel: crate::teams_relay::WakeChannel::Relay,
                    oldest_unread_ms: oldest_ms,
                    relay_sender: Some(sender),
                    pane_idle_confirmed: !busy_or_inactive && !focused,
                    pane_busy_or_sleeping_or_exited: busy_or_inactive,
                    user_focused_on_pane: focused,
                    last_user_interaction_ms: 0,
                });
            }
        }
        self.teams_hub
            .poll_idle_wakes(&self.state.orchestration_prefs, &candidates, now);

        while let Some((pane_id, line)) = self.teams_hub.pending_pty_sends.pop_front() {
            if let Some(sess) = self.sessions.iter_mut().find(|s| s.id == pane_id)
                && let Some(term) = &mut sess.terminal
                && !term.exited()
            {
                let _ = term.run_quick_command(&line, true);
            }
        }
        changed
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
                if std::env::args().any(|arg| arg == "--floating-fixture")
                    && !self.floating.capture_ready()
                {
                    self.visual_frames -= 1;
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
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
                    self.finish_exit(ctx);
                }
            }
        }

        self.native_hosts.poll();
        self.poll();
        self.poll_floating(ctx);
        self.sync_live_share();
        #[cfg(windows)]
        {
            self.poll_native_terminals(ctx);
            if self
                .native_terminals
                .panel
                .handle_escape(ctx, self.native_terminals.picker.is_some())
                .is_some()
            {
                self.native_terminals.picker = None;
            }
        }
        let previous_panels = self.state.panels;
        #[cfg(windows)]
        let had_floating_keyboard = self.floating.ui.has_keyboard();
        let terminal_focused = self.floating.terminal_focused(ctx)
            || self.view == View::Work
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
            || self.native_modal_open()
            || self.share.has_pending_ask()
            || self.share.is_pill_popover_open()
            || self.toolbar_menu_open
            || self.pending_close.is_some()
            || self.pending_close_session.is_some()
            || self.pending_remove_project.is_some()
            || self.pending_scm_discard.is_some()
            || self.pending_agent_install.is_some()
            || self.pending_transcript_trash.is_some()
            || self.rename_project_modal.is_some()
            || self.pending_exit
            || self.file_action.is_some()
            || self.restore_preview.is_some();
        self.floating_shortcuts(
            ctx,
            confirmation_open || self.overlay.is_some() || self.command_palette.open,
        );
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
        if self.overlay.is_some() || self.command_palette.open || confirmation_open {
            self.status_bar.close_popovers();
            self.pane_context_menu = None;
        }
        let dropped_paths: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped_paths.is_empty()
            && self.overlay.is_none()
            && !self.command_palette.open
            && !confirmation_open
        {
            let joined = dropped_paths
                .iter()
                .map(|p| shell_quote_path(p))
                .collect::<Vec<_>>()
                .join(" ");
            let _ = self.paste_into_active_terminal(ctx, &joined);
        }
        let picker_escape = self.sessions.iter_mut().fold(false, |consumed, session| {
            session.model_picker.handle_escape(ctx) | consumed
        });
        let status_bar_escape = self.status_bar.has_open_popover()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if status_bar_escape {
            self.status_bar.close_popovers();
        }
        let keyboard_free = !picker_escape
            && !status_bar_escape
            && !self.status_bar.has_open_popover()
            && self.pane_context_menu.is_none()
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
            if !self.command_palette.open {
                // Global zoom & workspace cycling shortcuts work from editor, terminal, or body:
                let zoom_in = ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::CTRL, egui::Key::Equals)
                        || i.consume_key(egui::Modifiers::CTRL, egui::Key::Plus)
                        || i.consume_key(
                            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                            egui::Key::Plus,
                        )
                        || i.consume_key(
                            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                            egui::Key::Equals,
                        )
                });
                let zoom_out =
                    ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Minus));
                let zoom_reset = ctx.input_mut(|i| {
                    if i.consume_key(egui::Modifiers::CTRL, egui::Key::Num0) {
                        return true;
                    }
                    let phys_zero = i.events.iter().any(|ev| {
                        matches!(
                            ev,
                            egui::Event::Key {
                                physical_key: Some(egui::Key::Num0),
                                pressed: true,
                                modifiers,
                                ..
                            } if modifiers.ctrl && !modifiers.shift && !modifiers.alt
                        )
                    });
                    if phys_zero {
                        i.events.retain(|ev| {
                            !matches!(
                                ev,
                                egui::Event::Key {
                                    physical_key: Some(egui::Key::Num0),
                                    pressed: true,
                                    modifiers,
                                    ..
                                } if modifiers.ctrl && !modifiers.shift && !modifiers.alt
                            )
                        });
                    }
                    phys_zero
                });
                if zoom_in {
                    self.zoom_font(1);
                } else if zoom_out {
                    self.zoom_font(-1);
                } else if zoom_reset {
                    self.zoom_font(0);
                }
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::PageUp)) {
                    self.cycle_workspace(-1);
                }
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::PageDown)) {
                    self.cycle_workspace(1);
                }
                if self.view == View::Files && self.documents.len() > 1 {
                    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft))
                    {
                        let len = self.documents.len();
                        self.active_document = (self.active_document + len - 1) % len;
                    } else if ctx
                        .input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowRight))
                    {
                        let len = self.documents.len();
                        self.active_document = (self.active_document + 1) % len;
                    }
                }
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
                if ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::N)
                }) {
                    self.choose_project();
                }
                if !self.floating.ui.has_keyboard() {
                    for (key, dir) in [
                        (egui::Key::E, crate::pane_grid::SplitDir::Row),
                        (egui::Key::O, crate::pane_grid::SplitDir::Col),
                    ] {
                        if ctx.input_mut(|i| {
                            i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, key)
                        }) {
                            self.split_active_shell(dir);
                        }
                    }
                }
                #[cfg(windows)]
                let native_escape = self.sessions.iter().any(|s| {
                    Some(s.id) == self.active_session
                        && s.terminal
                            .as_ref()
                            .is_some_and(Terminal::native_owns_escape)
                });
                #[cfg(not(windows))]
                let native_escape = false;
                if !native_escape
                    && !self.floating.ui.has_keyboard()
                    && ctx.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    if self.right_fullscreen {
                        self.right_fullscreen = false;
                    } else {
                        let key = crate::terminal_layout::project_key(self.state.active_project);
                        if let Some(layout) = self.state.terminal_layouts.get_mut(&key)
                            && layout.maximized.take().is_some()
                        {
                            self.grid_dirty = true;
                        }
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
                    let launch = self.default_new_launch();
                    self.new_session(launch);
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
        if take_fresh_shortcut(
            ctx,
            keyboard_free
                && !confirmation_open
                && !self.command_palette.open
                && self.overlay.is_none()
                && !self.floating.ui.has_keyboard(),
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
            egui::Key::B,
        ) {
            self.state.broadcast = !self.state.broadcast;
            self.save();
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
        let running_sessions_require_confirm = self.state.general_prefs.confirm_quit_running
            && (!self.sessions.is_empty()
                || self.floating.busy()
                || self.chats.iter().any(crate::chat::Chat::busy)
                || self.claude_chats.iter().any(crate::claude::Chat::busy)
                || self.share.hosting());
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_exit
            && (self.documents.iter().any(Document::dirty)
                || running_sessions_require_confirm
                || self.remote.needs_close_confirmation()
                || self.account_busy()
                || self.model_discovery.as_ref().is_some_and(|d| d.active())
                || self.native_exit_pending()
                || self.file_action_pending
                || self.transcript_trash_busy)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.native_terminals_not_closing() {
                #[cfg(windows)]
                {
                    self.native_terminals.picker = None;
                }
                self.pending_exit = true;
            }
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
                // `toolbarFit.js`: how tight the toolbar is comes from the room it measures
                // (the right group with everything shown, kept from the last frames).
                let fit_level = crate::toolbar_fit::toolbar_fit_level(
                    toolbar_room.width(),
                    160.0,
                    self.toolbar_right_full,
                    self.toolbar_secondary_w,
                    crate::toolbar_fit::TOOLBAR_MORE_WIDTH,
                    0.0,
                    crate::toolbar_fit::TOOLBAR_CENTRE_MIN,
                );
                let fit_full = fit_level == 0;
                let visible_right = if fit_full {
                    self.toolbar_right_full
                } else {
                    self.toolbar_right_full - self.toolbar_secondary_w
                        + crate::toolbar_fit::TOOLBAR_MORE_WIDTH
                        + 4.0
                };
                let regions = chrome::regions_fit(toolbar_room, visible_right + 4.0, fit_level);
                #[cfg(windows)]
                if let Some(frame) = &self.window_frame {
                    frame.show(ui, [regions.search, regions.actions]);
                }
                chrome::brand_with_build_fit(
                    ui,
                    regions.brand,
                    &self.dev_update.live_build_label(),
                    fit_level < 2,
                );
                if self.native_modal_open() || self.pending_exit {
                    ui.disable();
                }
                if regions.search.width() >= crate::toolbar_fit::TOOLBAR_COMMAND_ICON_WIDTH
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
                let mut secondary_w = 0.0_f32;
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
                let toolbar_french = self.state.language == crate::i18n::Language::French;
                let share_projects = self.live_share_project_list();
                let share_pill = self.share.pill_state().is_some();
                let share_x0 = ui.cursor().min.x;
                if (fit_full || share_pill)
                    && let Some(crate::share_panel::ShareUiAction::OpenDialog(tab)) =
                        self.share.show_toolbar_control(
                            ui,
                            &share_projects,
                            self.state.active_project,
                            toolbar_french,
                        )
                {
                    self.share.tab = tab;
                    self.overlay = Some(Overlay::Share);
                }
                if !share_pill {
                    secondary_w += ui.cursor().min.x - share_x0;
                }
                self.toolbar_menu_open |= self.share.is_pill_popover_open();
                let toolbar_spec = crate::theme::active(ui.ctx()).spec();
                let secondary_x0 = ui.cursor().min.x;
                if !fit_full {
                    egui::Popup::close_id(ui.ctx(), egui::Id::new("toolbar-issues"));
                }
                if fit_full {
                    let issues_id = egui::Id::new("toolbar-issues");
                    let issues_open = egui::Popup::is_id_open(ui.ctx(), issues_id);
                    let button = crate::issues_menu::button(ui, issues_open, toolbar_french);
                    let mut selected_issue_service = None;
                    let popup = egui::Popup::menu(&button)
                        .id(issues_id)
                        .align(egui::RectAlign::BOTTOM_END)
                        .gap(5.0)
                        .frame(
                            crate::layout_broadcast_ui::layout_menu_frame(&toolbar_spec.ui)
                                .shadow(ui.visuals().popup_shadow),
                        )
                        .show(|ui| {
                            selected_issue_service =
                                crate::issues_menu::contents(ui, &toolbar_spec.ui, toolbar_french);
                            if selected_issue_service.is_some() {
                                ui.close();
                            }
                        });
                    self.toolbar_menu_open |= popup.is_some();
                    if let Some(action) = selected_issue_service {
                        self.apply_toolbar_more_action(action);
                    }
                    if crate::layout_broadcast_ui::paint_broadcast_toolbar_button(
                        ui,
                        &toolbar_spec.ui,
                        toolbar_spec.radius_px,
                        toolbar_french,
                        self.state.broadcast,
                    )
                    .clicked()
                    {
                        self.state.broadcast = !self.state.broadcast;
                        self.save();
                    }
                    let layout_popup_id = egui::Id::new("toolbar-layout-menu");
                    let layout_menu_open = egui::Popup::is_id_open(ui.ctx(), layout_popup_id);
                    let layout = crate::layout_broadcast_ui::paint_layout_toolbar_button(
                        ui,
                        &toolbar_spec.ui,
                        toolbar_spec.radius_px,
                        toolbar_french,
                        layout_menu_open,
                    );
                    let pane_count = self
                        .sessions
                        .iter()
                        .filter(|s| s.project == self.state.active_project)
                        .count();
                    let layout_input = crate::layout_broadcast_ui::LayoutMenuInput {
                        french: toolbar_french,
                        has_active_pane: self.active_project_work_pane().is_some(),
                        can_split_shell: self.can_split_shell(pane_count > 0),
                        pane_count,
                        can_apply_grid: pane_count > 0 || self.sessions.len() < 100,
                    };
                    let mut layout_action = None;
                    let popup = egui::Popup::menu(&layout)
                        .id(layout_popup_id)
                        .align(egui::RectAlign::BOTTOM_END)
                        .gap(6.0)
                        .frame(
                            crate::layout_broadcast_ui::layout_menu_frame(&toolbar_spec.ui)
                                .shadow(ui.visuals().popup_shadow),
                        );
                    let shown = popup.show(|ui| {
                        if let Some(action) = crate::layout_broadcast_ui::show_layout_menu_contents(
                            ui,
                            &toolbar_spec.ui,
                            layout_input,
                        ) {
                            layout_action = Some(action);
                            ui.close();
                        }
                    });
                    self.toolbar_menu_open |= shown.is_some();
                    if let Some(action) = layout_action {
                        self.apply_layout_action(action);
                    }
                    self.floating_toolbar(ui);
                }
                secondary_w += ui.cursor().min.x - secondary_x0;
                let waiting_panes = self.sync_waiting_notifications();
                if self.state.notification_prefs.in_app_alerts
                    && let Some(pill_resp) = crate::dialogs_toolbar_ui::paint_agent_request_pill(
                        ui,
                        &toolbar_spec.ui,
                        waiting_panes.len(),
                        toolbar_french,
                    )
                    && pill_resp.clicked()
                    && let Some((first_pane, _)) = waiting_panes.first()
                {
                    self.handle_dialogs_toolbar_action(
                        ctx,
                        crate::dialogs_toolbar_ui::DialogsToolbarAction::FocusPane(*first_pane),
                    );
                }
                let usage_summary = crate::dialogs_toolbar_model::compute_usage_summary(
                    &self.state.task_history.records,
                    10.0,
                    crate::dialogs_toolbar_model::now_ms(),
                );
                let usage_popup_id = egui::Id::new("toolbar-usage-menu");
                let usage_open = egui::Popup::is_id_open(ui.ctx(), usage_popup_id);
                let usage_btn = crate::dialogs_toolbar_ui::paint_usage_toolbar_button(
                    ui,
                    &toolbar_spec.ui,
                    usage_summary.level,
                    toolbar_french,
                    usage_open,
                );
                let mut toolbar_dlg_act = None;
                let usage_popup = egui::Popup::menu(&usage_btn)
                    .id(usage_popup_id)
                    .align(egui::RectAlign::BOTTOM_END)
                    .gap(6.0);
                let shown_usage = usage_popup.show(|ui| {
                    if let Some(act) = crate::dialogs_toolbar_ui::show_usage_menu_contents(
                        ui,
                        &toolbar_spec.ui,
                        &usage_summary,
                        toolbar_french,
                    ) {
                        toolbar_dlg_act = Some(act);
                        ui.close();
                    }
                });
                self.toolbar_menu_open |= shown_usage.is_some();

                let notif_popup_id = egui::Id::new("toolbar-notifications-menu");
                let notif_open = egui::Popup::is_id_open(ui.ctx(), notif_popup_id);
                let notif_btn = crate::dialogs_toolbar_ui::paint_notifications_toolbar_button(
                    ui,
                    &toolbar_spec.ui,
                    self.notifications.unread_count(),
                    toolbar_french,
                    notif_open,
                );
                let notif_popup = egui::Popup::menu(&notif_btn)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .id(notif_popup_id)
                    .align(egui::RectAlign::BOTTOM_END)
                    .gap(6.0);
                let shown_notif = notif_popup.show(|ui| {
                    if let Some(act) = crate::dialogs_toolbar_ui::show_notifications_menu_contents(
                        ui,
                        &toolbar_spec.ui,
                        &self.notifications,
                        toolbar_french,
                    ) {
                        if matches!(act, crate::dialogs_toolbar_ui::DialogsToolbarAction::OpenNotification(_) | crate::dialogs_toolbar_ui::DialogsToolbarAction::OpenAlertsSettings) {
                            ui.close();
                        }
                        toolbar_dlg_act = Some(act);
                    }
                });
                self.toolbar_menu_open |= shown_notif.is_some();
                if let Some(act) = toolbar_dlg_act {
                    self.handle_dialogs_toolbar_action(ctx, act);
                }

                if let Some(pill_label) = self
                    .dev_update
                    .state()
                    .and_then(crate::dev_update::DevUpdateState::pill_label)
                {
                    let warn_c = toolbar_spec.ui.warn;
                    let btn = egui::Button::new(
                        egui::RichText::new(&pill_label)
                            .size(11.5)
                            .strong()
                            .color(warn_c),
                    )
                    .fill(warn_c.gamma_multiply(0.14_f32))
                    .stroke(egui::Stroke::new(1.0_f32, warn_c.gamma_multiply(0.45_f32)))
                    .corner_radius(egui::CornerRadius::same(10));
                    let tip = if toolbar_french {
                        "Nouveau code source disponible sur Git"
                    } else {
                        "Newer Git source commits available"
                    };
                    if ui.add(btn).on_hover_text(tip).clicked() {
                        self.dev_update_dialog_open = !self.dev_update_dialog_open;
                        if self.dev_update_dialog_open {
                            self.dev_update_notice = None;
                        }
                    }
                }

                let toggles_x0 = ui.cursor().min.x;
                if fit_full {
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
                }
                secondary_w += ui.cursor().min.x - toggles_x0;
                // `⋯`: the secondary buttons' actions when the toolbar is tight.
                let more_x0 = ui.cursor().min.x;
                if !fit_full {
                    let more_id = egui::Id::new("toolbar-more-menu");
                    let more_open = egui::Popup::is_id_open(ui.ctx(), more_id);
                    let more_btn = chrome::icon_button(
                        ui,
                        Icon::PaneMore,
                        crate::toolbar_fit::more_button_title(toolbar_french),
                        more_open,
                        false,
                    );
                    let entries =
                        crate::toolbar_fit::more_menu_entries(crate::toolbar_fit::MoreMenuState {
                            french: toolbar_french,
                            broadcast: self.state.broadcast,
                            floating_open: self.floating.ui.is_open(),
                            left_open: self.state.panels.left_open,
                            right_open: self.state.panels.right_open,
                            can_arrange_panes: self
                                .sessions
                                .iter()
                                .filter(|s| s.project == self.state.active_project)
                                .count()
                                >= 2,
                        });
                    let mut picked = None;
                    let shown_more = egui::Popup::menu(&more_btn)
                        .id(more_id)
                        .align(egui::RectAlign::BOTTOM_END)
                        .gap(6.0)
                        .show(|ui| {
                            ui.set_min_width(260.0);
                            for entry in &entries {
                                let Some(action) = entry.action else {
                                    ui.separator();
                                    continue;
                                };
                                let mut text = egui::RichText::new(format!(
                                    "{}{}",
                                    entry.label,
                                    if entry.checked { " ✓" } else { "" }
                                ));
                                if entry.danger {
                                    text = text.color(ui.visuals().error_fg_color);
                                }
                                let mut button = egui::Button::new(text).frame(false);
                                if let Some(sc) = entry.shortcut {
                                    button = button.shortcut_text(sc);
                                }
                                if ui.add(button).clicked() {
                                    picked = Some(action);
                                    ui.close();
                                }
                            }
                        });
                    self.toolbar_menu_open |= shown_more.is_some();
                    if let Some(action) = picked {
                        self.apply_toolbar_more_action(action);
                    }
                }
                let more_w = ui.cursor().min.x - more_x0;
                if chrome::icon_button(ui, Icon::Settings, tr("Settings"), false, false).clicked() {
                    self.overlay = Some(Overlay::Settings);
                }
                // Remember the room with everything shown (`toolbarFit.js` measures the groups
                // with the tight classes off; here the hidden buttons keep their last width).
                let used = ui.min_rect().width();
                if fit_full {
                    self.toolbar_right_full = used;
                    self.toolbar_secondary_w = secondary_w;
                } else {
                    self.toolbar_right_full = used - more_w + self.toolbar_secondary_w;
                }
                let next_level = crate::toolbar_fit::toolbar_fit_level(
                    toolbar_room.width(),
                    160.0,
                    self.toolbar_right_full,
                    self.toolbar_secondary_w,
                    crate::toolbar_fit::TOOLBAR_MORE_WIDTH,
                    0.0,
                    crate::toolbar_fit::TOOLBAR_CENTRE_MIN,
                );
                if next_level != fit_level {
                    ui.ctx().request_repaint();
                }
            });
        if self.state.broadcast {
            let french = self.state.language == crate::i18n::Language::French;
            let spec = crate::theme::active(ctx).spec();
            egui::TopBottomPanel::top("broadcast-banner")
                .exact_height(crate::layout_broadcast_ui::BROADCAST_BANNER_HEIGHT)
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    crate::layout_broadcast_ui::show_broadcast_banner(ui, &spec.ui, french);
                });
        }
        self.render_status_bar(ctx);
        self.sidebar(ctx);
        self.right_panel(ctx);
        #[cfg(windows)]
        {
            if self.view == View::Browser {
                self.browser_tab_open = true;
            }
            self.browser.begin_frame();
            self.agent_browsers.begin_frame();
        }
        let central = egui::CentralPanel::default().show(ctx, |ui| {
            if self.native_modal_open()
                || self.side_panels_ui.any_modal_open()
                || self.dialogs_toolbar_ui.any_modal_open()
                || self.new_task.open
                || self.linear.view.open
                || self.share.has_pending_ask()
                || self.finder.is_open()
                || self.toolbar_menu_open
                || self.status_bar.has_open_popover()
                || self.command_palette.open
                || self.overlay.is_some()
                || self.pending_exit
                || self.pending_close.is_some()
                || self.pending_scm_discard.is_some()
                || self.rename_project_modal.is_some()
                || self.file_action.is_some()
                || self.restore_preview.is_some()
            {
                ui.disable();
            }
            self.center_tabs(ui);
            match self.view {
                View::Work => self.work(ui),
                View::Files => self.editor(ui),
                View::GuestFiles => {
                    if take_close_shortcut(
                        ctx,
                        ui.is_enabled() && !self.floating.ui.is_open(),
                        egui::Modifiers::CTRL,
                    ) && let Some(key) = self.share.active_workspace_file().cloned()
                    {
                        self.share.close_guest_tab(&key);
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("guest-workspace-document")
                        .show(ui, |ui| {
                            self.share
                                .render_guest_document(ui, crate::i18n::is_french(), true)
                        });
                }
                View::Remote => {
                    self.remote
                        .show(ui, &self.state.hosts, self.state.editor_prefs.font_size);
                    self.open_requested_remote_terminal();
                }
                #[cfg(windows)]
                View::Browser => {
                    if !self.state.browser_prefs.separate_agent_session {
                        self.show_agent_browser = false;
                    }
                    if !self.native_modal_open()
                        && !self.share.has_pending_ask()
                        && !self.pending_exit
                        && self.pending_close.is_none()
                        && self.pending_close_session.is_none()
                        && self.pending_remove_project.is_none()
                        && self.pending_scm_discard.is_none()
                        && self.rename_project_modal.is_none()
                        && self.file_action.is_none()
                        && self.restore_preview.is_none()
                        && !self.command_palette.open
                        && self.overlay.is_none()
                        && !self.toolbar_menu_open
                        && !self.status_bar.has_open_popover()
                        && !self.finder.is_open()
                    {
                        if self.state.browser_prefs.separate_agent_session {
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
                        }
                        if self.show_agent_browser {
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
        #[cfg_attr(not(windows), allow(unused_variables))]
        let floating_rect = self.show_floating(ctx, central.response.rect);
        let can_insert = self.finder_target().is_some();
        if let Some(action) = self.finder.show_action(
            ctx,
            self.state.active().map(|p| p.path.as_path()),
            can_insert,
        ) {
            self.handle_finder_action(ctx, action);
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
                        can_rename: true,
                        language: if self.state.language == crate::i18n::Language::French {
                            crate::pane_menu_model::PaneMenuLang::Fr
                        } else {
                            crate::pane_menu_model::PaneMenuLang::En
                        },
                        ..Default::default()
                    },
                );
            });
        }
        self.confirmations(ctx);
        {
            let french = self.state.language == crate::i18n::Language::French;
            let proj_name = self
                .state
                .active()
                .map(|p| p.name.clone())
                .unwrap_or_default();
            if let Some(act) = crate::side_panels_ui::show_notes_modal(
                ctx,
                &mut self.side_panels_ui,
                &proj_name,
                french,
            ) {
                self.handle_side_panels_action(ctx, act);
            }
            let active_root = self.state.active().map(|p| p.path.clone());
            let ws_roots: Vec<PathBuf> =
                self.state.projects.iter().map(|p| p.path.clone()).collect();
            if let Some(act) = crate::side_panels_ui::show_sessions_modal(
                ctx,
                &mut self.side_panels_ui,
                &self.past_sessions_cache,
                active_root.as_deref(),
                &ws_roots,
                french,
            ) {
                self.handle_side_panels_action(ctx, act);
            }
            let candidates = self.review_candidate_branches();
            if let Some(act) = crate::side_panels_ui::show_review_modal(
                ctx,
                &mut self.side_panels_ui,
                &candidates,
                french,
            ) {
                self.handle_side_panels_action(ctx, act);
            }
            let ssh_aliases: Vec<String> =
                self.state.hosts.iter().map(|h| h.alias.clone()).collect();
            if let Some(act) = crate::dialogs_toolbar_ui::show_add_project_modal(
                ctx,
                &mut self.dialogs_toolbar_ui,
                &ssh_aliases,
                french,
            ) {
                self.handle_dialogs_toolbar_action(ctx, act);
            }
            if let Some(act) = crate::dialogs_toolbar_ui::show_github_modal(
                ctx,
                &mut self.dialogs_toolbar_ui,
                &proj_name,
                french,
            ) {
                self.handle_dialogs_toolbar_action(ctx, act);
            }
            crate::linear_dialog::show(ctx, &mut self.linear, french);
            let task_epoch = self.new_task.epoch;
            if let Some(command) = crate::new_task_form::show(ctx, &mut self.new_task, french) {
                self.new_task_command(task_epoch, command);
            }
            let existing_groups: Vec<String> = {
                let mut set: std::collections::BTreeSet<String> =
                    self.state.project_groups.values().cloned().collect();
                set.extend(self.state.collapsed_project_groups.iter().cloned());
                set.into_iter().collect()
            };
            if let Some(act) = crate::dialogs_toolbar_ui::show_project_group_modal(
                ctx,
                &mut self.dialogs_toolbar_ui,
                &existing_groups,
                french,
            ) {
                self.handle_dialogs_toolbar_action(ctx, act);
            }
            let pal = crate::theme::active(ctx).spec().ui;
            if let Some(act) = crate::teams_ui::show_worker_confirm_modal(
                ctx,
                &pal,
                &self.teams_hub.workers,
                french,
            ) {
                self.handle_teams_ui_action(act);
            }
            if let Some(act) = crate::teams_ui::show_relay_approval_modal(
                ctx,
                &pal,
                &mut self.teams_hub.approval_prompts,
                french,
            ) {
                self.handle_teams_ui_action(act);
            }
            if let Some(act) = crate::teams_ui::show_team_manager_modal(
                ctx,
                &pal,
                &mut self.teams_ui,
                &self.state,
                &self.teams_hub,
                french,
            ) {
                self.handle_teams_ui_action(act);
            }
        }
        #[cfg_attr(not(windows), allow(unused_variables))]
        let ask_modal_open = if self.share.hosting() {
            let all_projects = self.live_share_project_list();
            let french = self.state.language == crate::i18n::Language::French;
            self.share.show_ask_modal(ctx, &all_projects, french)
        } else {
            false
        };
        if self.dev_update_dialog_open {
            let french = self.state.language == crate::i18n::Language::French;
            let mut open = self.dev_update_dialog_open;
            let mut do_refresh = false;
            let mut do_apply = false;
            let mut do_copy_pull = false;
            let title = if french {
                "Mise à jour du code (Git)"
            } else {
                "Developer update (Git)"
            };
            egui::Window::new(title)
                .id(egui::Id::new("compat-dev-update-dialog"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .default_width(480.0)
                .open(&mut open)
                .show(ctx, |ui| {
                    if let Some(st) = self.dev_update.state() {
                        ui.label(if french {
                            format!(
                                "Version locale : b{} ({}) · Distante : b{} ({} commit(s) en retard)",
                                st.local_build, st.local_commit, st.remote_build, st.behind
                            )
                        } else {
                            format!(
                                "Local build: b{} ({}) · Upstream: b{} ({} commit(s) behind)",
                                st.local_build, st.local_commit, st.remote_build, st.behind
                            )
                        });
                        if !st.incoming_log.is_empty() {
                            ui.add_space(6.0);
                            ui.strong(if french {
                                "Commits entrants"
                            } else {
                                "Incoming commits"
                            });
                            egui::ScrollArea::vertical()
                                .id_salt("compat-dev-update-log")
                                .max_height(130.0)
                                .show(ui, |ui| {
                                    for line in &st.incoming_log {
                                        ui.monospace(line);
                                    }
                                });
                        }
                        if !st.incoming_files.is_empty() {
                            ui.add_space(6.0);
                            ui.small(if french {
                                format!("{} fichier(s) modifié(s) en amont", st.incoming_files.len())
                            } else {
                                format!("{} file(s) changed upstream", st.incoming_files.len())
                            });
                        }
                        if let Some(blocked) = &st.blocked {
                            ui.add_space(6.0);
                            match blocked {
                                crate::dev_update::DevUpdateBlocked::Dirty { overlap } => {
                                    ui.colored_label(
                                        ui.visuals().warn_fg_color,
                                        crate::dev_update::format_overlap_message(overlap, french),
                                    );
                                }
                                crate::dev_update::DevUpdateBlocked::Ahead { count } => {
                                    ui.colored_label(
                                        ui.visuals().warn_fg_color,
                                        if french {
                                            format!(
                                                "Cette branche a {count} commit(s) locaux d'avance."
                                            )
                                        } else {
                                            format!(
                                                "This branch is {count} local commit(s) ahead."
                                            )
                                        },
                                    );
                                }
                            }
                        }
                    } else {
                        ui.weak(if french {
                            "Dépôt Git non détecté."
                        } else {
                            "Git checkout not detected."
                        });
                    }
                    if let Some(notice) = &self.dev_update_notice {
                        ui.add_space(6.0);
                        ui.label(notice);
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let busy = self.dev_update.is_busy();
                        let can_ff = self
                            .dev_update
                            .state()
                            .is_some_and(|s| s.behind > 0 && s.blocked.is_none())
                            && !busy;
                        if ui
                            .add_enabled(
                                can_ff,
                                egui::Button::new(if french {
                                    "Mettre à jour (fast-forward)"
                                } else {
                                    "Update checkout (fast-forward)"
                                }),
                            )
                            .clicked()
                        {
                            do_apply = true;
                        }
                        if ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(if french { "Vérifier" } else { "Check now" }),
                            )
                            .clicked()
                        {
                            do_refresh = true;
                        }
                        if ui
                            .button(if french {
                                "Copier git pull"
                            } else {
                                "Copy git pull"
                            })
                            .clicked()
                        {
                            do_copy_pull = true;
                        }
                    });
                });
            self.dev_update_dialog_open = open;
            if do_refresh {
                self.dev_update_notice = None;
                self.dev_update.refresh_now(true);
            }
            if do_apply {
                self.dev_update_notice = None;
                self.dev_update.apply_now();
            }
            if do_copy_pull {
                ctx.copy_text("git pull --ff-only".into());
                self.dev_update_notice = Some(if french {
                    "Commande « git pull --ff-only » copiée.".into()
                } else {
                    "Copied \"git pull --ff-only\".".into()
                });
            }
        }
        #[cfg(windows)]
        {
            let media = crate::files::playback::Source::take_presented(ctx);
            let unobscured = !self.native_modal_open()
                && !self.side_panels_ui.any_modal_open()
                && !self.share.has_pending_ask()
                && !self.floating.ui.is_open()
                && !egui::Popup::is_any_open(ctx);
            self.media_player
                .retain(media.as_ref().map(|(source, _)| source));
            if let Some((source, bounds)) = media
                && unobscured
            {
                self.media_player.sync(
                    frame,
                    &source,
                    bounds,
                    ctx.pixels_per_point(),
                    self.state.language == crate::i18n::Language::French,
                );
            } else {
                self.media_player.hide();
            }
        }
        #[cfg(windows)]
        {
            self.dispatch_native_actions(ctx);
            self.show_native_picker(ctx);
            self.show_native_drain(ctx);
            let browser_unblocked = self.view == View::Browser
                && !self.dev_update_dialog_open
                && !self.native_modal_open()
                && !self.side_panels_ui.any_modal_open()
                && !self.dialogs_toolbar_ui.any_modal_open()
                && !self.new_task.open
                && !self.linear.view.open
                && self.teams_ui.manager_project.is_none()
                && self.teams_hub.approval_prompts.is_empty()
                && !ask_modal_open
                && !self.share.has_pending_ask()
                && !self.pending_exit
                && self.pending_close.is_none()
                && self.pending_close_session.is_none()
                && self.pending_remove_project.is_none()
                && self.pending_scm_discard.is_none()
                && self.pending_agent_install.is_none()
                && self.rename_project_modal.is_none()
                && self.file_action.is_none()
                && self.restore_preview.is_none()
                && !self.command_palette.open
                && self.overlay.is_none()
                && !self.toolbar_menu_open
                && !self.status_bar.has_open_popover()
                && !self.finder.is_open()
                && !egui::Popup::is_any_open(ctx);
            let reclaim_parent_focus = (!had_floating_keyboard && self.floating.ui.has_keyboard())
                || ctx.input(|i| i.pointer.any_pressed());
            let owner_took_focus = self.browser.sync_presentation(
                browser_unblocked && !self.show_agent_browser,
                floating_rect,
                reclaim_parent_focus,
            );
            let active_agent_project = self
                .state
                .active()
                .map(|p| p.id)
                .filter(|_| browser_unblocked && self.show_agent_browser);
            let agent_took_focus = self.agent_browsers.sync_presentation(
                active_agent_project,
                floating_rect,
                reclaim_parent_focus,
            );
            if (owner_took_focus || agent_took_focus) && self.floating.ui.has_keyboard() {
                self.floating.ui.keyboard = false;
                let floating_focus = egui::Id::new("floating-terminal-content");
                if ctx.memory(|m| m.focused()) == Some(floating_focus) {
                    ctx.memory_mut(|m| m.surrender_focus(floating_focus));
                }
            }
        }
        #[cfg(windows)]
        if self.overlay != Some(Overlay::Settings)
            || self.settings_section != SettingsSection::Passwords
        {
            self.vault.suspend();
        }
        if previous_panels != self.state.panels {
            self.save();
        }
        ctx.request_repaint_after(Duration::from_millis(
            if self.sessions.is_empty() && !self.floating.busy() {
                150
            } else {
                33
            },
        ));
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

pub const SUPPORTED_HELP_SHORTCUT_KEYS: &[&str] = &[
    // Panes
    "Ctrl+Shift+T",
    "Ctrl+Shift+Space",
    "Ctrl+Shift+P",
    "Ctrl+Shift+J",
    "Ctrl+Shift+E",
    "Ctrl+Shift+O",
    "Ctrl+Shift+W",
    "Ctrl+Shift+R",
    "Alt+Arrows",
    "Esc",
    "Ctrl+`",
    // Terminal
    "Ctrl+Shift+F",
    "Ctrl+Shift+C",
    "Ctrl+Shift+V",
    "Ctrl+=",
    "Ctrl+-",
    "Ctrl+0",
    "Shift+PageUp",
    "Shift+PageDown",
    // Workspaces
    "Ctrl+Shift+N",
    "Ctrl+PageUp",
    "Ctrl+PageDown",
    // Editor
    "Ctrl+S",
    "Ctrl+W",
    "Ctrl+F",
    "Ctrl+H",
    "Ctrl+G",
    "Alt+Z",
    "F7 / Shift+F7",
    // App
    "Ctrl+Shift+B",
    "Alt+A",
    "Ctrl+Shift+K",
    "Ctrl+Shift+X",
    "Ctrl+Shift+G",
    "Ctrl+,",
    #[cfg(windows)]
    "Win+H",
    "F1",
];

pub fn shell_quote_path(path: &std::path::Path) -> String {
    let raw = path.display().to_string();
    let needs_quote = raw.is_empty()
        || raw.chars().any(|c| {
            c.is_whitespace()
                || matches!(
                    c,
                    '\'' | '"' | '$' | '`' | '&' | '|' | ';' | '(' | ')' | '<' | '>'
                )
        });
    if !needs_quote {
        raw
    } else {
        format!("'{}'", raw.replace('\'', "''"))
    }
}

fn show_editor_bars(ui: &mut egui::Ui, d: &mut Document) {
    if d.tools.goto_open {
        ui.horizontal(|ui| {
            ui.label(tr("Go to line"));
            let input = ui.add(
                egui::TextEdit::singleline(&mut d.tools.goto_line)
                    .desired_width(180.0)
                    .hint_text(tr("Go to line (e.g. 42 or 42:10)…")),
            );
            if d.tools.focus_goto {
                input.request_focus();
                d.tools.focus_goto = false;
            }
            let enter = (input.has_focus() || input.lost_focus())
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            let go_clicked = ui.small_button(tr("Go")).clicked();
            if (enter || go_clicked)
                && let Some((char_pos, line_0)) =
                    crate::files::goto_line_char_offset(&d.text, &d.tools.goto_line)
            {
                d.tools.pending_char_range = Some((char_pos, char_pos));
                d.tools.pending_scroll_line = Some(line_0);
                d.tools.goto_open = false;
            }
            let esc = input.has_focus()
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            if esc || ui.small_button("×").clicked() {
                d.tools.goto_open = false;
            }
        });
    }
    if d.tools.find_open {
        let mut matches = crate::files::find_char_matches(&d.text, &d.tools.find_query);
        if matches.is_empty() {
            d.tools.match_index = 0;
        } else {
            d.tools.match_index %= matches.len();
        }
        let mut step_dir: Option<isize> = None;
        let mut do_replace_one = false;
        let mut do_replace_all = false;
        let mut close_find = false;
        ui.horizontal_wrapped(|ui| {
            let find_input = ui.add(
                egui::TextEdit::singleline(&mut d.tools.find_query)
                    .desired_width(180.0)
                    .hint_text(tr("Find in file…")),
            );
            if d.tools.focus_find {
                find_input.request_focus();
                d.tools.focus_find = false;
            }
            if find_input.changed() {
                d.tools.match_index = 0;
                matches = crate::files::find_char_matches(&d.text, &d.tools.find_query);
                if let Some(&(sc, ec, line)) = matches.first() {
                    d.tools.pending_char_range = Some((sc, ec));
                    d.tools.pending_scroll_line = Some(line);
                }
            }
            let count_text = if d.tools.find_query.is_empty() {
                String::new()
            } else if matches.is_empty() {
                "0 / 0".to_string()
            } else {
                format!("{} / {}", d.tools.match_index + 1, matches.len())
            };
            if !count_text.is_empty() {
                ui.weak(count_text);
            }
            if ui
                .add_enabled(!matches.is_empty(), egui::Button::new("↑").small())
                .clicked()
            {
                step_dir = Some(-1);
            }
            if ui
                .add_enabled(!matches.is_empty(), egui::Button::new("↓").small())
                .clicked()
            {
                step_dir = Some(1);
            }
            if find_input.has_focus() || find_input.lost_focus() {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter)) {
                    step_dir = Some(-1);
                    find_input.request_focus();
                } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                    step_dir = Some(1);
                    find_input.request_focus();
                }
            }
            if d.tools.replace_open {
                let rep_input = ui.add(
                    egui::TextEdit::singleline(&mut d.tools.replace_text)
                        .desired_width(160.0)
                        .hint_text(tr("Replace with…")),
                );
                if (rep_input.has_focus() || rep_input.lost_focus())
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                {
                    do_replace_one = true;
                    rep_input.request_focus();
                }
                if ui
                    .add_enabled(
                        !matches.is_empty(),
                        egui::Button::new(tr("Replace")).small(),
                    )
                    .clicked()
                {
                    do_replace_one = true;
                }
                if ui
                    .add_enabled(
                        !matches.is_empty(),
                        egui::Button::new(tr("Replace all")).small(),
                    )
                    .clicked()
                {
                    do_replace_all = true;
                }
            }
            let esc = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            if esc || ui.small_button("×").clicked() {
                close_find = true;
            }
        });
        if let Some(dir) = step_dir
            && !matches.is_empty()
        {
            let len = matches.len() as isize;
            d.tools.match_index =
                (((d.tools.match_index as isize + dir) % len + len) % len) as usize;
            let (sc, ec, line) = matches[d.tools.match_index];
            d.tools.pending_char_range = Some((sc, ec));
            d.tools.pending_scroll_line = Some(line);
        }
        if do_replace_one && let Some(&(sc, ec, line)) = matches.get(d.tools.match_index) {
            let (sb, eb) = crate::files::char_range_to_byte_range(&d.text, sc, ec);
            d.text.replace_range(sb..eb, &d.tools.replace_text);
            d.preview = None;
            d.unpinned_preview = false;
            d.last_edited_at = Some(std::time::Instant::now());
            d.syntax_cache.invalidate();
            let rep_chars = d.tools.replace_text.chars().count();
            d.tools.pending_char_range = Some((sc, sc + rep_chars));
            d.tools.pending_scroll_line = Some(line);
        } else if do_replace_all && !matches.is_empty() {
            for &(sc, ec, _) in matches.iter().rev() {
                let (sb, eb) = crate::files::char_range_to_byte_range(&d.text, sc, ec);
                d.text.replace_range(sb..eb, &d.tools.replace_text);
            }
            d.preview = None;
            d.unpinned_preview = false;
            d.last_edited_at = Some(std::time::Instant::now());
            d.syntax_cache.invalidate();
            d.tools.match_index = 0;
        }
        if close_find {
            d.tools.find_open = false;
            d.tools.replace_open = false;
        }
    }
}

fn show_diff_document(ui: &mut egui::Ui, d: &Document, font_size: f32, diff_word_wrap: bool) {
    let font = egui::FontId::monospace(font_size);
    if d.text.trim().is_empty() {
        ui.weak(tr("No diff output for this file."));
        return;
    }
    let add_fg = egui::Color32::from_rgb(0x73, 0xc9, 0x91);
    let del_fg = egui::Color32::from_rgb(0xf1, 0x4c, 0x4c);
    let hunk_fg = egui::Color32::from_rgb(0x56, 0xb6, 0xc2);
    let meta_fg = ui.visuals().weak_text_color();
    let base_fg = ui.visuals().text_color();

    let scroll = if diff_word_wrap {
        egui::ScrollArea::vertical()
    } else {
        egui::ScrollArea::both()
    }
    .id_salt((&d.path, d.diff_title.as_deref().unwrap_or("diff")));
    scroll.show(ui, |ui| {
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = if diff_word_wrap {
            ui.available_width().max(80.0_f32)
        } else {
            f32::INFINITY
        };
        for (idx, line) in d.text.split('\n').enumerate() {
            if idx > 0 {
                job.append(
                    "\n",
                    0.0,
                    egui::TextFormat {
                        font_id: font.clone(),
                        color: base_fg,
                        ..Default::default()
                    },
                );
            }
            let (fg, bg) = if line.starts_with("+++ ")
                || line.starts_with("--- ")
                || line.starts_with("diff --git")
                || line.starts_with("index ")
                || line.starts_with("new file")
                || line.starts_with("deleted file")
            {
                (meta_fg, egui::Color32::TRANSPARENT)
            } else if line.starts_with("@@") {
                (hunk_fg, hunk_fg.gamma_multiply(0.08_f32))
            } else if line.starts_with('+') {
                (add_fg, add_fg.gamma_multiply(0.10_f32))
            } else if line.starts_with('-') {
                (del_fg, del_fg.gamma_multiply(0.10_f32))
            } else {
                (base_fg, egui::Color32::TRANSPARENT)
            };
            job.append(
                line,
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: fg,
                    background: bg,
                    ..Default::default()
                },
            );
        }
        ui.add(egui::Label::new(job).selectable(true));
    });
}

fn document_toolbar(
    ui: &mut egui::Ui,
    d: &mut Document,
    readable: bool,
    editor_prefs: &mut crate::workspace::EditorPreferences,
    profile: Option<&std::path::Path>,
) -> (bool, bool, bool) {
    let mut open_external = false;
    let source_mode = !d.media && d.diff_title.is_none() && !(readable && d.preview_mode);
    let mut prefs_changed = false;
    if ui.is_enabled() && source_mode {
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
            d.tools.find_open = true;
            d.tools.focus_find = true;
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::H)) {
            d.tools.find_open = true;
            d.tools.replace_open = true;
            d.tools.focus_find = true;
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::G)) {
            d.tools.goto_open = true;
            d.tools.focus_goto = true;
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::Z)) {
            editor_prefs.word_wrap = !editor_prefs.word_wrap;
            prefs_changed = true;
        }
        let prev_hunk = ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::F7));
        let next_hunk =
            !prev_hunk && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F7));
        if (prev_hunk || next_hunk)
            && let Some(orig) = d.original_text()
        {
            let hunks = crate::files::changed_line_hunks(orig, &d.text);
            if !hunks.is_empty() {
                let len = hunks.len();
                d.tools.diff_hunk_index = if prev_hunk {
                    (d.tools.diff_hunk_index + len - 1) % len
                } else {
                    (d.tools.diff_hunk_index + 1) % len
                };
                let (char_pos, line_0) = hunks[d.tools.diff_hunk_index];
                d.tools.pending_char_range = Some((char_pos, char_pos));
                d.tools.pending_scroll_line = Some(line_0);
            }
        }
    }
    let close = ui
        .horizontal(|ui| {
            if !d.media {
                let save_clicked = ui.button(tr("Save (Ctrl+S)")).clicked();
                if ui.is_enabled()
                    && (save_clicked
                        || ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S)))
                {
                    match d.save(profile) {
                        Ok(()) => {
                            d.error.clear();
                            d.unpinned_preview = false;
                            d.deleted_on_disk = false;
                        }
                        Err(err) => d.error = err,
                    }
                }
            }
            if readable {
                ui.selectable_value(&mut d.preview_mode, false, tr("Source"));
                ui.selectable_value(&mut d.preview_mode, true, tr("Readable preview"));
            }
            if source_mode {
                prefs_changed |= ui
                    .toggle_value(&mut editor_prefs.word_wrap, tr("Word wrap"))
                    .changed();
                if ui
                    .toggle_value(&mut editor_prefs.line_numbers, tr("Line numbers"))
                    .changed()
                {
                    prefs_changed = true;
                }
                if ui
                    .toggle_value(
                        &mut editor_prefs.syntax_highlighting,
                        tr("Syntax highlighting"),
                    )
                    .changed()
                {
                    prefs_changed = true;
                    d.syntax_cache.invalidate();
                }
                if ui.button(tr("Find…")).clicked() {
                    d.tools.find_open = true;
                    d.tools.focus_find = true;
                }
                if ui.button(tr("Go to line")).clicked() {
                    d.tools.goto_open = true;
                    d.tools.focus_goto = true;
                }
            }
            if d.external_available() {
                if ui.small_button(crate::i18n::tr("Copy path")).clicked() {
                    ui.ctx()
                        .copy_text(dunce::simplified(&d.path).display().to_string());
                }
                open_external = ui
                    .button(tr("Open in VS Code"))
                    .on_hover_text(tr("Open in VS Code at the cursor position"))
                    .clicked();
            }
            let close = ui.button(tr("Close")).clicked()
                || take_close_shortcut(ui.ctx(), ui.is_enabled(), egui::Modifiers::CTRL);
            if let Some(diff_title) = &d.diff_title {
                ui.small(format!("{diff_title} · {}", d.path.display()));
            } else if source_mode {
                let lang_name = crate::syntax::SyntaxLang::from_path(&d.path).display_name();
                ui.small(format!("{} · {lang_name}", d.path.display()));
            } else {
                ui.small(d.path.display().to_string());
            }
            close
        })
        .inner;
    (close, prefs_changed, open_external)
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
    fn task_preparation_prevents_settings_restore_until_worktree_result_is_drained() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        assert!(app.can_restore());
        app.worktree_state
            .request_create(crate::worktree_state::CreateRequest {
                project_id: 1,
                cwd: temp.path().join("missing"),
                label: "fixture".into(),
                base_branch: None,
                copy_env: false,
                run_setup: false,
                general_prefs: Default::default(),
                git_prefs: Default::default(),
                target: crate::worktree_state::WorktreeLaunchTarget::ShellNewPane,
            });
        assert!(!app.can_restore());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.worktree_state.is_busy() {
            app.worktree_state.poll(&app.state.projects);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.can_restore());
        assert!(app.sessions.is_empty());
        assert!(!temp.path().join("missing").exists());
    }

    #[test]
    fn worktree_preparation_rejects_removed_or_retargeted_project_and_preserves_copy() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let prepared = temp.path().join("prepared");
        std::fs::create_dir(&prepared).unwrap();
        std::fs::write(prepared.join("keep"), "fixture").unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        assert!(!app.accept_prepared_worktree(7, &source, &prepared));
        app.state.projects.push(workspace::Project {
            id: 7,
            name: "fixture".into(),
            path: source.clone(),
        });
        assert!(app.accept_prepared_worktree(7, &source, &prepared));
        app.state.projects[0].path = temp.path().join("different");
        assert!(!app.accept_prepared_worktree(7, &source, &prepared));
        assert!(app.error.contains("No agent was started"));
        assert!(app.error.contains(prepared.to_str().unwrap()));
        assert!(prepared.join("keep").exists());
        assert!(app.sessions.is_empty());
        assert!(app.state.active_project.is_none());
    }

    #[test]
    fn issues_menu_github_without_project_clears_previous_repository_results() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        assert!(app.state.active_project.is_none());
        app.dialogs_toolbar_ui
            .github_issues
            .push(crate::dialogs_toolbar_model::GhIssue {
                number: 1,
                title: "Previous repository fixture".into(),
                state: "OPEN".into(),
                author: "fixture".into(),
                url: "https://github.com/fixture/old/issues/1".into(),
            });
        app.dialogs_toolbar_ui.github_query = "old repository".into();
        app.open_github_modal(None);
        assert!(app.dialogs_toolbar_ui.github_issues.is_empty());
        assert!(app.dialogs_toolbar_ui.github_prs.is_empty());
        assert!(app.dialogs_toolbar_ui.github_query.is_empty());
        assert_eq!(
            app.dialogs_toolbar_ui.github_error,
            "Select a project first."
        );
    }

    #[test]
    #[cfg(windows)]
    fn claude_launch_finishes_account_trust_before_the_child_reads_it() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path().join("home");
        let project = home.join("project");
        let account = home.join("account");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir(&account).unwrap();
        std::fs::write(home.join(".claude.json"), r#"{"defaultAccount":true}"#).unwrap();
        std::fs::write(account.join(".claude.json"), "{}").unwrap();
        let script = fixture.path().join("observe.ps1");
        std::fs::write(&script, r#"
$config = Get-Content -Raw -LiteralPath (Join-Path $env:CLAUDE_CONFIG_DIR '.claude.json') | ConvertFrom-Json
$trusted = @($config.projects.PSObject.Properties | Where-Object { $_.Value.hasTrustDialogAccepted -eq $true }).Count
Set-Content -LiteralPath (Join-Path $env:CLAUDE_CONFIG_DIR 'observed.txt') -Value $trusted
"#).unwrap();
        let prefs = crate::agent_preferences::Preferences {
            command: format!("& '{}'", script.display().to_string().replace('\'', "''")),
            env: format!("CLAUDE_CONFIG_DIR={}", account.display()),
            ..Default::default()
        };
        let mut session = Session {
            profile_data_dir: None,
            profile_root: None,
            profile_migration: None,
            resume_required: false,
            ssh_retry: Default::default(),
            ssh_tmux_id: None,
            native_host: None,
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            settings_changed: false,
            codex_seed: None,
            conversation_id: None,
            profile_binding: None,
            launch_fingerprint: None,
            session_options: None,
            worktree: None,
            pane_name: None,
            broadcast: true,
            scrollback: 100,
            id: 1,
            project: Some(1),
            title: "Synthetic trust launch".into(),
            launch: Launch::Agent("claude".into()),
            cwd: project,
            terminal: None,
            loading: None,
            error: String::new(),
        };
        session.start_with_trust(Some(prefs), None, Some(home.clone()));
        let observed = account.join("observed.txt");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !std::fs::read_to_string(&observed).is_ok_and(|value| value.trim() == "1")
            && std::time::Instant::now() < deadline
        {
            session.poll();
            assert!(session.error.is_empty(), "{}", session.error);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(std::fs::read_to_string(observed).unwrap().trim(), "1");
        assert_eq!(
            std::fs::read_to_string(home.join(".claude.json")).unwrap(),
            r#"{"defaultAccount":true}"#
        );
        if let Some(mut terminal) = session.terminal.take() {
            terminal.stop_for_restart().unwrap();
        }
    }
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
            profile_data_dir: None,
            profile_root: None,
            profile_migration: None,
            resume_required: false,
            ssh_retry: Default::default(),
            ssh_tmux_id: None,
            native_host: None,
            restarting: Some(Restart {
                preferred_shell: None,
                switch_mode: false,
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
            profile_binding: None,
            settings_changed: false,
            launch_fingerprint: None,
            session_options: None,
            worktree: None,
            pane_name: None,
            broadcast: true,
            scrollback: 10_000,
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
            preferred_shell: None,
            switch_mode: false,
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
            profile_data_dir: None,
            profile_root: None,
            profile_migration: None,
            resume_required: false,
            ssh_retry: Default::default(),
            ssh_tmux_id: None,
            native_host: None,
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            codex_seed: None,
            conversation_id: None,
            profile_binding: None,
            settings_changed: false,
            launch_fingerprint: None,
            session_options: None,
            worktree: None,
            pane_name: None,
            broadcast: true,
            scrollback: 10_000,
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

    #[cfg(windows)]
    #[test]
    fn playback_editor_open_routes_local_media_to_both_renderers() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("clip.WEBM");
        std::fs::write(&path, include_bytes!("../tests/fixtures/player-blue.webm")).unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(tmp.path().join("state")), false, || {});
        app.apply_gpui_action(gpui_bridge::GpuiAction::AddProjectPath(tmp.path().into()));
        app.apply_gpui_action(gpui_bridge::GpuiAction::OpenDocumentPath(path));
        let snapshot = app.gpui_snapshot(960., 640., 4.);
        assert!(snapshot.active_document.unwrap().playback.is_some());
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.editor(ui));
        });
        assert!(
            crate::files::playback::Source::take_presented(&ctx)
                .unwrap()
                .1
                .is_positive()
        );
        assert!(crate::files::playback::Source::take_presented(&ctx).is_none());
    }

    #[test]
    fn compatibility_local_image_fits_and_opens_its_menu() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("portrait.png");
        image::RgbaImage::from_pixel(100, 400, image::Rgba([0u8, 90, 80, 255]))
            .save(&path)
            .unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(tmp.path().join("state")), false, || {});
        app.documents = vec![Document::open(tmp.path(), &path).unwrap()];
        app.active_document = 0;
        app.view = View::Files;
        let ctx = egui::Context::default();
        let mut frame = |events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640., 320.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.editor(ui));
                },
            )
        };
        let output = frame(vec![]);
        let rect = ctx
            .tessellate(output.shapes, output.pixels_per_point)
            .into_iter()
            .find_map(|shape| {
                if let egui::epaint::Primitive::Mesh(mesh) = shape.primitive
                    && mesh.texture_id != egui::TextureId::default()
                {
                    Some(mesh.calc_bounds())
                } else {
                    None
                }
            })
            .expect("displayed image");
        assert!(rect.height() < 320. && (rect.height() / rect.width() - 4.).abs() < 0.1);
        assert!((rect.center().x - 320.).abs() < 2.);
        let pos = rect.center();
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]);
        }
        assert!(egui::Popup::is_any_open(&ctx));
    }

    #[test]
    fn compatibility_outline_double_click_selects_the_property_for_typing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("fixture.json");
        let original = r#"{"first":"😀","target":42,"last":3}"#;
        std::fs::write(&path, original).unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        let mut document = Document::open(temp.path(), &path).unwrap();
        document.preview_mode = false;
        app.documents = vec![document];
        app.active_document = 0;
        app.view = View::Files;

        let ctx = egui::Context::default();
        ctx.style_mut(|style| style.animation_time = 0.0);
        let mut time = 0.0;
        let mut frame = |events| {
            time += 0.02;
            ctx.run(
                egui::RawInput {
                    time: Some(time),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1050.0, 750.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::SidePanel::left("files").show(ctx, |ui| app.explorer(ui));
                    egui::CentralPanel::default().show(ctx, |ui| app.editor(ui));
                },
            )
        };
        let locate = |output: egui::FullOutput, label: &str| {
            output
                .shapes
                .into_iter()
                .find_map(|clipped| {
                    if let egui::Shape::Text(text) = clipped.shape
                        && text.galley.text() == label
                    {
                        Some(text.visual_bounding_rect().center())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| panic!("missing visible label {label}"))
        };
        let pointer = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]
        };
        frame(vec![]);
        let header = locate(
            frame(vec![]),
            if crate::i18n::is_french() {
                "Plan"
            } else {
                "Outline"
            },
        );
        frame(pointer(header, true));
        frame(pointer(header, false));
        frame(vec![]);
        let row = locate(frame(vec![]), "target");
        frame(pointer(row, true));
        frame(pointer(row, false));
        frame(vec![egui::Event::Text("tgt".into())]);
        let filtered = frame(vec![]);
        assert!(!filtered.shapes.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "last")
        ));
        locate(filtered, "target");
        frame(vec![egui::Event::Text("ignored".into())]);
        let _ = locate(
            frame(vec![]),
            if crate::i18n::is_french() {
                "Aucun résultat trouvé."
            } else {
                "No results found."
            },
        );
        frame(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        frame(vec![]);
        for _ in 0..20 {
            frame(vec![]);
        }
        let row = locate(frame(vec![]), "target");
        frame(pointer(row, true));
        frame(pointer(row, false));
        frame(pointer(row, true));
        frame(pointer(row, false));
        frame(vec![egui::Event::Text("replacement".into())]);
        assert_eq!(
            app.documents[0].text,
            r#"{"first":"😀",replacement,"last":3}"#
        );
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
                        let mut prefs = crate::workspace::EditorPreferences::default();
                        close = document_toolbar(ui, &mut document, false, &mut prefs, None).0;
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
            profile_data_dir: None,
            profile_root: None,
            profile_migration: None,
            resume_required: false,
            ssh_retry: Default::default(),
            ssh_tmux_id: None,
            native_host: None,
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            settings_changed: false,
            codex_seed: None,
            conversation_id: None,
            profile_binding: None,
            launch_fingerprint: crate::launch_fingerprint::for_agent("codex", &prefs),
            session_options: None,
            worktree: None,
            pane_name: None,
            broadcast: true,
            scrollback: 10_000,
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
        for (launch, native_host) in [
            (Launch::CodexChat, None),
            (Launch::ClaudeChat, None),
            (
                Launch::Ssh(Host {
                    alias: "fixture".into(),
                    folder: String::new(),
                    ..Default::default()
                }),
                Some(workspace::SavedHostTerminal::default()),
            ),
        ] {
            let mut session = Session {
                profile_data_dir: None,
                profile_root: None,
                profile_migration: None,
                resume_required: false,
                ssh_retry: Default::default(),
                ssh_tmux_id: None,
                native_host,
                restarting: None,
                codex_discovery: None,
                codex_restore: None,
                model_picker: Default::default(),
                last_model: None,
                model_save_attempt: None,
                settings_changed: false,
                codex_seed: None,
                conversation_id: None,
                profile_binding: None,
                launch_fingerprint: None,
                session_options: None,
                worktree: None,
                pane_name: None,
                broadcast: true,
                scrollback: 10_000,
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
            profile_data_dir: None,
            profile_root: None,
            profile_migration: None,
            resume_required: false,
            ssh_retry: Default::default(),
            ssh_tmux_id: None,
            native_host: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            restarting: None,
            codex_seed: None,
            conversation_id: None,
            profile_binding: None,
            settings_changed: false,
            launch_fingerprint: None,
            session_options: None,
            worktree: None,
            pane_name: None,
            broadcast: true,
            scrollback: 10_000,
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
        while session.loading.is_some() || session.profile_migration.is_some() {
            session.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            session
                .error
                .contains("configured session profile has changed")
        );
        assert!(session.terminal.is_none());
        assert_eq!(session.conversation_id.as_deref(), Some(id));
        // The following seed checks exercise a separate new-pane profile context.
        session.profile_binding = None;
        session.profile_root = None;
        session.resume_required = false;
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
            profile_data_dir: None,
            profile_root: None,
            profile_migration: None,
            resume_required: false,
            ssh_retry: Default::default(),
            ssh_tmux_id: None,
            native_host: None,
            codex_seed: None,
            conversation_id: None,
            profile_binding: None,
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            settings_changed: false,
            launch_fingerprint: None,
            session_options: None,
            worktree: None,
            pane_name: None,
            broadcast: true,
            scrollback: 10_000,
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
            profile_data_dir: None,
            profile_root: None,
            profile_migration: None,
            resume_required: false,
            ssh_retry: Default::default(),
            ssh_tmux_id: None,
            native_host: None,
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            settings_changed: false,
            codex_seed: None,
            conversation_id: None,
            profile_binding: None,
            launch_fingerprint: None,
            session_options: None,
            worktree: None,
            pane_name: None,
            broadcast: true,
            scrollback: 10_000,
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
        assert!((session.loading.is_some() || session.profile_migration.is_some()));
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
            profile_data_dir: None,
            profile_root: None,
            profile_migration: None,
            resume_required: false,
            ssh_retry: Default::default(),
            ssh_tmux_id: None,
            native_host: None,
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            settings_changed: false,
            codex_seed: None,
            conversation_id: None,
            profile_binding: None,
            launch_fingerprint: None,
            session_options: None,
            worktree: None,
            pane_name: None,
            broadcast: true,
            scrollback: 10_000,
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

    #[test]
    fn preset_grid_rebuild_survives_workspace_sync_and_preserves_shape() {
        let mut ws = workspace::Workspace {
            active_project: Some(1),
            ..Default::default()
        };
        for id in [10_u64, 20] {
            ws.sessions.push(workspace::SavedSession {
                legacy_number: None,
                ssh_tmux_id: None,
                native_host: None,
                last_model: None,
                codex_seed: None,
                conversation_id: None,
                profile_binding: None,
                launch_fingerprint: None,
                session_options: None,
                native_options: None,
                worktree: None,
                pane_name: None,
                broadcast: true,
                id,
                project: Some(1),
                title: format!("Pane {id}"),
                launch: Launch::Shell,
                cwd: PathBuf::from("."),
            });
        }
        let key = crate::terminal_layout::project_key(Some(1));
        ws.terminal_layouts
            .entry(key.clone())
            .or_default()
            .sync(&[10, 20], [1200.0, 800.0]);

        // Apply `1 x 2` (1 column, 2 stacked rows) over two existing panes; wide room `[1200, 800]`
        // would pick `2 x 1` if `sync_terminal_layouts` overwrote the rebuilt tree.
        let plan = crate::layout_broadcast_ui::plan_preset_grid(
            crate::pane_grid::GridPreset::OneByTwo,
            &[10, 20],
            98,
        );
        assert_eq!(plan.shells_to_spawn, 0);
        let fin = plan.finalize(&[], Some(20));
        assert_eq!(fin.ordered_ids, vec![10, 20]);
        assert_eq!(fin.row_counts, vec![1, 1]);
        assert_eq!(fin.active_id, Some(20));

        ws.terminal_layouts
            .entry(key.clone())
            .or_default()
            .rebuild_ordered(&fin.ordered_ids, &fin.row_counts);
        ws.sync_terminal_layouts([1200.0, 800.0]);

        let tree = ws.terminal_layouts[&key]
            .tree
            .as_ref()
            .expect("preset tree");
        let geom = crate::pane_grid::grid_layout(Some(tree));
        assert_eq!(geom.tiles.len(), 2);
        let top = geom.tiles[0].rect.to_px(1200.0, 800.0, 4.0);
        let bottom = geom.tiles[1].rect.to_px(1200.0, 800.0, 4.0);
        assert!(
            (top.width - 1200.0).abs() < 1.0 && bottom.top > top.top,
            "1 x 2 preset must stay stacked vertically across workspace sync: top={top:?}, bottom={bottom:?}"
        );
    }

    #[test]
    fn supported_help_shortcuts_populate_all_five_groups_and_shell_quote_escapes_paths() {
        for french in [false, true] {
            let groups = crate::help_panel::filter_supported_shortcuts(
                french,
                SUPPORTED_HELP_SHORTCUT_KEYS,
                "",
            );
            assert_eq!(
                groups.len(),
                5,
                "all 5 shortcut reference groups must be populated (french={french})"
            );
            let total_rows: usize = groups.iter().map(|g| g.rows.len()).sum();
            assert_eq!(total_rows, SUPPORTED_HELP_SHORTCUT_KEYS.len());
        }

        assert_eq!(
            shell_quote_path(std::path::Path::new("src/main.rs")),
            "src/main.rs"
        );
        assert_eq!(
            shell_quote_path(std::path::Path::new("C:\\My Docs\\file name.txt")),
            "'C:\\My Docs\\file name.txt'"
        );
        assert_eq!(
            shell_quote_path(std::path::Path::new("it's a test.sh")),
            "'it''s a test.sh'"
        );
    }
}
