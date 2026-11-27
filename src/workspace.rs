//! Persistent, local-only state. Restoring metadata never starts a process.
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub type Result<T> = std::result::Result<T, String>;

const fn default_pane_broadcast() -> bool {
    true
}

use crate::theme_palettes::FileIconStyle;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RendererPreference {
    #[default]
    #[serde(alias = "gpui", alias = "auto")]
    AutoGpui,
    #[serde(alias = "compat", alias = "egui")]
    CompatEgui,
}

impl RendererPreference {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AutoGpui => "auto_gpui",
            Self::CompatEgui => "compat_egui",
        }
    }

    #[must_use]
    pub fn from_flag(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "gpui" | "auto" | "auto_gpui" | "default" => Some(Self::AutoGpui),
            "compat" | "egui" | "compat_egui" | "opengl" => Some(Self::CompatEgui),
            _ => None,
        }
    }

    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::AutoGpui, false) => "GPUI (GPU-accelerated, 1:1)",
            (Self::AutoGpui, true) => "GPUI (accéléré GPU, 1:1)",
            (Self::CompatEgui, false) => "Compatibility (OpenGL / egui)",
            (Self::CompatEgui, true) => "Compatibilité (OpenGL / egui)",
        }
    }

    #[must_use]
    pub const fn description(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::AutoGpui, false) => {
                "Primary DirectX / Metal / Vulkan renderer with automatic fallback to Compatibility on older GPUs or Remote Desktop."
            }
            (Self::AutoGpui, true) => {
                "Moteur principal DirectX / Metal / Vulkan avec repli automatique vers Compatibilité sur anciens GPU ou Bureau à distance."
            }
            (Self::CompatEgui, false) => {
                "Universal OpenGL compatibility renderer for older hardware, VMs, and headless screenshot fixtures."
            }
            (Self::CompatEgui, true) => {
                "Moteur de compatibilité OpenGL universel pour matériel ancien, machines virtuelles et captures."
            }
        }
    }
}

/// Resolves the startup UI renderer from CLI flags (`--renderer gpui|compat`),
/// `TESSEL_RENDERER` environment override, visual-test screenshot fixtures, and
/// saved workspace preference (`AutoGpui` by default).
#[must_use]
pub fn resolve_startup_renderer(
    args: &[String],
    env_override: Option<&str>,
    saved: RendererPreference,
    visual_test_fixture: bool,
) -> RendererPreference {
    for (idx, arg) in args.iter().enumerate() {
        if let Some(val) = arg.strip_prefix("--renderer=")
            && let Some(pref) = RendererPreference::from_flag(val)
        {
            return pref;
        }
        if arg == "--renderer"
            && let Some(next) = args.get(idx + 1)
            && let Some(pref) = RendererPreference::from_flag(next)
        {
            return pref;
        }
    }
    if let Some(env_val) = env_override
        && let Some(pref) = RendererPreference::from_flag(env_val)
    {
        return pref;
    }
    if visual_test_fixture {
        return RendererPreference::CompatEgui;
    }
    saved
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SideTab {
    Dashboard,
    #[default]
    Files,
    Changes,
    Tasks,
    #[serde(alias = "taskHistory")]
    TaskHistory,
    History,
    Conversations,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Panels {
    pub left_open: bool,
    pub right_open: bool,
    pub right_tab: SideTab,
}

const fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeepAwakeSetting {
    #[default]
    Off,
    #[serde(alias = "agent")]
    Agents,
    On,
}

impl KeepAwakeSetting {
    #[must_use]
    pub const fn into_mode(self) -> crate::sidebar_model::KeepAwakeMode {
        match self {
            Self::Off => crate::sidebar_model::KeepAwakeMode::Off,
            Self::Agents => crate::sidebar_model::KeepAwakeMode::Agents,
            Self::On => crate::sidebar_model::KeepAwakeMode::On,
        }
    }

    #[must_use]
    pub const fn from_mode(mode: crate::sidebar_model::KeepAwakeMode) -> Self {
        match mode {
            crate::sidebar_model::KeepAwakeMode::Off => Self::Off,
            crate::sidebar_model::KeepAwakeMode::Agents => Self::Agents,
            crate::sidebar_model::KeepAwakeMode::On => Self::On,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct StatusBarPreferences {
    #[serde(default = "default_true")]
    pub visible: bool,
    pub keep_awake: KeepAwakeSetting,
    #[serde(default = "default_true")]
    pub show_ssh: bool,
    #[serde(default = "default_true")]
    pub show_resources: bool,
    #[serde(default = "default_true")]
    pub show_ports: bool,
}

impl Default for StatusBarPreferences {
    fn default() -> Self {
        Self {
            visible: true,
            keep_awake: KeepAwakeSetting::Off,
            show_ssh: true,
            show_resources: true,
            show_ports: true,
        }
    }
}

/// Floating-terminal preferences are separate from the project pane tree.
/// They contain no terminal output, process authority or remote credentials.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct FloatingPreferences {
    pub open: bool,
    pub height: f64,
}

impl Default for FloatingPreferences {
    fn default() -> Self {
        Self {
            open: false,
            height: crate::palette::FLOATING_DEFAULT_HEIGHT,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct SidebarPreferences {
    pub sort_by_activity: bool,
    pub compact_rows: bool,
    pub hide_sleeping: bool,
    pub hide_agent_task_line: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct LauncherPreferences {
    pub preferred_shell: Option<String>,
    pub remember_default: Option<String>,
}

const fn default_tab_size() -> u8 {
    4
}

const fn default_auto_save_delay_ms() -> u32 {
    1000
}

const fn default_terminal_scrollback() -> usize {
    5000
}

const fn default_terminal_line_height() -> f32 {
    1.0
}

fn default_branch_prefix_custom() -> String {
    "agent".into()
}

const fn default_usage_refresh_minutes() -> u16 {
    5
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct EditorPreferences {
    pub font_size: u8,
    #[serde(default = "default_true")]
    pub syntax_highlighting: bool,
    #[serde(default = "default_true")]
    pub line_numbers: bool,
    pub word_wrap: bool,
    #[serde(default = "default_tab_size")]
    pub tab_size: u8,
    pub auto_save: bool,
    #[serde(default = "default_auto_save_delay_ms")]
    pub auto_save_delay_ms: u32,
    #[serde(default = "default_true")]
    pub preview_tabs: bool,
    #[serde(default = "default_true")]
    pub show_git_ignored: bool,
    pub compact_folders: bool,
    pub outline_follow_cursor: bool,
    pub outline_filter_on_type: bool,
    #[serde(default = "default_true")]
    pub open_editors_shown: bool,
    #[serde(default = "default_true")]
    pub explorer_folder_shown: bool,
    pub explorer_outline_height: u16,
    pub explorer_timeline_height: u16,
    pub timeline_shown: bool,
    pub timeline_git: bool,
    pub timeline_local: bool,
    pub outline_sort: crate::outline_tree::Sort,
    pub diff_word_wrap: bool,
}

impl Default for EditorPreferences {
    fn default() -> Self {
        Self {
            font_size: 12,
            syntax_highlighting: true,
            line_numbers: true,
            word_wrap: true,
            tab_size: 4,
            auto_save: false,
            auto_save_delay_ms: 1000,
            preview_tabs: true,
            show_git_ignored: true,
            compact_folders: false,
            outline_follow_cursor: true,
            outline_filter_on_type: true,
            open_editors_shown: true,
            explorer_folder_shown: true,
            explorer_outline_height: 220,
            explorer_timeline_height: 220,
            timeline_shown: false,
            timeline_git: true,
            timeline_local: true,
            outline_sort: Default::default(),
            diff_word_wrap: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct GeneralPreferences {
    #[serde(default = "default_true")]
    pub restore_workspaces: bool,
    #[serde(default = "default_true")]
    pub confirm_close_agent: bool,
    #[serde(default = "default_true")]
    pub confirm_quit_running: bool,
    pub workspace_dir: String,
    #[serde(default = "default_true")]
    pub confirm_delete_workspace: bool,
}

impl Default for GeneralPreferences {
    fn default() -> Self {
        Self {
            restore_workspaces: true,
            confirm_close_agent: true,
            confirm_quit_running: true,
            workspace_dir: String::new(),
            confirm_delete_workspace: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerminalCursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalPreferences {
    #[serde(default = "default_terminal_scrollback")]
    pub scrollback: usize,
    #[serde(default = "default_terminal_line_height")]
    pub line_height: f32,
    pub cursor_style: TerminalCursorStyle,
    #[serde(default = "default_true")]
    pub cursor_blink: bool,
    #[serde(default = "default_true")]
    pub tile_new_panes: bool,
    #[serde(default = "default_true")]
    pub copy_on_select: bool,
    #[serde(default = "default_true")]
    pub right_click_paste: bool,
    #[serde(default = "default_true")]
    pub confirm_multiline_paste: bool,
    pub focus_follows_mouse: bool,
}

impl Default for TerminalPreferences {
    fn default() -> Self {
        Self {
            scrollback: 5000,
            line_height: 1.0,
            cursor_style: TerminalCursorStyle::Block,
            cursor_blink: true,
            tile_new_panes: true,
            copy_on_select: true,
            right_click_paste: true,
            confirm_multiline_paste: true,
            focus_follows_mouse: false,
        }
    }
}

#[path = "workspace/browser_preferences.rs"]
mod browser_preferences;
pub use browser_preferences::{BrowserPreferences, OpenLinksTarget};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BranchPrefixMode {
    GitUsername,
    #[default]
    Custom,
    None,
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScmOrderSetting {
    #[default]
    ChangesFirst,
    StagedFirst,
    UntrackedFirst,
}

impl ScmOrderSetting {
    #[must_use]
    pub const fn into_panel(self) -> crate::scm_changes_panel::ScmGroupOrder {
        match self {
            Self::ChangesFirst => crate::scm_changes_panel::ScmGroupOrder::ChangesFirst,
            Self::StagedFirst => crate::scm_changes_panel::ScmGroupOrder::StagedFirst,
            Self::UntrackedFirst => crate::scm_changes_panel::ScmGroupOrder::UntrackedFirst,
        }
    }

    #[must_use]
    pub const fn from_panel(order: crate::scm_changes_panel::ScmGroupOrder) -> Self {
        match order {
            crate::scm_changes_panel::ScmGroupOrder::ChangesFirst => Self::ChangesFirst,
            crate::scm_changes_panel::ScmGroupOrder::StagedFirst => Self::StagedFirst,
            crate::scm_changes_panel::ScmGroupOrder::UntrackedFirst => Self::UntrackedFirst,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScmViewModeSetting {
    #[default]
    Tree,
    List,
}

impl ScmViewModeSetting {
    #[must_use]
    pub const fn into_panel(self) -> crate::scm_changes_panel::ScmViewMode {
        match self {
            Self::Tree => crate::scm_changes_panel::ScmViewMode::Tree,
            Self::List => crate::scm_changes_panel::ScmViewMode::List,
        }
    }

    #[must_use]
    pub const fn from_panel(mode: crate::scm_changes_panel::ScmViewMode) -> Self {
        match mode {
            crate::scm_changes_panel::ScmViewMode::Tree => Self::Tree,
            crate::scm_changes_panel::ScmViewMode::List => Self::List,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct GitPreferences {
    pub branch_prefix_mode: BranchPrefixMode,
    #[serde(default = "default_branch_prefix_custom")]
    pub branch_prefix_custom: String,
    pub scm_group_order: ScmOrderSetting,
    pub scm_view_mode: ScmViewModeSetting,
}

impl Default for GitPreferences {
    fn default() -> Self {
        Self {
            branch_prefix_mode: BranchPrefixMode::Custom,
            branch_prefix_custom: default_branch_prefix_custom(),
            scm_group_order: ScmOrderSetting::ChangesFirst,
            scm_view_mode: ScmViewModeSetting::Tree,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuickCommand {
    pub id: u64,
    pub name: String,
    pub text: String,
    #[serde(default = "default_true")]
    pub press_enter: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsagePercentDisplay {
    #[default]
    Used,
    Remaining,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct StatsPreferences {
    pub usage_percentage_display: UsagePercentDisplay,
    #[serde(default = "default_usage_refresh_minutes")]
    pub usage_refresh_minutes: u16,
}

impl Default for StatsPreferences {
    fn default() -> Self {
        Self {
            usage_percentage_display: UsagePercentDisplay::Used,
            usage_refresh_minutes: 5,
        }
    }
}

impl Default for Panels {
    fn default() -> Self {
        Self {
            left_open: true,
            right_open: true,
            right_tab: SideTab::Files,
        }
    }
}

impl Panels {
    pub fn toggle_right(&mut self, tab: SideTab) {
        self.right_open = !self.right_open || self.right_tab != tab;
        self.right_tab = tab;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub id: u64,
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Host {
    pub alias: String,
    pub folder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistent_sessions: Option<bool>,
    /// `tessel-host` daemon for fast Files/Changes: on by default, off only when the user says so.
    #[serde(default = "default_host_daemon", skip_serializing_if = "is_true")]
    pub host_daemon: bool,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            alias: String::new(),
            folder: String::new(),
            persistent_sessions: None,
            host_daemon: true,
        }
    }
}

const fn default_host_daemon() -> bool {
    true
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_true(value: &bool) -> bool {
    *value
}

impl Host {
    /// Persistent `tmux` sessions are enabled by default unless explicitly set to `Some(false)`.
    #[must_use]
    pub const fn persistent_sessions_enabled(&self) -> bool {
        !matches!(self.persistent_sessions, Some(false))
    }
}

const fn default_agent_folder_trust() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Task {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preparation: Option<crate::task_preparation::Details>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<crate::task_report::Report>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<crate::task_decision::Decision>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub last_waiting: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<u64>,
    pub id: u64,
    pub project: u64,
    pub title: String,
    pub column: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_by: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "crate::task_timeline::Timeline::is_empty"
    )]
    pub timeline: crate::task_timeline::Timeline,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NativeThread {
    pub project: u64,
    pub provider: String,
    pub thread: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SavedWorktree {
    pub path: PathBuf,
    pub branch: String,
    /// Descriptive origin only; old/discovered copies may have no recorded base.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectWorktreePrefs {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shown_worktrees: Vec<String>,
    #[serde(default)]
    pub inbox_dismissed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedSession {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_tmux_id: Option<String>,
    /// An explicit native marker suppresses the legacy SSH launch path. It is
    /// inert on restore/import; terminal authority is never stored here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_host: Option<SavedHostTerminal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_model: Option<crate::pane_session_model::SavedPaneModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_seed: Option<crate::codex_terminal::Seed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_binding: Option<crate::session_profile_identity::Binding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_options: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_options: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<SavedWorktree>,
    #[serde(default)]
    pub pane_name: Option<String>,
    #[serde(default = "default_pane_broadcast")]
    pub broadcast: bool,
    pub id: u64,
    pub project: Option<u64>,
    pub title: String,
    pub launch: crate::launch::Launch,
    pub cwd: PathBuf,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SavedHostTerminal {
    pub identity: Option<SavedHostTerminalIdentity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedHostTerminalIdentity {
    pub epoch: tessel_host_protocol::Epoch,
    pub terminal: tessel_host_protocol::TerminalId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Workspace {
    pub schema: u32,
    pub projects: Vec<Project>,
    pub hosts: Vec<Host>,
    pub agent_model_defaults:
        std::collections::BTreeMap<String, crate::agent_model_defaults::Defaults>,
    pub agent_prefs: std::collections::BTreeMap<String, crate::agent_preferences::Preferences>,
    #[serde(default = "default_agent_folder_trust")]
    pub agent_folder_trust: bool,
    pub tasks: Vec<Task>,
    #[serde(skip_serializing_if = "crate::task_history::is_empty")]
    pub task_history: crate::side_panels_model::TaskHistoryStore,
    #[serde(skip_serializing_if = "crate::task_history::not_migrated")]
    pub task_history_migrated: bool,
    pub sessions: Vec<SavedSession>,
    pub next_pane: u64,
    pub native_threads: std::collections::HashMap<u64, NativeThread>,
    pub chat_threads: std::collections::HashMap<u64, String>,
    pub claude_threads: std::collections::HashMap<u64, String>,
    pub active_project: Option<u64>,
    pub dark: bool,
    pub theme: Option<crate::theme_palettes::ThemeId>,
    #[serde(default)]
    pub file_icons: FileIconStyle,
    #[serde(default)]
    pub renderer: RendererPreference,
    pub language: crate::i18n::Language,
    pub columns: usize,
    pub broadcast: bool,
    pub panels: Panels,
    pub sidebar: SidebarPreferences,
    pub launcher_prefs: LauncherPreferences,
    pub general_prefs: GeneralPreferences,
    pub terminal_prefs: TerminalPreferences,
    pub editor_prefs: EditorPreferences,
    pub browser_prefs: BrowserPreferences,
    pub git_prefs: GitPreferences,
    pub quick_commands: Vec<QuickCommand>,
    pub stats_prefs: StatsPreferences,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_agents: Vec<crate::settings_extra_model::CustomAgent>,
    #[serde(default)]
    pub orchestration_prefs: crate::settings_extra_model::OrchestrationPreferences,
    #[serde(default)]
    pub notification_prefs: crate::settings_extra_model::NotificationPreferences,
    #[serde(default)]
    pub voice_prefs: crate::settings_extra_model::VoicePreferences,
    #[serde(default)]
    pub provider_accounts_prefs: crate::settings_extra_model::ProviderAccountsPreferences,
    #[serde(default)]
    pub context_prefs: crate::settings_extra_model::ContextPreferences,
    pub status_bar: StatusBarPreferences,
    pub floating: FloatingPreferences,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub project_worktrees: std::collections::BTreeMap<u64, ProjectWorktreePrefs>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub project_groups: std::collections::BTreeMap<u64, String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub collapsed_project_groups: std::collections::BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<crate::teams_relay::SavedTeam>,
    pub terminal_layouts: std::collections::HashMap<String, crate::terminal_layout::TerminalLayout>,
    pub font_size: f32,
    pub next_id: u64,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            schema: 1,
            projects: vec![],
            hosts: vec![],
            agent_prefs: Default::default(),
            agent_folder_trust: true,
            agent_model_defaults: Default::default(),
            tasks: vec![],
            task_history: Default::default(),
            task_history_migrated: false,
            sessions: vec![],
            next_pane: 1,
            native_threads: Default::default(),
            chat_threads: std::collections::HashMap::new(),
            claude_threads: Default::default(),
            active_project: None,
            dark: true,
            theme: None,
            file_icons: FileIconStyle::default(),
            renderer: Default::default(),
            language: Default::default(),
            columns: 1,
            broadcast: false,
            panels: Default::default(),
            sidebar: Default::default(),
            launcher_prefs: Default::default(),
            general_prefs: Default::default(),
            terminal_prefs: Default::default(),
            editor_prefs: Default::default(),
            browser_prefs: Default::default(),
            git_prefs: Default::default(),
            quick_commands: Vec::new(),
            stats_prefs: Default::default(),
            custom_agents: Vec::new(),
            orchestration_prefs: Default::default(),
            notification_prefs: Default::default(),
            voice_prefs: Default::default(),
            provider_accounts_prefs: Default::default(),
            context_prefs: Default::default(),
            status_bar: Default::default(),
            floating: Default::default(),
            project_worktrees: Default::default(),
            project_groups: Default::default(),
            collapsed_project_groups: Default::default(),
            teams: Vec::new(),
            terminal_layouts: Default::default(),
            font_size: 15.0,
            next_id: 1,
        }
    }
}

impl Workspace {
    pub fn normalize_pane_numbers(&mut self) {
        let numbers = crate::pane_numbers::numbers(
            self.sessions
                .iter()
                .map(|session| (session.id, session.project, session.legacy_number)),
        );
        for session in &mut self.sessions {
            session.legacy_number = numbers.get(&session.id).copied();
        }
    }
    pub fn sync_terminal_layouts(&mut self, room: [f64; 2]) {
        let mut groups: std::collections::HashMap<String, Vec<(u64, String)>> = Default::default();
        for session in &self.sessions {
            groups
                .entry(crate::terminal_layout::project_key(session.project))
                .or_default()
                .push((
                    session.id,
                    session
                        .worktree
                        .as_ref()
                        .map(|wt| crate::pane_grid::norm_path(&wt.path.to_string_lossy()))
                        .unwrap_or_default(),
                ));
        }
        self.terminal_layouts
            .retain(|key, _| groups.contains_key(key));
        for (key, ids) in groups {
            self.terminal_layouts
                .entry(key)
                .or_default()
                .sync_worktrees(&ids, room, self.terminal_prefs.tile_new_panes);
        }
    }
    pub fn validate(&self) -> Result<()> {
        if !self.floating.height.is_finite()
            || !(crate::palette::FLOATING_MIN_HEIGHT..=crate::palette::FLOATING_MAX_HEIGHT)
                .contains(&self.floating.height)
        {
            return Err("Invalid floating terminal height.".into());
        }
        if self.schema != 1 {
            return Err(
                "This workspace was written by an unsupported version; it will not be overwritten."
                    .into(),
            );
        }
        if self.projects.len() > 1000 || self.hosts.len() > 1000 || self.tasks.len() > 10000 {
            return Err("Workspace exceeds supported limits.".into());
        }
        if !(6..=100).contains(&self.editor_prefs.font_size) {
            return Err("Invalid editor font size.".into());
        }
        if !(1..=8).contains(&self.editor_prefs.tab_size) {
            return Err("Invalid editor tab size.".into());
        }
        if !(250..=10_000).contains(&self.editor_prefs.auto_save_delay_ms) {
            return Err("Invalid editor auto-save delay.".into());
        }
        if self.general_prefs.workspace_dir.len() > 4096
            || self
                .general_prefs
                .workspace_dir
                .chars()
                .any(char::is_control)
        {
            return Err("Invalid worktrees directory override.".into());
        }
        if !(500..=100_000).contains(&self.terminal_prefs.scrollback)
            || !self.terminal_prefs.line_height.is_finite()
            || !(1.0..=3.0).contains(&self.terminal_prefs.line_height)
        {
            return Err("Invalid terminal preferences.".into());
        }
        if self.git_prefs.branch_prefix_custom.len() > 80
            || self
                .git_prefs
                .branch_prefix_custom
                .chars()
                .any(char::is_control)
        {
            return Err("Invalid custom Git branch prefix.".into());
        }
        if !matches!(
            self.stats_prefs.usage_refresh_minutes,
            0 | 2 | 5 | 15 | 30 | 60
        ) {
            return Err("Invalid usage refresh interval.".into());
        }
        crate::settings_extra_model::validate_custom_agents(&self.custom_agents)?;
        self.orchestration_prefs.validate()?;
        self.notification_prefs.validate()?;
        self.voice_prefs.validate()?;
        self.provider_accounts_prefs.validate()?;
        if self.quick_commands.len() > 200 {
            return Err("Too many quick commands.".into());
        }
        let mut qc_ids = std::collections::HashSet::with_capacity(self.quick_commands.len());
        for cmd in &self.quick_commands {
            let trimmed_name = cmd.name.trim();
            if !qc_ids.insert(cmd.id)
                || cmd.id == u64::MAX
                || trimmed_name.is_empty()
                || cmd.name.chars().count() > 120
                || cmd.name.chars().any(char::is_control)
                || cmd.text.trim().is_empty()
                || cmd.text.chars().count() > 20_000
                || cmd.text.contains('\0')
            {
                return Err("Invalid quick command.".into());
            }
        }
        if let Some(shell) = &self.launcher_prefs.preferred_shell
            && !matches!(shell.as_str(), "powershell" | "cmd" | "wsl" | "sh")
        {
            return Err("Unsupported preferred shell.".into());
        }
        if let Some(rem) = &self.launcher_prefs.remember_default
            && (rem.len() > 64 || rem.chars().any(char::is_control))
        {
            return Err("Invalid remembered launcher entry.".into());
        }
        if self
            .agent_prefs
            .keys()
            .any(|id| crate::agent_catalog::find(id).is_none())
        {
            return Err("Unknown saved agent preference.".into());
        }
        if self.agent_model_defaults.len() > 60
            || self.agent_model_defaults.iter().any(|(id, defaults)| {
                crate::agent_model_catalog::find(id).is_none() || !defaults.validate()
            })
        {
            return Err("Invalid saved agent model defaults.".into());
        }
        if self.sessions.len() > 100 || self.next_id == u64::MAX {
            return Err("Workspace limits exceeded.".into());
        }
        if self.chat_threads.len() > 1000
            || self.chat_threads.values().any(|id| {
                id.is_empty()
                    || id.len() > 100
                    || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
        {
            return Err("Invalid saved chat metadata.".into());
        }
        if self.claude_threads.len() > 1000
            || self
                .claude_threads
                .values()
                .any(|id| !crate::claude::valid_uuid(id))
        {
            return Err("Invalid saved Claude chat metadata".into());
        }
        let mut session_ids = std::collections::HashSet::new();
        if self.next_pane == u64::MAX || self.native_threads.len() > 1000 {
            return Err("Native chat metadata limit exceeded.".into());
        }
        for (pane, thread) in &self.native_threads {
            if *pane == u64::MAX || !valid_native_thread(&thread.provider, &thread.thread) {
                return Err("Invalid saved native chat thread.".into());
            }
            if let Some(session) = self.sessions.iter().find(|s| s.id == *pane)
                && (session.project != Some(thread.project)
                    || session.launch.native_chat() != Some(thread.provider.as_str()))
            {
                return Err("Native chat thread belongs to a different pane.".into());
            }
        }
        for session in &self.sessions {
            if session.native_host.is_some()
                && !matches!(session.launch, crate::launch::Launch::Ssh(_))
            {
                return Err("Native host metadata requires an SSH pane.".into());
            }
            if let Some(model) = &session.last_model {
                let value =
                    serde_json::to_value(model).map_err(|_| "Invalid saved model metadata.")?;
                if !matches!(session.launch, crate::launch::Launch::Agent(_))
                    || crate::pane_session_model::saved_pane_model(Some(&value)).as_ref()
                        != Some(model)
                {
                    return Err("Invalid saved model metadata.".into());
                }
            }
            if let Some(seed) = &session.codex_seed
                && (!seed.valid()
                    || session.conversation_id.is_some()
                    || !matches!(&session.launch, crate::launch::Launch::Agent(agent) if agent == "codex"))
            {
                return Err("Invalid Codex recovery metadata.".into());
            }
            if let Some(binding) = &session.profile_binding
                && (!binding.valid()
                    || !(matches!(&session.launch, crate::launch::Launch::Agent(agent) if crate::claude_terminal::supported(agent) || agent == "codex")
                        || session.launch.native_chat().is_some()))
            {
                return Err("Invalid local session profile identity.".into());
            }
            if let Some(id) = &session.conversation_id
                && (!crate::claude::valid_uuid(id)
                    || !matches!(&session.launch, crate::launch::Launch::Agent(agent) if crate::claude_terminal::supported(agent) || agent == "codex"))
            {
                return Err("Invalid terminal conversation metadata.".into());
            }
            if !session_ids.insert(session.id)
                || session.id == u64::MAX
                || !session.cwd.is_absolute()
                || session.title.len() > 512
            {
                return Err("Invalid session metadata.".into());
            }
            if session.launch_fingerprint.as_deref().is_some_and(|value| {
                !matches!(session.launch, crate::launch::Launch::Agent(_))
                    || !crate::launch_fingerprint::valid(value)
            }) {
                return Err("Invalid saved launch fingerprint.".into());
            }
            if let Some(values) = &session.session_options {
                let agent = match &session.launch {
                    crate::launch::Launch::Agent(agent) => agent,
                    _ => return Err("Only agent terminals may have saved session options.".into()),
                };
                if crate::agent_model_catalog::find(agent).is_none()
                    || !valid_session_options(values)
                {
                    return Err("Invalid saved session model options.".into());
                }
            }
            if let Some(values) = &session.native_options {
                let provider = match session.launch {
                    crate::launch::Launch::CodexChat => "codex",
                    crate::launch::Launch::ClaudeChat => "claude",
                    _ => return Err("Only native chats may have native options.".into()),
                };
                if !serde_json::from_value::<crate::native_chat_options::Options>(values.clone())
                    .is_ok_and(|options| options.valid_for(provider))
                {
                    return Err("Invalid native chat model options.".into());
                }
            }
            if let Some(wt) = &session.worktree {
                let b = wt.branch.trim();
                if !wt.path.is_absolute()
                    || b.is_empty()
                    || wt.branch.len() > 200
                    || wt.branch.chars().any(char::is_control)
                    || wt.base_branch.as_ref().is_some_and(|b| {
                        b.trim().is_empty() || b.len() > 1024 || b.chars().any(char::is_control)
                    })
                {
                    return Err("Invalid session worktree metadata.".into());
                }
            }
            if session.launch.native_chat().is_some() {
                let project = session
                    .project
                    .and_then(|id| self.projects.iter().find(|p| p.id == id));
                let allowed_cwd = project.is_some_and(|p| {
                    p.path == session.cwd
                        || session
                            .worktree
                            .as_ref()
                            .is_some_and(|wt| wt.path == session.cwd)
                });
                if !allowed_cwd {
                    return Err("Invalid native chat pane.".into());
                }
            }
            match &session.launch {
                crate::launch::Launch::Agent(name)
                    if crate::agent_catalog::find(name).is_none() =>
                {
                    return Err("Unknown saved agent.".into());
                }
                crate::launch::Launch::Ssh(host) => validate_host(&host.alias)?,
                _ => {}
            }
        }
        if self.project_worktrees.len() > 1000
            || self.project_worktrees.values().any(|prefs| {
                prefs.shown_worktrees.len() > 200
                    || prefs
                        .shown_worktrees
                        .iter()
                        .any(|p| p.is_empty() || p.len() > 4096 || p.contains('\0'))
            })
        {
            return Err("Invalid project worktree visibility metadata.".into());
        }
        if self.project_groups.len() > 1000
            || self.project_groups.iter().any(|(pid, g)| {
                *pid >= self.next_id
                    || g.trim().is_empty()
                    || g.chars().count() > 120
                    || g.chars().any(char::is_control)
            })
            || self.collapsed_project_groups.len() > 1000
            || self.collapsed_project_groups.iter().any(|g| {
                g.trim().is_empty() || g.chars().count() > 120 || g.chars().any(char::is_control)
            })
        {
            return Err("Invalid project group metadata.".into());
        }
        if self.terminal_layouts.len() > 101 {
            return Err("Too many terminal layouts".into());
        }
        for (key, layout) in &self.terminal_layouts {
            let allowed = self
                .sessions
                .iter()
                .filter(|session| crate::terminal_layout::project_key(session.project) == *key)
                .map(|session| session.id)
                .collect();
            layout.validate(&allowed)?;
        }
        if !(1..=3).contains(&self.columns)
            || !self.font_size.is_finite()
            || !(10.0..=28.0).contains(&self.font_size)
        {
            return Err("Invalid appearance settings.".into());
        }
        let mut ids = std::collections::HashSet::new();
        for project in &self.projects {
            if !ids.insert(project.id)
                || project.id >= self.next_id
                || project.name.len() > 512
                || !project.path.is_absolute()
            {
                return Err("Invalid project metadata.".into());
            }
        }
        crate::task_history::validate(&self.task_history)?;
        crate::task_dependencies::validate(&self.tasks, self.next_id)?;
        for task in &self.tasks {
            if let Some(details) = &task.preparation {
                details.validate()?;
            }
            if task.created_by == Some(0) {
                return Err("Invalid task creator.".into());
            }
            if let Some(report) = &task.report {
                report.validate()?;
            }
            if let Some(decision) = &task.decision {
                decision.validate(self.next_id)?;
                if !ids.insert(decision.request) {
                    return Err("Duplicate decision identity.".into());
                }
            }
            if task.last_waiting.len() > crate::task_dependencies::MAX_DEPS
                || task
                    .last_waiting
                    .iter()
                    .any(|id| *id == 0 || *id >= self.next_id)
            {
                return Err("Invalid task readiness state.".into());
            }
            task.timeline.validate()?;
            if task.column != 1
                && task
                    .timeline
                    .work_periods
                    .iter()
                    .any(|p| p.end_ms.is_none())
            {
                return Err("Only Doing tasks may have an open work period.".into());
            }
            if !ids.insert(task.id)
                || task.id >= self.next_id
                || task.column > 3
                || task.title.len() > 4096
                || task
                    .guest_by
                    .as_ref()
                    .is_some_and(|s| s.chars().count() > 40)
            {
                return Err("Invalid task metadata.".into());
            }
        }
        for host in &self.hosts {
            validate_host(&host.alias)?;
        }
        Ok(())
    }
    pub fn check_storage_budget(&self) -> Result<()> {
        if serde_json::to_vec_pretty(self)
            .map_err(|_| "Cannot encode workspace")?
            .len()
            > 4 * 1024 * 1024
        {
            return Err(
                "Task change exceeds workspace storage capacity; no change was applied.".into(),
            );
        }
        Ok(())
    }
    /// Reserve space for settings; discard oldest history before refusing a card.
    pub fn task_cards_size(&self) -> Result<usize> {
        serde_json::to_vec_pretty(&self.tasks)
            .map(|b| b.len())
            .map_err(|_| "Cannot encode task cards".into())
    }
    #[cfg(test)]
    pub fn prepare_task_change(&mut self) -> Result<Vec<u8>> {
        self.prepare_task_change_from(0)
    }
    pub fn prepare_task_change_from(&mut self, previous_cards: usize) -> Result<Vec<u8>> {
        const BOARD_LIMIT: usize = 5 * 1024 * 1024 / 2;
        let cards_size = self.task_cards_size()?;
        if cards_size > 2 * 1024 * 1024 && cards_size > previous_cards {
            return Err("Task cards exceed their storage capacity; no change was applied.".into());
        }
        let allowed = BOARD_LIMIT.max(cards_size + 512 * 1024);
        loop {
            let size = serde_json::to_vec_pretty(&(&self.tasks, &self.task_history))
                .map_err(|_| "Cannot encode task board")?
                .len();
            if size <= allowed {
                break;
            }
            if self.task_history.records.is_empty() {
                return Err(
                    "Task board exceeds its storage capacity; no change was applied.".into(),
                );
            }
            let history_size = serde_json::to_vec_pretty(&self.task_history)
                .map_err(|_| "Cannot encode task history")?
                .len();
            let average = (history_size / self.task_history.records.len()).max(1);
            let remove = ((size - allowed) / average + 1).min(self.task_history.records.len());
            self.task_history.records.drain(..remove);
        }
        self.validate()?;
        loop {
            let bytes = serde_json::to_vec_pretty(self).map_err(|_| "Cannot encode workspace")?;
            if bytes.len() <= 4 * 1024 * 1024 {
                return Ok(bytes);
            }
            if self.task_history.records.is_empty() {
                return Err("Workspace storage capacity exceeded; no change was applied.".into());
            }
            let history_size = serde_json::to_vec_pretty(&self.task_history)
                .map_err(|_| "Cannot encode history")?
                .len();
            let average = (history_size / self.task_history.records.len()).max(1);
            let remove = ((bytes.len() - 4 * 1024 * 1024) / average + 1)
                .min(self.task_history.records.len());
            self.task_history.records.drain(..remove);
        }
    }
    pub fn next_pane_id(&self) -> u64 {
        self.sessions
            .iter()
            .map(|s| s.id)
            .chain(self.native_threads.keys().copied())
            .fold(self.next_pane.max(1), |next, id| {
                next.max(id.saturating_add(1))
            })
    }

    pub fn migrate_native_threads(&mut self) {
        let mut names: Vec<_> = self
            .sessions
            .iter()
            .map(|session| crate::agent_names::NamedPane {
                id: session.id,
                number: session.id,
                named: matches!(
                    session.launch,
                    crate::launch::Launch::Agent(_)
                        | crate::launch::Launch::CodexChat
                        | crate::launch::Launch::ClaudeChat
                ),
                name: session.pane_name.clone(),
            })
            .collect();
        crate::agent_names::ensure_agent_names(&mut names);
        for (session, name) in self.sessions.iter_mut().zip(names) {
            session.pane_name = name.name;
        }

        for session in &self.sessions {
            let Some(project) = session.project else {
                continue;
            };
            let Some(provider) = session.launch.native_chat() else {
                continue;
            };
            let legacy = if provider == "Codex" {
                self.chat_threads.remove(&project)
            } else {
                self.claude_threads.remove(&project)
            };
            if let Some(thread) = legacy {
                self.native_threads
                    .entry(session.id)
                    .or_insert_with(|| NativeThread {
                        project,
                        provider: provider.into(),
                        thread,
                    });
            }
        }
        self.next_pane = self.next_pane_id();
    }

    pub fn native_resume(&self, pane: u64, project: u64, provider: &str) -> Option<&str> {
        self.native_threads
            .get(&pane)
            .filter(|saved| saved.project == project && saved.provider == provider)
            .map(|saved| saved.thread.as_str())
    }

    pub fn remember_native_thread(
        &mut self,
        pane: u64,
        project: u64,
        provider: &str,
        thread: Option<&str>,
    ) -> Result<bool> {
        if !self.sessions.iter().any(|s| {
            s.id == pane && s.project == Some(project) && s.launch.native_chat() == Some(provider)
        }) {
            return Err("Native chat pane no longer exists in this project.".into());
        }
        let Some(thread) = thread else {
            return Ok(self.native_threads.remove(&pane).is_some());
        };
        if !valid_native_thread(provider, thread) {
            return Err("Invalid native thread ID.".into());
        }
        if self.native_resume(pane, project, provider) == Some(thread) {
            return Ok(false);
        }
        if !self.native_threads.contains_key(&pane) && self.native_threads.len() >= 1000 {
            return Err("Native thread history is full.".into());
        }
        self.native_threads.insert(
            pane,
            NativeThread {
                project,
                provider: provider.into(),
                thread: thread.into(),
            },
        );
        Ok(true)
    }

    /// Converts a saved Claude/Codex session in-place between `Launch::Agent` (Terminal)
    /// and `Launch::ClaudeChat` / `Launch::CodexChat` (Native Chat) on the same pane `id`,
    /// preserving `pane_name`, `project`, `cwd`, `worktree`, model options, and conversation ID.
    pub fn switch_saved_session_chat_mode(
        &mut self,
        pane_id: u64,
        live_chat_thread: Option<&str>,
    ) -> Result<bool> {
        let Some(idx) = self.sessions.iter().position(|s| s.id == pane_id) else {
            return Ok(false);
        };
        let Some(project_id) = self.sessions[idx].project else {
            return Err("Native chat requires an active workspace project.".into());
        };
        let allowed_cwd = self
            .projects
            .iter()
            .find(|p| p.id == project_id)
            .is_some_and(|p| {
                p.path == self.sessions[idx].cwd
                    || self.sessions[idx]
                        .worktree
                        .as_ref()
                        .is_some_and(|wt| wt.path == self.sessions[idx].cwd)
            });
        if !allowed_cwd {
            return Err("Session working directory is outside the project.".into());
        }
        if self.sessions[idx].native_host.is_some() {
            return Err("SSH host terminals cannot switch to local native chat.".into());
        }

        let current_launch = self.sessions[idx].launch.clone();
        match current_launch {
            crate::launch::Launch::Agent(agent) if agent == "claude" || agent == "codex" => {
                let (provider, next_launch, next_title) = if agent == "claude" {
                    (
                        "Claude",
                        crate::launch::Launch::ClaudeChat,
                        "Claude Chat".to_owned(),
                    )
                } else {
                    (
                        "Codex",
                        crate::launch::Launch::CodexChat,
                        "Codex Chat".to_owned(),
                    )
                };
                let conv = self.sessions[idx].conversation_id.take();
                let prev_opts = self.sessions[idx].session_options.take();
                let mut native_opts =
                    crate::native_chat_options::Options::from_value(prev_opts.as_ref());
                native_opts.normalize();
                self.sessions[idx].last_model = None;
                self.sessions[idx].codex_seed = None;
                self.sessions[idx].launch_fingerprint = None;
                self.sessions[idx].native_options = native_opts
                    .valid_for(&agent)
                    .then(|| native_opts.value())
                    .flatten();
                self.sessions[idx].launch = next_launch;
                self.sessions[idx].title = next_title;
                if let Some(tid) = conv.as_deref().or(live_chat_thread)
                    && valid_native_thread(provider, tid)
                {
                    self.native_threads.insert(
                        pane_id,
                        NativeThread {
                            project: project_id,
                            provider: provider.to_owned(),
                            thread: tid.to_owned(),
                        },
                    );
                }
                self.validate()?;
                Ok(true)
            }
            crate::launch::Launch::ClaudeChat | crate::launch::Launch::CodexChat => {
                let (agent, next_title) =
                    if matches!(current_launch, crate::launch::Launch::ClaudeChat) {
                        ("claude", "Claude Code".to_owned())
                    } else {
                        ("codex", "Codex".to_owned())
                    };
                let removed = self.native_threads.remove(&pane_id);
                let chosen_thread = live_chat_thread
                    .map(str::to_owned)
                    .or_else(|| removed.map(|nt| nt.thread));
                let prev_native = self.sessions[idx].native_options.take();
                self.sessions[idx].session_options = prev_native.filter(valid_session_options);
                self.sessions[idx].conversation_id =
                    chosen_thread.filter(|id| crate::claude::valid_uuid(id));
                self.sessions[idx].codex_seed = None;
                self.sessions[idx].last_model = None;
                self.sessions[idx].launch_fingerprint = None;
                self.sessions[idx].launch = crate::launch::Launch::Agent(agent.to_owned());
                self.sessions[idx].title = next_title;
                self.validate()?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub fn id(&mut self) -> Result<u64> {
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .filter(|n| *n < u64::MAX)
            .ok_or("Workspace identifiers exhausted")?;
        Ok(id)
    }
    pub fn add_project(&mut self, path: &Path) -> Result<u64> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        if !path.is_dir() {
            return Err("Choose a directory.".into());
        }
        if let Some(project) = self.projects.iter().find(|p| p.path == path) {
            self.active_project = Some(project.id);
            return Ok(project.id);
        }
        let id = self.id()?;
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .to_string();
        self.projects.push(Project { id, name, path });
        self.active_project = Some(id);
        Ok(id)
    }
    pub fn active(&self) -> Option<&Project> {
        self.projects
            .iter()
            .find(|p| Some(p.id) == self.active_project)
    }
}

/// Persisted pane choices are complete, bounded model/option objects.
/// Interactive source-compatible normalization happens before saving them.
pub fn valid_session_options(value: &serde_json::Value) -> bool {
    // The source always keeps model, plus up to ten entries encountered
    // before model in the incoming object.
    let Some(values) = value.as_object().filter(|v| v.len() <= 11) else {
        return false;
    };
    let Some(model) = values.get("model").and_then(serde_json::Value::as_str) else {
        return false;
    };
    let options = values
        .iter()
        .filter(|(key, _)| key.as_str() != "model")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    crate::agent_model_defaults::Defaults {
        model: Some(model.into()),
        values_by_model: std::collections::BTreeMap::from([(model.into(), options)]),
    }
    .validate()
}

pub fn valid_native_thread(provider: &str, thread: &str) -> bool {
    match provider {
        "Codex" => {
            !thread.is_empty()
                && thread.len() <= 100
                && thread
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
        }
        "Claude" => crate::claude::valid_uuid(thread),
        _ => false,
    }
}

pub fn validate_host(alias: &str) -> Result<()> {
    if alias.is_empty()
        || alias.starts_with('-')
        || alias.len() > 253
        || !alias
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-@:".contains(c))
    {
        return Err(
            "Use an SSH config alias or user@host (no options, spaces or commands).".into(),
        );
    }
    Ok(())
}

pub struct Store {
    pub warning: String,
    path: PathBuf,
    _lock: fs::File,
}

impl Store {
    pub fn directory(&self) -> &Path {
        self.path.parent().expect("workspace file has a directory")
    }
    pub fn open(dir: &Path) -> Result<(Self, Workspace)> {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("workspace.lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock_exclusive()
            .map_err(|_| "Another Tessel Rust instance is using this workspace.".to_string())?;
        let path = dir.join("workspace.json");
        let mut state = match fs::File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(4 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() > 4 * 1024 * 1024 {
                    return Err("Workspace is too large; original retained.".into());
                }
                let mut state: Workspace = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("Cannot read workspace; original retained: {e}"))?;
                state.validate()?;
                state.migrate_native_threads();
                state.validate()?;
                state
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Workspace::default(),
            Err(e) => return Err(e.to_string()),
        };
        let mut warning = String::new();
        state.normalize_pane_numbers();
        if !state.task_history_migrated {
            match crate::task_history::load_legacy(dir) {
                Ok(legacy) => {
                    let mut merged = state.task_history.clone();
                    match crate::task_history::merge_legacy(
                        &mut merged,
                        legacy,
                        &state.tasks,
                        state.next_id,
                    ) {
                        Ok(()) => {
                            state.task_history = merged;
                            state.task_history_migrated = true;
                        }
                        Err(error) => {
                            warning = error;
                        }
                    }
                }
                Err(error) => {
                    warning = error;
                }
            }
        }
        crate::task_history::raise_identity_floors(&mut state)?;
        state.validate()?;
        Ok((
            Self {
                path,
                _lock: lock,
                warning,
            },
            state,
        ))
    }
    pub fn restore(&self, before: &Workspace, after: &Workspace) -> Result<PathBuf> {
        after.validate()?;
        let backup = tempfile::Builder::new()
            .prefix("workspace-before-restore-")
            .suffix(".json")
            .tempfile_in(self.path.parent().ok_or("No settings directory")?)
            .map_err(|e| e.to_string())?;
        let (_, path) = backup.keep().map_err(|e| e.to_string())?;
        atomic_write(
            &path,
            &serde_json::to_vec_pretty(before).map_err(|e| e.to_string())?,
        )?;
        self.save(after)?;
        Ok(path)
    }
    pub fn save(&self, state: &Workspace) -> Result<()> {
        state.validate()?;
        let bytes = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("Workspace exceeds 4 MiB; previous settings retained".into());
        }
        atomic_write(&self.path, &bytes)
    }
    pub(crate) fn save_task_bytes(&self, bytes: &[u8]) -> Result<()> {
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("Workspace storage capacity exceeded.".into());
        }
        atomic_write(&self.path, bytes)
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    prepare_atomic_write(path, bytes)?
        .persist(path)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Stage the bytes beside the destination so callers can recheck it before the rename.
pub(crate) fn prepare_atomic_write(path: &Path, bytes: &[u8]) -> Result<tempfile::NamedTempFile> {
    let parent = path.parent().ok_or("No parent directory")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    if let Ok(metadata) = fs::metadata(path) {
        temp.as_file()
            .set_permissions(metadata.permissions())
            .map_err(|e| e.to_string())?;
    }
    temp.write_all(bytes).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    Ok(temp)
}

#[cfg(test)]
mod tests {
    #[test]
    fn saved_worktree_origin_is_optional_and_roundtrips() {
        let old = serde_json::json!({"path":"C:/synthetic/copy", "branch":"agent/task"});
        let mut copy: super::SavedWorktree = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(copy.base_branch, None);
        assert_eq!(serde_json::to_value(&copy).unwrap(), old);
        copy.base_branch = Some("feature/source".into());
        let restored: super::SavedWorktree =
            serde_json::from_value(serde_json::to_value(&copy).unwrap()).unwrap();
        assert_eq!(copy, restored);
    }
    use super::*;
    #[test]
    fn pane_numbers_survive_save_reload_reordering_and_new_panes() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let project = state.add_project(dir.path()).unwrap();
        let cwd = state
            .projects
            .iter()
            .find(|p| p.id == project)
            .unwrap()
            .path
            .clone();
        for (id, number) in [(900, 4), (7, 1)] {
            state.sessions.push(
                serde_json::from_value(serde_json::json!({
                    "id":id,"project":project,"title":"Synthetic","launch":"CodexChat",
                    "cwd":cwd,"legacy_number":number
                }))
                .unwrap(),
            );
        }
        state.next_pane = 1000;
        state.normalize_pane_numbers();
        store.save(&state).unwrap();
        drop(store);
        let (_store, mut loaded) = Store::open(dir.path()).unwrap();
        loaded.sessions.reverse();
        loaded.normalize_pane_numbers();
        assert_eq!(
            loaded
                .sessions
                .iter()
                .find(|s| s.id == 900)
                .unwrap()
                .legacy_number,
            Some(4)
        );
        loaded.sessions.retain(|s| s.id != 7);
        loaded.sessions.push(
            serde_json::from_value(serde_json::json!({
                "id":800,"project":project,"title":"New","launch":"CodexChat","cwd":cwd
            }))
            .unwrap(),
        );
        loaded.normalize_pane_numbers();
        assert_eq!(
            loaded
                .sessions
                .iter()
                .find(|s| s.id == 800)
                .unwrap()
                .legacy_number,
            Some(1)
        );
        assert_eq!(
            loaded
                .sessions
                .iter()
                .find(|s| s.id == 900)
                .unwrap()
                .legacy_number,
            Some(4)
        );
    }
    #[test]
    fn terminal_conversations_round_trip_and_reject_invalid_or_wrong_provider_ids() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let id = crate::claude::uuid().unwrap();
        state.sessions.push(SavedSession {
            legacy_number: None,
            ssh_tmux_id: None,
            native_host: None,
            last_model: None,
            codex_seed: None,
            conversation_id: Some(id.clone()),
            profile_binding: None,
            launch_fingerprint: None,
            session_options: None,
            native_options: None,
            worktree: None,
            pane_name: None,
            broadcast: true,
            id: 1,
            project: None,
            title: "Synthetic".into(),
            launch: crate::launch::Launch::Agent("claude".into()),
            cwd: dir.path().to_owned(),
        });
        store.save(&state).unwrap();
        drop(store);
        let (store, mut loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(
            loaded.sessions[0].conversation_id.as_deref(),
            Some(id.as_str())
        );
        loaded.sessions[0].conversation_id = Some("../../untrusted".into());
        assert!(store.save(&loaded).is_err());
        loaded.sessions[0].conversation_id = Some(id);
        loaded.sessions[0].launch = crate::launch::Launch::Shell;
        assert!(store.save(&loaded).is_err());
        loaded.sessions[0].launch = crate::launch::Launch::Agent("codex".into());
        store.save(&loaded).unwrap();
        drop(store);
        let (_, codex_loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(
            codex_loaded.sessions[0].conversation_id,
            loaded.sessions[0].conversation_id
        );
        let mut recovery = codex_loaded;
        let seed = crate::codex_terminal::Discovery::new(
            &crate::agent_preferences::Preferences {
                env: "CODEX_HOME=synthetic".into(),
                ..Default::default()
            },
            dir.path(),
        )
        .unwrap()
        .seed();
        recovery.sessions[0].codex_seed = Some(seed.clone());
        assert!(recovery.validate().is_err()); // ID and pending discovery are exclusive.
        recovery.sessions[0].conversation_id = None;
        assert!(recovery.validate().is_ok());
        let restored: Workspace =
            serde_json::from_slice(&serde_json::to_vec(&recovery).unwrap()).unwrap();
        assert_eq!(restored.sessions[0].codex_seed, Some(seed));
        recovery.sessions[0].launch = crate::launch::Launch::Shell;
        assert!(recovery.validate().is_err());
        let mut old = serde_json::to_value(&state.sessions[0]).unwrap();
        old.as_object_mut().unwrap().remove("conversation_id");
        assert!(
            serde_json::from_value::<SavedSession>(old)
                .unwrap()
                .conversation_id
                .is_none()
        );
    }
    #[test]
    fn pane_model_choices_roundtrip_independently_and_invalid_choices_never_replace_state() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        for (id, options) in [
            (1, json!({"model":"pane-one"})),
            (
                2,
                json!({"model":"pane-two","effort":"low","fastMode":false}),
            ),
        ] {
            state.sessions.push(SavedSession {
                legacy_number: None,
                ssh_tmux_id: None,
                native_host: None,
                last_model: None,
                codex_seed: None,
                conversation_id: None,
                profile_binding: None,
                launch_fingerprint: None,
                session_options: Some(options),
                native_options: None,
                worktree: None,
                pane_name: None,
                broadcast: true,
                id,
                project: None,
                title: "Codex".into(),
                launch: crate::launch::Launch::Agent("codex".into()),
                cwd: dir.path().to_owned(),
            });
        }
        store.save(&state).unwrap();
        let snapshot = std::fs::read(dir.path().join("workspace.json")).unwrap();
        for invalid in [
            json!({"effort":"low"}),
            json!({"model":"bad;command"}),
            json!({"model":"ok","effort":7}),
            json!({"model":"ok","invalid-id":true}),
        ] {
            let mut next = state.clone();
            next.sessions[0].session_options = Some(invalid);
            assert!(store.save(&next).is_err());
            assert_eq!(
                std::fs::read(dir.path().join("workspace.json")).unwrap(),
                snapshot
            );
        }
        let mut shell = state.clone();
        shell.sessions[0].launch = crate::launch::Launch::Shell;
        assert!(shell.validate().is_err());
        let mut many = json!({"model":"bounded"});
        for index in 0..10 {
            many[format!("option{index}")] = true.into();
        }
        let mut bounded = state.clone();
        bounded.sessions[0].session_options = Some(many.clone());
        assert!(bounded.validate().is_ok());
        many["overflow"] = false.into();
        bounded.sessions[0].session_options = Some(many);
        assert!(bounded.validate().is_err());
        let mut old = serde_json::to_value(&state.sessions[0]).unwrap();
        old.as_object_mut().unwrap().remove("session_options");
        assert!(
            serde_json::from_value::<SavedSession>(old)
                .unwrap()
                .session_options
                .is_none()
        );
        drop(store);
        let (_, restored) = Store::open(dir.path()).unwrap();
        assert_eq!(
            restored.sessions[0].session_options,
            state.sessions[0].session_options
        );
        assert_eq!(
            restored.sessions[1].session_options,
            state.sessions[1].session_options
        );
    }
    #[test]
    fn projects_roundtrip_and_select_automatically() {
        let dir = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let id = state.add_project(project.path()).unwrap();
        assert_eq!(state.active_project, Some(id));
        assert_eq!(state.add_project(project.path()).unwrap(), id);
        assert_eq!(state.projects.len(), 1);
        store.save(&state).unwrap();
        assert!(Store::open(dir.path()).is_err());
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.projects, state.projects);
    }
    #[test]
    fn language_survives_restart_and_old_workspaces_keep_their_default() {
        let old: Workspace = serde_json::from_str("{}").unwrap();
        assert_eq!(old.language, crate::i18n::Language::English);
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.language = crate::i18n::Language::French;
        store.save(&state).unwrap();
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.language, crate::i18n::Language::French);
        assert!(loaded.sessions.is_empty());
    }
    #[test]
    fn named_themes_survive_restart_and_preserve_legacy_light_preference() {
        let old: Workspace = serde_json::from_str(r#"{"dark":false}"#).unwrap();
        assert!(!old.dark);
        assert!(old.theme.is_none());
        let dir = tempfile::tempdir().unwrap();
        for theme in crate::theme_palettes::THEMES {
            let (store, mut state) = Store::open(dir.path()).unwrap();
            state.theme = Some(theme);
            store.save(&state).unwrap();
            drop(store);
            let (store, loaded) = Store::open(dir.path()).unwrap();
            assert_eq!(loaded.theme, Some(theme));
            assert!(loaded.sessions.is_empty());
            drop(store);
        }
        assert!(serde_json::from_str::<Workspace>(r#"{"theme":"unrecognized-theme"}"#).is_err());
    }
    #[test]
    fn corrupt_state_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("workspace.json"), b"broken").unwrap();
        assert!(Store::open(dir.path()).is_err());
        assert_eq!(
            fs::read(dir.path().join("workspace.json")).unwrap(),
            b"broken"
        );
    }
    #[test]
    fn floating_preferences_survive_restart_without_joining_the_grid() {
        let old: Workspace = serde_json::from_str("{}").unwrap();
        assert_eq!(old.floating, FloatingPreferences::default());
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.floating = FloatingPreferences {
            open: true,
            height: 0.72,
        };
        store.save(&state).unwrap();
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.floating, state.floating);
        assert!(loaded.sessions.is_empty());
        for height in [f64::NAN, f64::INFINITY, 0.14, 0.91] {
            state.floating.height = height;
            assert!(state.validate().is_err());
        }
    }
    #[test]
    fn panel_toggles_survive_restart_and_old_settings_keep_visible_defaults() {
        let old: Workspace = serde_json::from_str("{}").unwrap();
        assert_eq!(old.panels, Panels::default());
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.panels.left_open = false;
        state.panels.toggle_right(SideTab::Files);
        assert!(!state.panels.right_open);
        state.panels.toggle_right(SideTab::Tasks);
        assert!(state.panels.right_open);
        assert_eq!(state.panels.right_tab, SideTab::Tasks);
        state.panels.toggle_right(SideTab::Changes);
        assert!(state.panels.right_open);
        store.save(&state).unwrap();
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.panels, state.panels);
        assert!(loaded.sessions.is_empty());
        assert!(
            serde_json::from_str::<Workspace>(r#"{"panels":{"right_tab":"unknown"}}"#).is_err()
        );
    }
    #[test]
    fn native_chat_metadata_is_project_scoped_unique_and_persistent() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let project = state.add_project(dir.path()).unwrap();
        let cwd = state.active().unwrap().path.clone();
        for (id, launch) in [
            (1, crate::launch::Launch::CodexChat),
            (2, crate::launch::Launch::ClaudeChat),
        ] {
            state.sessions.push(SavedSession {
                legacy_number: None,
                ssh_tmux_id: None,
                native_host: None,
                last_model: None,
                codex_seed: None,
                conversation_id: None,
                profile_binding: None,
                launch_fingerprint: None,
                session_options: None,
                native_options: Some(serde_json::json!({"effort":"high"})),
                worktree: None,
                pane_name: None,
                broadcast: true,
                id,
                project: Some(project),
                title: launch.native_chat().unwrap().into(),
                launch,
                cwd: cwd.clone(),
            });
        }
        state.sync_terminal_layouts([1000.0, 700.0]);
        store.save(&state).unwrap();
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.sessions.len(), 2);
        assert_eq!(
            loaded.sessions[0].native_options,
            Some(serde_json::json!({"effort":"high"}))
        );
        assert!(
            loaded
                .sessions
                .iter()
                .all(|pane| pane.session_options.is_none())
        );
        let mut incompatible = loaded.clone();
        incompatible.sessions[0].native_options = Some(serde_json::json!({"effort":"bogus"}));
        assert!(incompatible.validate().is_err());
        incompatible = loaded.clone();
        incompatible.sessions[1].native_options = Some(serde_json::json!({"effort":"none"}));
        assert!(incompatible.validate().is_err());
        let mut bad_options = loaded.clone();
        bad_options.sessions[0].native_options = Some(serde_json::json!({"model":"--unsafe"}));
        assert!(bad_options.validate().is_err());
        bad_options.sessions[0].native_options = Some(serde_json::json!({"permissions":"yolo"}));
        assert!(bad_options.validate().is_err());

        assert_eq!(loaded.sessions[0].launch.native_chat(), Some("Codex"));
        assert_eq!(loaded.terminal_layouts, state.terminal_layouts);
        let mut invalid = loaded.clone();
        invalid.sessions[1].launch = crate::launch::Launch::CodexChat;
        assert!(invalid.validate().is_ok());
        invalid = loaded.clone();
        invalid.sessions[0].project = None;
        assert!(invalid.validate().is_err());
        invalid = loaded;
        invalid.sessions[0].cwd = cwd.join("unrelated");
        assert!(invalid.validate().is_err());
    }
    #[test]
    fn terminal_split_geometry_survives_storage_without_starting_saved_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        for id in 1..=2 {
            state.sessions.push(SavedSession {
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
                project: None,
                title: format!("Sleeping {id}"),
                launch: crate::launch::Launch::Shell,
                cwd: dir.path().to_owned(),
            });
        }
        state.sync_terminal_layouts([1200.0, 700.0]);
        let layout = state.terminal_layouts.get_mut("global").unwrap();
        let geometry = crate::pane_grid::grid_layout(layout.tree.as_ref());
        assert!(layout.resize(&geometry.dividers[0].split_id, 0, 1200.0, 80.0, None));
        store.save(&state).unwrap();
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.terminal_layouts, state.terminal_layouts);
        assert_eq!(loaded.sessions.len(), 2);
        assert_eq!(loaded.sessions[0].title, "Sleeping 1");
    }
    #[test]
    fn refuses_ssh_options_and_shell_syntax() {
        for host in ["-oProxyCommand=evil", "host;command", "user@host\nnext", ""] {
            assert!(validate_host(host).is_err());
        }
        assert!(validate_host("user@example.org").is_ok());
    }
    #[test]
    fn session_metadata_roundtrip_does_not_start_processes() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.sessions.push(SavedSession {
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
            id: 1,
            project: None,
            title: "Sleeping".into(),
            launch: crate::launch::Launch::Shell,
            cwd: dir.path().to_owned(),
        });
        store.save(&state).unwrap();
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.sessions[0].title, "Sleeping");
    }
    #[test]
    fn multiple_native_panes_have_independent_resume_ids_and_legacy_migration_is_one_time() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let project = state.add_project(dir.path()).unwrap();
        let cwd = state.active().unwrap().path.clone();
        for id in [10, 11] {
            state.sessions.push(SavedSession {
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
                project: Some(project),
                title: format!("Codex {id}"),
                launch: crate::launch::Launch::CodexChat,
                cwd: cwd.clone(),
            });
        }
        state.sessions.push(SavedSession {
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
            id: 12,
            project: Some(project),
            title: "Claude".into(),
            launch: crate::launch::Launch::ClaudeChat,
            cwd: cwd.clone(),
        });
        let claude_id = "550e8400-e29b-41d4-a716-446655440000";
        state.claude_threads.insert(project, claude_id.into());
        state.chat_threads.insert(project, "legacy-thread".into());
        store.save(&state).unwrap();
        drop(store);
        let (store, mut state) = Store::open(dir.path()).unwrap();
        assert_eq!(
            state.native_resume(10, project, "Codex"),
            Some("legacy-thread")
        );
        assert_eq!(state.native_resume(11, project, "Codex"), None);
        assert!(state.chat_threads.is_empty());
        assert_eq!(state.native_resume(12, project, "Claude"), Some(claude_id));
        assert_eq!(state.native_resume(12, project, "Codex"), None);
        assert!(state.claude_threads.is_empty());

        assert!(
            state
                .remember_native_thread(11, project, "Codex", Some("second-thread"))
                .unwrap()
        );
        assert!(
            state
                .remember_native_thread(11, project + 1, "Codex", Some("wrong-project"))
                .is_err()
        );
        assert!(
            state
                .remember_native_thread(10, project, "Claude", Some("wrong-provider"))
                .is_err()
        );
        state.migrate_native_threads();
        assert_eq!(
            state.native_resume(10, project, "Codex"),
            Some("legacy-thread")
        );
        assert_eq!(
            state.native_resume(11, project, "Codex"),
            Some("second-thread")
        );
        state.sessions.clear();
        assert!(state.next_pane_id() > 11);
        store.save(&state).unwrap();
        drop(store);
        let (_, state) = Store::open(dir.path()).unwrap();
        assert!(state.next_pane_id() > 11);
        assert_eq!(
            state.native_resume(11, project, "Codex"),
            Some("second-thread")
        );
        let mut bad = state.clone();
        bad.sessions.push(SavedSession {
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
            id: 11,
            project: Some(project),
            title: "Shell".into(),
            launch: crate::launch::Launch::Shell,
            cwd,
        });
        assert!(bad.validate().is_err());
    }
    #[test]
    fn agent_names_migrate_once_and_survive_saved_workspace_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        for (id, name, launch) in [
            (
                1,
                Some("My agent"),
                crate::launch::Launch::Agent("codex".into()),
            ),
            (
                2,
                Some("my AGENT"),
                crate::launch::Launch::Agent("claude".into()),
            ),
            (3, None, crate::launch::Launch::Agent("gemini".into())),
            (4, Some("Old shell label"), crate::launch::Launch::Shell),
        ] {
            state.sessions.push(SavedSession {
                legacy_number: None,
                ssh_tmux_id: None,
                native_host: None,
                last_model: None,
                id,
                codex_seed: None,
                conversation_id: None,
                profile_binding: None,
                launch_fingerprint: None,
                session_options: None,
                native_options: None,
                worktree: None,
                pane_name: name.map(str::to_owned),
                broadcast: true,
                project: None,
                title: format!("Program {id}"),
                launch,
                cwd: dir.path().to_owned(),
            });
        }
        store.save(&state).unwrap();
        drop(store);
        let (store, mut loaded) = Store::open(dir.path()).unwrap();
        let names: Vec<_> = loaded
            .sessions
            .iter()
            .map(|s| s.pane_name.as_deref())
            .collect();
        assert_eq!(names, [Some("My agent"), Some("Ada"), Some("Bohr"), None]);
        assert_eq!(loaded.sessions[0].title, "Program 1");
        loaded.sessions[1].pane_name = Some("Renamed agent".into());
        store.save(&loaded).unwrap();
        drop(store);
        let (_, reopened) = Store::open(dir.path()).unwrap();
        assert_eq!(
            reopened.sessions[1].pane_name.as_deref(),
            Some("Renamed agent")
        );
        assert_eq!(reopened.sessions[2].pane_name.as_deref(), Some("Bohr"));
        assert_eq!(reopened.sessions[1].title, "Program 2");
    }
    #[test]
    fn broadcast_flags_default_safely_for_legacy_json_and_persist_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_session: SavedSession = serde_json::from_value(serde_json::json!({
            "id": 1,
            "project": null,
            "title": "Shell",
            "launch": "Shell",
            "cwd": dir.path(),
        }))
        .unwrap();
        assert!(legacy_session.broadcast);
        let legacy_workspace: Workspace = serde_json::from_str(r#"{"schema":1}"#).unwrap();
        assert!(!legacy_workspace.broadcast);

        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.broadcast = true;
        let mut unchecked = legacy_session;
        unchecked.broadcast = false;
        state.sessions.push(unchecked);
        store.save(&state).unwrap();
        drop(store);
        let (_, reopened) = Store::open(dir.path()).unwrap();
        assert!(reopened.broadcast);
        assert!(!reopened.sessions[0].broadcast);
    }

    #[test]
    fn switch_saved_session_chat_mode_preserves_pane_identity_and_conversation_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let proj_dir = dir.path().join("demo");
        fs::create_dir_all(&proj_dir).unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let pid = state.add_project(&proj_dir).unwrap();
        let proj_path = state.projects[0].path.clone();
        let uuid = "123e4567-e89b-42d3-a456-426614174000";

        state.sessions.push(SavedSession {
            legacy_number: None,
            ssh_tmux_id: None,
            native_host: None,
            last_model: None,
            codex_seed: None,
            conversation_id: Some(uuid.to_owned()),
            profile_binding: Some(crate::session_profile_identity::Binding::Managed {
                account_id: "original".into(),
            }),
            launch_fingerprint: None,
            session_options: Some(serde_json::json!({"model": "claude-sonnet-4-6"})),
            native_options: None,
            worktree: None,
            pane_name: Some("Architect".to_owned()),
            broadcast: true,
            id: 7,
            project: Some(pid),
            title: "Claude Code".to_owned(),
            launch: crate::launch::Launch::Agent("claude".into()),
            cwd: proj_path,
        });
        store.save(&state).unwrap();
        let reopened: Workspace =
            serde_json::from_slice(&fs::read(dir.path().join("workspace.json")).unwrap()).unwrap();
        assert_eq!(
            reopened.sessions[0].profile_binding,
            state.sessions[0].profile_binding
        );
        assert!(state.sessions[0].profile_binding.is_some());

        // 1. Switch Terminal (`Agent("claude")`) -> NativeChat (`ClaudeChat`) in-place:
        assert!(state.switch_saved_session_chat_mode(7, None).unwrap());
        assert_eq!(state.sessions[0].id, 7);
        assert_eq!(state.sessions[0].pane_name.as_deref(), Some("Architect"));
        assert!(matches!(
            state.sessions[0].launch,
            crate::launch::Launch::ClaudeChat
        ));
        assert_eq!(state.native_resume(7, pid, "Claude"), Some(uuid));
        store.save(&state).unwrap();
        let reopened: Workspace =
            serde_json::from_slice(&fs::read(dir.path().join("workspace.json")).unwrap()).unwrap();
        assert_eq!(
            reopened.sessions[0].profile_binding,
            state.sessions[0].profile_binding
        );
        assert!(state.sessions[0].profile_binding.is_some());

        // 2. Switch NativeChat (`ClaudeChat`) -> Terminal (`Agent("claude")`) in-place:
        assert!(state.switch_saved_session_chat_mode(7, None).unwrap());
        assert_eq!(state.sessions[0].id, 7);
        assert_eq!(state.sessions[0].pane_name.as_deref(), Some("Architect"));
        assert!(matches!(
            &state.sessions[0].launch,
            crate::launch::Launch::Agent(agent) if agent == "claude"
        ));
        assert_eq!(state.sessions[0].conversation_id.as_deref(), Some(uuid));
        assert!(!state.native_threads.contains_key(&7));
        store.save(&state).unwrap();
        let reopened: Workspace =
            serde_json::from_slice(&fs::read(dir.path().join("workspace.json")).unwrap()).unwrap();
        assert_eq!(
            reopened.sessions[0].profile_binding,
            state.sessions[0].profile_binding
        );
        assert!(state.sessions[0].profile_binding.is_some());
    }
}
