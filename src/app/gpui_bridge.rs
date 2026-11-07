//! UI-agnostic bridge exposing `Tessel`'s shared backend state and actions to `crate::gpui_ui`.
//!
//! Ensures 100% zero-duplication state sharing between the primary `GPUI` renderer and the
//! `eframe`/`egui` Compatibility renderer.

use super::{Launch, Overlay, Session, SettingsSection, Store, Tessel, View, floating_terminal};
use crate::i18n::tr;
use crate::pane_grid::{FocusDir, GridPreset, PxRect, SplitDir};
use crate::theme_palettes::ThemeId;
use crate::workspace::{KeepAwakeSetting, RendererPreference, SideTab, Workspace};
use std::{path::PathBuf, sync::mpsc, thread, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GpuiCenterMode {
    NoProjectsWelcome,
    EmptyProjectLauncher,
    WorkGrid,
    DocumentEditor,
    RemoteSftp,
    BrowserTab,
}

#[derive(Clone, Debug)]
pub(crate) struct GpuiToolbarSnapshot {
    pub left_open: bool,
    pub right_open: bool,
    pub broadcast_enabled: bool,
    pub broadcast_count: usize,
    pub floating_open: bool,
    pub share_active: bool,
    pub waiting_agents_count: usize,
    pub unread_notifications: usize,
    pub has_active_project: bool,
    pub active_project_name: Option<String>,
    pub preferred_shell_label: String,
    pub installed_agents: Vec<(String, String)>,
    pub command_palette_open: bool,
    pub error_banner: Option<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiProjectItem {
    pub id: u64,
    pub name: String,
    pub initials: String,
    pub path_display: String,
    pub group_name: Option<String>,
    pub active: bool,
    pub git_branch: Option<String>,
    pub session_count: usize,
    pub running_agents: usize,
    pub waiting_agents: usize,
    pub secondary_worktrees: Vec<(String, String)>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiViewTabItem {
    pub id: String,
    pub title: String,
    pub hint: String,
    pub kind: crate::view_tabs::Kind,
    pub doc_index: Option<usize>,
    pub selected: bool,
    pub dirty: bool,
    pub closable: bool,
    pub preview: bool,
    pub deleted: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiPaneTileSnapshot {
    pub session_id: u64,
    pub title: String,
    pub kind_label: String,
    pub is_agent: bool,
    pub is_native_chat: bool,
    pub active: bool,
    pub maximized: bool,
    pub broadcast: bool,
    pub loading: bool,
    pub exited: bool,
    pub model_chip: Option<String>,
    pub account_chip: Option<String>,
    pub worktree_branch: Option<String>,
    pub cwd_display: String,
    pub rect_px: PxRect,
    pub preview_lines: Vec<String>,
    pub error: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiDividerSnapshot {
    pub key: String,
    pub split_id: String,
    pub dir: SplitDir,
    pub index: usize,
    pub rect_px: PxRect,
    pub split_side_px: f64,
}

#[derive(Clone, Debug)]
pub(crate) struct GpuiStatusBarSnapshot {
    pub visible: bool,
    pub keep_awake: KeepAwakeSetting,
    pub keep_awake_active: bool,
    pub workspace_path: Option<String>,
    pub git_branch: Option<String>,
    pub session_count: usize,
    pub working_agents: usize,
    pub ssh_host_count: usize,
    pub version_build: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiActiveDocumentSummary {
    pub title: String,
    pub path_display: String,
    pub dirty: bool,
    pub preview_mode: bool,
    pub media: bool,
    pub line_count: usize,
    pub preview_lines: Vec<String>,
    pub error: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiSidePanelSummary {
    pub tab: SideTab,
    pub changed_files_count: usize,
    pub staged_count: usize,
    pub tasks_todo: usize,
    pub tasks_doing: usize,
    pub tasks_done: usize,
    pub history_count: usize,
    pub conversations_count: usize,
    pub explorer_root_name: Option<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiWorkspaceSnapshot {
    pub theme: ThemeId,
    pub dark: bool,
    pub french: bool,
    pub font_size: f32,
    pub renderer: RendererPreference,
    pub right_fullscreen: bool,
    pub right_tab: SideTab,
    pub center_mode: GpuiCenterMode,
    pub overlay_title: Option<String>,
    pub toolbar: GpuiToolbarSnapshot,
    pub projects: Vec<GpuiProjectItem>,
    pub view_tabs: Vec<GpuiViewTabItem>,
    pub tiles: Vec<GpuiPaneTileSnapshot>,
    pub dividers: Vec<GpuiDividerSnapshot>,
    pub active_document: Option<GpuiActiveDocumentSummary>,
    pub side_panel: GpuiSidePanelSummary,
    pub status_bar: GpuiStatusBarSnapshot,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) enum GpuiAction {
    ToggleLeftSidebar,
    ToggleRightSidebar,
    SelectSideTab(SideTab),
    ToggleRightFullscreen,
    SelectProject(u64),
    AddProjectPath(PathBuf),
    PromptPickProjectFolder,
    OpenAddProjectDialog,
    NewShell,
    NewShellSplit(u64, SplitDir),
    LaunchDefaultOrShell,
    LaunchAgent(String),
    LaunchCodexChat,
    LaunchClaudeChat,
    FocusSession(u64),
    CloseSession(u64),
    ToggleMaximizeSession(u64),
    ToggleSessionBroadcast(u64),
    ToggleBroadcastMode,
    ApplyGridPreset(GridPreset),
    AutoTilePanes,
    ResizeSplitDivider {
        split_id: String,
        dir: SplitDir,
        index: usize,
        split_side_px: f64,
        delta_px: f64,
    },
    FocusPaneDirection(FocusDir),
    SelectWorkTab,
    SelectDocumentTab(usize),
    CloseDocumentTab(usize),
    SelectBrowserTab,
    CloseBrowserTab,
    ToggleFloatingTerminal,
    ToggleCommandPalette,
    OpenSettings,
    OpenSettingsPage(String),
    CloseOverlays,
    OpenHelp,
    OpenLiveShare,
    OpenGitHubDialog,
    SetTheme(ThemeId),
    SetLanguage {
        french: bool,
    },
    SetRenderer(RendererPreference),
    ZoomFont(i8),
    CycleKeepAwake,
    CycleWorkspace(i32),
    DismissError,
}

fn project_initials(name: &str) -> String {
    let mut out = String::new();
    for part in name
        .split(['-', '_', ' ', '.'])
        .filter(|segment| !segment.is_empty())
    {
        if let Some(ch) = part.chars().next() {
            out.extend(ch.to_uppercase());
            if out.chars().count() >= 2 {
                return out;
            }
        }
    }
    for ch in name.chars() {
        if !ch.is_whitespace() {
            out.extend(ch.to_uppercase());
            if out.chars().count() >= 2 {
                break;
            }
        }
    }
    if out.is_empty() { "WS".into() } else { out }
}

impl Tessel {
    pub(crate) fn new_for_gpui(repaint_cb: impl Fn() + Send + Sync + 'static) -> Self {
        let override_dir = std::env::args()
            .collect::<Vec<_>>()
            .windows(2)
            .find(|pair| pair[0] == "--data-dir")
            .map(|pair| PathBuf::from(&pair[1]));
        Self::new_for_gpui_in_dir(override_dir, true, repaint_cb)
    }

    pub(crate) fn new_for_gpui_in_dir(
        override_dir: Option<PathBuf>,
        probe_environment: bool,
        repaint_cb: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let repaint = std::sync::Arc::new(repaint_cb);
        let (sender, receiver) = mpsc::channel();
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
        crate::i18n::set_language(state.language);
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
        let task_history_store = store
            .as_ref()
            .map(|s| crate::side_panels_model::TaskHistoryStore::load(s.directory()))
            .unwrap_or_default();
        let activity_store = store
            .as_ref()
            .map(|s| crate::side_panels_model::ActivityStore::load(s.directory()))
            .unwrap_or_default();
        let floating_repaint = {
            let repaint = std::sync::Arc::clone(&repaint);
            move || repaint()
        };
        let telemetry_repaint = {
            let repaint = std::sync::Arc::clone(&repaint);
            move || repaint()
        };
        let mut app = Self {
            #[cfg(windows)]
            window_frame: None,
            floating: floating_terminal::State::new_with_repaint(floating_repaint, state.floating),
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
            settings_section: SettingsSection::General,
            settings_nav: Default::default(),
            settings_pages: crate::settings_pages_panel::SettingsPagesState::new(),
            settings_extra: crate::settings_extra_panel::SettingsExtraState::new(),
            claude_managed_accounts,
            codex_managed_accounts,
            provider_secret_flags,
            cli_sign_ins: if probe_environment {
                directories::BaseDirs::new()
                    .map(|b| crate::settings_extra_model::probe_cli_sign_ins(b.home_dir()))
                    .unwrap_or_default()
            } else {
                Default::default()
            },
            scanned_skills: Vec::new(),
            installed_mcp_servers: Vec::new(),
            context_scan: Default::default(),
            devtools_statuses: Vec::new(),
            devtools_rx: None,
            mcp_test_rx: None,
            automations_store,
            last_automation_minute: None,
            appearance_panel: Default::default(),
            command_palette: Default::default(),
            command_palette_focused: false,
            toolbar_menu_open: false,
            pane_context_menu: None,
            pane_context_menu_just_opened: false,
            finder: Default::default(),
            grid_room: [960.0, 640.0],
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
            scm_state: Default::default(),
            scm_panel: Default::default(),
            commit_in_flight: false,
            pending_scm_discard: None,
            pending_agent_install: None,
            active_agent_installs: Vec::new(),
            rename_project_modal: None,
            pending_close_session: None,
            pending_remove_project: None,
            pending_quick_command: None,
            worktree_state: Default::default(),
            worktree_cleanup: None,
            pending_delete_worktree: None,
            git_username: None,
            git_username_rx: if probe_environment {
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || {
                    let mut cmd = std::process::Command::new("git");
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
            } else {
                None
            },
            right_fullscreen: false,
            remember_next_project_launch,
            wsl_available,
            chats: Vec::new(),
            claude_chats: Vec::new(),
            share: crate::share_panel::SharePanel::default(),
            remote: crate::remote_panel::RemotePanel::default(),
            status_bar: Default::default(),
            status_telemetry: crate::status_metrics::StatusTelemetry::spawn(telemetry_repaint),
            keep_awake: Default::default(),
            side_panels_ui: crate::side_panels_ui::SidePanelsUiState::new(),
            dialogs_toolbar_ui: crate::dialogs_toolbar_ui::DialogsToolbarUiState::new(),
            teams_hub: Default::default(),
            teams_ui: Default::default(),
            notifications: Default::default(),
            notified_waiting_panes: Default::default(),
            task_history_store,
            activity_store,
            past_sessions_cache: Vec::new(),
            scm_commits_cache: Vec::new(),
            scm_commits_project: None,
            #[cfg(windows)]
            browser_tab_open: false,
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
            app.active_session = app
                .sessions
                .iter()
                .find(|s| s.project == app.state.active_project)
                .map(|s| s.id);
        }
        if probe_environment {
            app.detect_agents();
            app.refresh_project();
            app.worktree_state.refresh_all(&app.state.projects);
        }
        app
    }

    pub(crate) fn poll_for_gpui(&mut self) {
        self.native_hosts.poll();
        self.poll();
        self.floating.poll();
        self.sync_live_share();
    }

    pub(crate) fn gpui_snapshot(
        &mut self,
        grid_room_w: f32,
        grid_room_h: f32,
        divider_px: f32,
    ) -> GpuiWorkspaceSnapshot {
        use crate::status_metrics::should_keep_awake;
        use crate::view_tabs_browser_model::{OpenTabLabelInput, base_tab_label};

        let room = [
            f64::from(grid_room_w.max(320.0)),
            f64::from(grid_room_h.max(180.0)),
        ];
        self.grid_room = room;
        let french = self.state.language == crate::i18n::Language::French;
        let theme = self.state.theme.unwrap_or(ThemeId::Classic);

        let project_session_ids: Vec<u64> = self
            .sessions
            .iter()
            .filter(|s| s.project == self.state.active_project)
            .map(|s| s.id)
            .collect();
        if self.active_session.is_none()
            || !project_session_ids
                .iter()
                .any(|id| Some(*id) == self.active_session)
        {
            self.active_session = project_session_ids.first().copied();
        }
        let layout_key = crate::terminal_layout::project_key(self.state.active_project);
        let maximized_id = if project_session_ids.is_empty() {
            None
        } else {
            let layout = self
                .state
                .terminal_layouts
                .entry(layout_key.clone())
                .or_default();
            layout.sync(&project_session_ids, room);
            layout.maximized
        };

        let mut any_agent_working = false;
        let mut total_waiting_agents = 0usize;
        let mut active_working_agents = 0usize;
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
            if is_waiting {
                total_waiting_agents += 1;
            }
            if s.project == self.state.active_project && is_working {
                active_working_agents += 1;
            }
        }

        let keep_awake_active = self.keep_awake.sync(should_keep_awake(
            self.state.status_bar.keep_awake.into_mode(),
            any_agent_working,
        ));

        let mut tiles = Vec::new();
        let mut dividers = Vec::new();
        if let Some(max_sid) = maximized_id.filter(|id| project_session_ids.contains(id)) {
            if let Some(sess) = self.sessions.iter().find(|s| s.id == max_sid) {
                tiles.push(self.build_pane_tile_snapshot(
                    sess,
                    true,
                    PxRect {
                        left: 0.0,
                        top: 0.0,
                        width: room[0],
                        height: room[1],
                    },
                ));
            }
        } else if let Some(layout) = self.state.terminal_layouts.get(&layout_key) {
            let geom = crate::pane_grid::grid_layout(layout.tree.as_ref());
            let div_px = f64::from(divider_px);
            for tile in &geom.tiles {
                if let Ok(sid) = tile.leaf.id.parse::<u64>()
                    && let Some(sess) = self.sessions.iter().find(|s| s.id == sid)
                {
                    let rect_px = tile.rect.to_px(room[0], room[1], div_px);
                    tiles.push(self.build_pane_tile_snapshot(sess, false, rect_px));
                }
            }
            for div in &geom.dividers {
                let rect_px = div.rect.to_px(room[0], room[1], div_px);
                let split_px = div.split_rect.to_px(room[0], room[1], div_px);
                let split_side_px = match div.dir {
                    SplitDir::Row => split_px.width,
                    SplitDir::Col => split_px.height,
                };
                dividers.push(GpuiDividerSnapshot {
                    key: div.key.clone(),
                    split_id: div.split_id.clone(),
                    dir: div.dir,
                    index: div.index,
                    rect_px,
                    split_side_px,
                });
            }
        }

        let projects: Vec<GpuiProjectItem> = self
            .state
            .projects
            .iter()
            .map(|p| {
                let summary = self.worktree_state.summary(p.id);
                let git_branch = summary.map(|s| s.branch.clone()).filter(|b| !b.is_empty());
                let secondary_worktrees = summary
                    .map(|s| {
                        s.worktrees
                            .iter()
                            .filter(|wt| !wt.is_primary)
                            .map(|wt| (wt.branch.clone(), wt.path.clone()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let mut sess_count = 0usize;
                let mut running_agents = 0usize;
                let mut waiting_agents = 0usize;
                for s in self.sessions.iter().filter(|s| s.project == Some(p.id)) {
                    sess_count += 1;
                    if self.session_is_running_agent(s.id) {
                        running_agents += 1;
                    }
                    let waiting = match &s.launch {
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
                    if waiting {
                        waiting_agents += 1;
                    }
                }
                GpuiProjectItem {
                    id: p.id,
                    name: p.name.clone(),
                    initials: project_initials(&p.name),
                    path_display: p.path.to_string_lossy().into_owned(),
                    group_name: self.state.project_groups.get(&p.id).cloned(),
                    active: self.state.active_project == Some(p.id),
                    git_branch,
                    session_count: sess_count,
                    running_agents,
                    waiting_agents,
                    secondary_worktrees,
                }
            })
            .collect();

        let paths: Vec<String> = self
            .documents
            .iter()
            .map(|d| d.path.to_string_lossy().into_owned())
            .collect();
        let tab_inputs: Vec<OpenTabLabelInput<'_>> = paths
            .iter()
            .map(|path| OpenTabLabelInput {
                tab_path: path,
                diff: None,
            })
            .collect();
        let mut view_tabs = Vec::with_capacity(self.documents.len() + 2);
        view_tabs.push(GpuiViewTabItem {
            id: "work".into(),
            title: tr("Terminal").into(),
            hint: tr("Terminal").into(),
            kind: crate::view_tabs::Kind::Terminal,
            doc_index: None,
            selected: self.view == View::Work,
            dirty: false,
            closable: false,
            preview: false,
            deleted: false,
        });
        #[cfg(windows)]
        if self.browser_tab_open || self.view == View::Browser {
            view_tabs.push(GpuiViewTabItem {
                id: "browser".into(),
                title: tr("Browser").into(),
                hint: tr("Browser").into(),
                kind: crate::view_tabs::Kind::Browser,
                doc_index: None,
                selected: self.view == View::Browser,
                dirty: false,
                closable: true,
                preview: false,
                deleted: false,
            });
        }
        for (i, doc) in self.documents.iter().enumerate() {
            let computed_label = base_tab_label(&tab_inputs[i], &tab_inputs);
            let title = doc
                .diff_title
                .as_deref()
                .unwrap_or(computed_label.as_str())
                .to_owned();
            let kind = if doc.diff_title.is_some() {
                crate::view_tabs::Kind::Diff
            } else {
                crate::view_tabs::Kind::Editor
            };
            view_tabs.push(GpuiViewTabItem {
                id: format!("doc:{i}"),
                title,
                hint: paths[i].clone(),
                kind,
                doc_index: Some(i),
                selected: self.view == View::Files && self.active_document == i,
                dirty: doc.dirty(),
                closable: true,
                preview: doc.unpinned_preview,
                deleted: doc.deleted_on_disk,
            });
        }

        let center_mode = match self.view {
            View::Files if !self.documents.is_empty() => GpuiCenterMode::DocumentEditor,
            View::Remote => GpuiCenterMode::RemoteSftp,
            #[cfg(windows)]
            View::Browser => GpuiCenterMode::BrowserTab,
            _ => {
                if self.state.projects.is_empty() && project_session_ids.is_empty() {
                    GpuiCenterMode::NoProjectsWelcome
                } else if project_session_ids.is_empty() {
                    GpuiCenterMode::EmptyProjectLauncher
                } else {
                    GpuiCenterMode::WorkGrid
                }
            }
        };

        let active_document = (self.view == View::Files)
            .then(|| self.documents.get(self.active_document))
            .flatten()
            .map(|doc| {
                let title = doc
                    .diff_title
                    .clone()
                    .or_else(|| {
                        doc.path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                    })
                    .unwrap_or_else(|| tr("Editor").into());
                let line_count = if doc.media {
                    0
                } else {
                    doc.text.lines().count().max(1)
                };
                let preview_lines: Vec<String> = doc
                    .text
                    .lines()
                    .take(48)
                    .map(|l| l.replace('\t', "    "))
                    .collect();
                GpuiActiveDocumentSummary {
                    title,
                    path_display: doc.path.to_string_lossy().into_owned(),
                    dirty: doc.dirty(),
                    preview_mode: doc.preview_mode,
                    media: doc.media,
                    line_count,
                    preview_lines,
                    error: doc.error.clone(),
                }
            });

        let preferred_shell_label = match self.state.launcher_prefs.preferred_shell.as_deref() {
            Some("cmd") if cfg!(windows) => "CMD".into(),
            Some("wsl") if cfg!(windows) => "WSL".into(),
            _ if cfg!(windows) => "PowerShell".into(),
            _ => "Shell".into(),
        };
        let mut installed_agents: Vec<(String, String)> = self
            .agents
            .entries()
            .iter()
            .filter(|(id, installed)| {
                *installed && crate::agent_preferences::enabled(&self.state, id)
            })
            .map(|(id, _)| {
                let label = crate::agent_catalog::find(id)
                    .map_or_else(|| id.clone(), |entry| entry.name.to_owned());
                (id.clone(), label)
            })
            .collect();
        for custom in self.state.custom_agents.iter().filter(|c| c.enabled) {
            installed_agents.push((custom.id.clone(), custom.name.clone()));
        }

        let broadcast_count = self
            .sessions
            .iter()
            .filter(|s| s.project == self.state.active_project && s.broadcast)
            .count();

        let (changed_files_count, staged_count) = match &self.scm_state.result {
            Some(Ok(parsed)) => {
                let staged = parsed
                    .entries
                    .iter()
                    .filter(|e| e.area == crate::scm_changes_panel::ScmArea::Staged)
                    .count();
                (parsed.entries.len(), staged)
            }
            _ => (0, 0),
        };

        let active_pid = self.state.active_project;
        let mut tasks_todo = 0usize;
        let mut tasks_doing = 0usize;
        let mut tasks_done = 0usize;
        if let Some(pid) = active_pid {
            for t in self.state.tasks.iter().filter(|t| t.project == pid) {
                match t.column {
                    0 => tasks_todo += 1,
                    1 | 2 => tasks_doing += 1,
                    _ => tasks_done += 1,
                }
            }
        }

        let active_proj_ref = self.state.active();
        let active_branch = active_pid
            .and_then(|pid| self.worktree_state.summary(pid))
            .map(|s| s.branch.clone())
            .filter(|b| !b.is_empty());

        let overlay_title = if self.command_palette.open {
            Some(tr("Command Palette").into())
        } else {
            self.overlay.map(|ov| {
                tr(match ov {
                    Overlay::Settings => "Settings",
                    Overlay::Share => "Live Share",
                    Overlay::Help => "Keyboard shortcuts",
                    Overlay::Launcher => "Open a terminal or agent",
                    Overlay::RenameAgent => "Rename pane",
                })
                .into()
            })
        };

        GpuiWorkspaceSnapshot {
            theme,
            dark: self.state.dark,
            french,
            font_size: self.state.font_size,
            renderer: self.state.renderer,
            right_fullscreen: self.right_fullscreen,
            right_tab: self.state.panels.right_tab,
            center_mode,
            overlay_title,
            toolbar: GpuiToolbarSnapshot {
                left_open: self.state.panels.left_open,
                right_open: self.state.panels.right_open,
                broadcast_enabled: self.state.broadcast,
                broadcast_count,
                floating_open: self.floating.ui.is_open(),
                share_active: self.overlay == Some(Overlay::Share)
                    || self.share.has_pending_ask()
                    || self.share.is_pill_popover_open(),
                waiting_agents_count: total_waiting_agents,
                unread_notifications: self.notifications.unread_count(),
                has_active_project: active_proj_ref.is_some(),
                active_project_name: active_proj_ref.map(|p| p.name.clone()),
                preferred_shell_label,
                installed_agents,
                command_palette_open: self.command_palette.open,
                error_banner: (!self.error.is_empty()).then(|| self.error.clone()),
            },
            projects,
            view_tabs,
            tiles,
            dividers,
            active_document,
            side_panel: GpuiSidePanelSummary {
                tab: self.state.panels.right_tab,
                changed_files_count,
                staged_count,
                tasks_todo,
                tasks_doing,
                tasks_done,
                history_count: self.task_history_store.records.len(),
                conversations_count: self.activity_store.messages.len(),
                explorer_root_name: active_proj_ref.map(|p| p.name.clone()),
            },
            status_bar: GpuiStatusBarSnapshot {
                visible: self.state.status_bar.visible,
                keep_awake: self.state.status_bar.keep_awake,
                keep_awake_active,
                workspace_path: active_proj_ref.map(|p| p.path.to_string_lossy().into_owned()),
                git_branch: active_branch,
                session_count: project_session_ids.len(),
                working_agents: active_working_agents,
                ssh_host_count: self.state.hosts.len(),
                version_build: format!("v{} {}", env!("CARGO_PKG_VERSION"), env!("TESSEL_BUILD")),
            },
        }
    }

    fn build_pane_tile_snapshot(
        &self,
        sess: &Session,
        maximized: bool,
        rect_px: PxRect,
    ) -> GpuiPaneTileSnapshot {
        let is_agent = matches!(
            sess.launch,
            Launch::Agent(_) | Launch::CodexChat | Launch::ClaudeChat
        );
        let is_native_chat = sess.launch.native_chat().is_some();
        let kind_label = match &sess.launch {
            Launch::Shell => "Shell".into(),
            Launch::CodexChat => "Codex Chat".into(),
            Launch::ClaudeChat => "Claude Chat".into(),
            Launch::Agent(agent) => agent.clone(),
            Launch::Ssh(host) => format!("SSH · {}", host.alias),
        };
        let model_chip = sess
            .last_model
            .as_ref()
            .map(|m| m.model.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                sess.session_options
                    .as_ref()
                    .and_then(|v| v.get("model"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            });
        let account_chip = match &sess.launch {
            Launch::Agent(agent) if agent == "claude" => self
                .claude_managed_accounts
                .selected_id
                .as_deref()
                .and_then(|id| {
                    self.claude_managed_accounts
                        .accounts
                        .iter()
                        .find(|a| a.id == id)
                })
                .map(|a| a.label.clone()),
            Launch::Agent(agent) if agent == "codex" => self
                .codex_managed_accounts
                .selected_id
                .as_deref()
                .and_then(|id| {
                    self.codex_managed_accounts
                        .accounts
                        .iter()
                        .find(|a| a.id == id)
                })
                .map(|a| a.label.clone()),
            _ => None,
        };
        let preview_lines: Vec<String> = if let Some(term) = &sess.terminal {
            let out = term.output();
            let all: Vec<&str> = out.lines().collect();
            let start = all.len().saturating_sub(24);
            all[start..].iter().map(|s| (*s).to_owned()).collect()
        } else if sess.loading.is_some() {
            vec![tr("Starting session…").into()]
        } else {
            Vec::new()
        };

        GpuiPaneTileSnapshot {
            session_id: sess.id,
            title: sess.label().to_owned(),
            kind_label,
            is_agent,
            is_native_chat,
            active: self.active_session == Some(sess.id),
            maximized,
            broadcast: sess.broadcast,
            loading: sess.loading.is_some() || sess.restarting.is_some(),
            exited: sess.terminal.as_ref().is_some_and(|t| t.exited()),
            model_chip,
            account_chip,
            worktree_branch: sess.worktree.as_ref().map(|w| w.branch.clone()),
            cwd_display: sess.cwd.to_string_lossy().into_owned(),
            rect_px,
            preview_lines,
            error: sess.error.clone(),
        }
    }

    pub(crate) fn apply_gpui_action(&mut self, action: GpuiAction) {
        match action {
            GpuiAction::ToggleLeftSidebar => {
                self.state.panels.left_open = !self.state.panels.left_open;
                self.save();
            }
            GpuiAction::ToggleRightSidebar => {
                self.state.panels.right_open = !self.state.panels.right_open;
                if !self.state.panels.right_open {
                    self.right_fullscreen = false;
                }
                self.save();
            }
            GpuiAction::SelectSideTab(tab) => {
                self.state.panels.toggle_right(tab);
                if !self.state.panels.right_open {
                    self.right_fullscreen = false;
                }
                self.save();
            }
            GpuiAction::ToggleRightFullscreen => {
                if !self.state.panels.right_open {
                    self.state.panels.right_open = true;
                }
                self.right_fullscreen = !self.right_fullscreen;
            }
            GpuiAction::SelectProject(id) => {
                if self.state.projects.iter().any(|p| p.id == id)
                    && self.state.active_project != Some(id)
                {
                    self.state.active_project = Some(id);
                    self.active_session = self
                        .sessions
                        .iter()
                        .find(|s| s.project == Some(id))
                        .map(|s| s.id);
                    self.view = View::Work;
                    self.refresh_project();
                    self.save();
                }
            }
            GpuiAction::AddProjectPath(path) => match self.state.add_project(&path) {
                Ok(_) => {
                    self.view = View::Work;
                    self.refresh_project();
                    self.save();
                }
                Err(e) => self.error = e,
            },
            GpuiAction::PromptPickProjectFolder => {
                self.choose_project();
            }
            GpuiAction::OpenAddProjectDialog => {
                self.open_add_project_modal();
            }
            GpuiAction::NewShell => {
                self.new_session_placed(Launch::Shell, None);
            }
            GpuiAction::NewShellSplit(target_id, dir) => {
                self.new_session_placed(Launch::Shell, Some((target_id, dir)));
            }
            GpuiAction::LaunchDefaultOrShell => {
                let launch = self.default_new_launch();
                self.new_session_placed(launch, None);
            }
            GpuiAction::LaunchAgent(agent_id) => {
                self.new_session_placed(Launch::Agent(agent_id), None);
            }
            GpuiAction::LaunchCodexChat => {
                if let Some(pid) = self.state.active_project
                    && let Some(proj) = self.state.projects.iter().find(|p| p.id == pid).cloned()
                {
                    self.new_session_placed(Launch::CodexChat, None);
                    if let Some(pane_id) = self.active_session {
                        let mut chat = crate::chat::Chat::new(pid, proj.path);
                        chat.pane = pane_id;
                        self.chats.push(chat);
                    }
                } else {
                    self.error = tr("Select a project for native chat.").into();
                }
            }
            GpuiAction::LaunchClaudeChat => {
                if let Some(pid) = self.state.active_project
                    && let Some(proj) = self.state.projects.iter().find(|p| p.id == pid).cloned()
                {
                    self.new_session_placed(Launch::ClaudeChat, None);
                    if let Some(pane_id) = self.active_session {
                        let mut chat = crate::claude::Chat::new(pid, proj.path, None);
                        chat.pane = pane_id;
                        self.claude_chats.push(chat);
                    }
                } else {
                    self.error = tr("Select a project for native chat.").into();
                }
            }
            GpuiAction::FocusSession(id) => {
                if self.sessions.iter().any(|s| s.id == id) {
                    self.active_session = Some(id);
                    self.view = View::Work;
                }
            }
            GpuiAction::CloseSession(id) => {
                self.close_session(id);
            }
            GpuiAction::ToggleMaximizeSession(id) => {
                let key = crate::terminal_layout::project_key(self.state.active_project);
                let layout = self.state.terminal_layouts.entry(key).or_default();
                if layout.maximized == Some(id) {
                    layout.maximized = None;
                } else {
                    layout.maximized = Some(id);
                    self.active_session = Some(id);
                }
                self.save();
            }
            GpuiAction::ToggleSessionBroadcast(id) => {
                if let Some(sess) = self.sessions.iter_mut().find(|s| s.id == id) {
                    sess.broadcast = !sess.broadcast;
                    self.save();
                }
            }
            GpuiAction::ToggleBroadcastMode => {
                self.state.broadcast = !self.state.broadcast;
                self.save();
            }
            GpuiAction::ApplyGridPreset(preset) => {
                self.apply_grid_preset(preset);
            }
            GpuiAction::AutoTilePanes => {
                self.tile_sessions(None);
                self.save();
            }
            GpuiAction::ResizeSplitDivider {
                split_id,
                dir: _,
                index,
                split_side_px,
                delta_px,
            } => {
                let key = crate::terminal_layout::project_key(self.state.active_project);
                if let Some(layout) = self.state.terminal_layouts.get_mut(&key)
                    && layout.resize(&split_id, index, split_side_px, delta_px, None)
                {
                    self.save();
                }
            }
            GpuiAction::FocusPaneDirection(dir) => {
                let key = crate::terminal_layout::project_key(self.state.active_project);
                let Some(active_id) = self.active_session else {
                    return;
                };
                if let Some(layout) = self.state.terminal_layouts.get(&key) {
                    let geom = crate::pane_grid::grid_layout(layout.tree.as_ref());
                    let panes: Vec<(String, PxRect)> = geom
                        .tiles
                        .iter()
                        .map(|t| {
                            (
                                t.leaf.id.clone(),
                                t.rect.to_px(self.grid_room[0], self.grid_room[1], 4.0),
                            )
                        })
                        .collect();
                    if let Some(next_str) =
                        crate::pane_grid::move_focus(&panes, &active_id.to_string(), dir)
                        && let Ok(next_id) = next_str.parse::<u64>()
                    {
                        self.active_session = Some(next_id);
                    }
                }
            }
            GpuiAction::SelectWorkTab => {
                self.view = View::Work;
            }
            GpuiAction::SelectDocumentTab(idx) => {
                if idx < self.documents.len() {
                    self.active_document = idx;
                    self.view = View::Files;
                }
            }
            GpuiAction::CloseDocumentTab(idx) => {
                self.request_document_close(idx);
            }
            GpuiAction::SelectBrowserTab => {
                #[cfg(windows)]
                {
                    self.browser_tab_open = true;
                    self.view = View::Browser;
                }
            }
            GpuiAction::CloseBrowserTab => {
                #[cfg(windows)]
                {
                    self.browser_tab_open = false;
                    if self.view == View::Browser {
                        self.view = if self.documents.is_empty() {
                            View::Work
                        } else {
                            View::Files
                        };
                    }
                }
            }
            GpuiAction::ToggleFloatingTerminal => {
                self.toggle_floating_headless();
            }
            GpuiAction::ToggleCommandPalette => {
                self.command_palette.open = !self.command_palette.open;
            }
            GpuiAction::OpenSettings => {
                if self.overlay == Some(Overlay::Settings) {
                    self.overlay = None;
                } else {
                    self.overlay = Some(Overlay::Settings);
                }
            }
            GpuiAction::OpenSettingsPage(page_id) => {
                self.open_settings_by_page_id(&page_id);
            }
            GpuiAction::CloseOverlays => {
                self.command_palette.open = false;
                self.overlay = None;
                self.dialogs_toolbar_ui.add_project_open = false;
                self.dialogs_toolbar_ui.github_open = false;
            }
            GpuiAction::OpenHelp => {
                if self.overlay == Some(Overlay::Help) {
                    self.overlay = None;
                } else {
                    self.overlay = Some(Overlay::Help);
                }
            }
            GpuiAction::OpenLiveShare => {
                if self.overlay == Some(Overlay::Share) {
                    self.overlay = None;
                } else {
                    self.overlay = Some(Overlay::Share);
                }
            }
            GpuiAction::OpenGitHubDialog => {
                self.open_github_modal(self.state.active_project);
            }
            GpuiAction::SetTheme(theme) => {
                if self.state.theme != Some(theme) {
                    self.state.theme = Some(theme);
                    self.save();
                }
            }
            GpuiAction::SetLanguage { french } => {
                let lang = if french {
                    crate::i18n::Language::French
                } else {
                    crate::i18n::Language::English
                };
                if self.state.language != lang {
                    self.state.language = lang;
                    crate::i18n::set_language(lang);
                    self.save();
                }
            }
            GpuiAction::SetRenderer(pref) => {
                if self.state.renderer != pref {
                    self.state.renderer = pref;
                    self.save();
                }
            }
            GpuiAction::ZoomFont(delta) => {
                self.zoom_font(delta);
            }
            GpuiAction::CycleKeepAwake => {
                self.state.status_bar.keep_awake = match self.state.status_bar.keep_awake {
                    KeepAwakeSetting::Off => KeepAwakeSetting::Agents,
                    KeepAwakeSetting::Agents => KeepAwakeSetting::On,
                    KeepAwakeSetting::On => KeepAwakeSetting::Off,
                };
                self.save();
            }
            GpuiAction::CycleWorkspace(step) => {
                self.cycle_workspace(step);
            }
            GpuiAction::DismissError => {
                self.error.clear();
            }
        }
    }
}
